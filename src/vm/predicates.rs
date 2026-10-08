//! A predicate run: a gate's or a conversion's tests over one value,
//! each limit test a body the machine calls.
use super::{
    calls::Invoke,
    natives::{Ctx, Step},
};
use crate::{
    ir::{self, Predicate},
    value::{Reason, Value},
};
use alloc::vec::Vec;

/// What the run's verdict becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum After {
    /// The Bool itself, for a `Test` terminator's branch.
    Branch,
    /// The value on a pass; a `Cast` Fault or an `Err LimitError` on a
    /// failure, by the mode.
    Convert(ir::ConversionMode),
}

/// A composite predicate in progress: the stack of combinators above
/// the leaf being decided.
#[derive(Debug)]
pub(super) struct Run {
    stack: Vec<Node>,
    value: Value,
    after: After,
    /// Set once the verdict is known and nothing waits on a call.
    verdict: Option<bool>,
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    parts: Vec<Predicate>,
    next: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    All,
    Any,
    Not,
}

impl Run {
    pub(super) fn new(
        predicate: Predicate,
        value: Value,
        after: After,
    ) -> Self {
        Self {
            stack: alloc::vec![Node {
                kind: Kind::All,
                parts: alloc::vec![predicate],
                next: 0,
            }],
            value,
            after,
            verdict: None,
        }
    }

    /// One move: fold a returned Bool into the combinators, then descend
    /// to the next leaf, calling a test body when one is met.
    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        let mut result = returned.map(|value| ctx.prelude.is_true(&value));
        loop {
            if let Some(decided) = result.take() {
                match self.settle(decided) {
                    Some(verdict) => return self.finish(verdict, ctx),
                    None => continue,
                }
            }
            let Some(node) = self.stack.last_mut() else {
                unreachable!("the run settles before the stack empties")
            };
            if node.next >= node.parts.len() {
                // Exhausted: All passed, Any failed.
                let exhausted = match node.kind {
                    Kind::All => true,
                    Kind::Any => false,
                    Kind::Not => unreachable!("a not has its part"),
                };
                self.stack.pop();
                if self.stack.is_empty() {
                    return self.finish(exhausted, ctx);
                }
                result = Some(exhausted);
                continue;
            }
            let part = node.parts[node.next].clone();
            node.next += 1;
            match part {
                Predicate::All(parts) => self.stack.push(Node {
                    kind: Kind::All,
                    parts,
                    next: 0,
                }),
                Predicate::Any(parts) => self.stack.push(Node {
                    kind: Kind::Any,
                    parts,
                    next: 0,
                }),
                Predicate::Not(inner) => self.stack.push(Node {
                    kind: Kind::Not,
                    parts: alloc::vec![*inner],
                    next: 0,
                }),
                Predicate::Nonzero { .. } => {
                    result = Some(nonzero(&self.value));
                }
                // A function value is not tested; its calls are.
                Predicate::Function { .. } => result = Some(true),
                Predicate::Test { index, meanings } => {
                    let Some(body) = ctx.program.tests[index] else {
                        return Step::Fault(Reason::Message(alloc::format!(
                            "limit test {index} has no body"
                        )));
                    };
                    return Step::Call(Invoke::Body {
                        body,
                        args: alloc::vec![self.value.clone()],
                        meanings,
                    });
                }
            }
        }
    }

    /// Fold a leaf's verdict into the combinators above it; the run's
    /// verdict when they are all decided.
    fn settle(&mut self, mut decided: bool) -> Option<bool> {
        loop {
            let node = self.stack.last_mut()?;
            let short = match node.kind {
                Kind::All if !decided => Some(false),
                Kind::Any if decided => Some(true),
                Kind::Not => Some(!decided),
                Kind::All | Kind::Any => None,
            };
            let verdict = short?;
            self.stack.pop();
            if self.stack.is_empty() {
                return Some(verdict);
            }
            decided = verdict;
        }
    }

    fn finish(&mut self, verdict: bool, ctx: &Ctx<'_>) -> Step {
        self.verdict = Some(verdict);
        let value = core::mem::replace(&mut self.value, Value::Unit);
        match self.after {
            After::Branch => Step::Done(ctx.prelude.bool(verdict)),
            After::Convert(ir::ConversionMode::TryCheck) => {
                Step::Done(if verdict {
                    ctx.prelude.ok(value)
                } else {
                    ctx.prelude.limit_error()
                })
            }
            After::Convert(_) => {
                if verdict {
                    Step::Done(value)
                } else {
                    Step::Fault(Reason::Cast)
                }
            }
        }
    }
}

fn nonzero(value: &Value) -> bool {
    match value {
        Value::Int(number) => *number != 0,
        Value::Big(_) => true,
        Value::F32(float) => *float != 0.0,
        Value::F64(float) => *float != 0.0,
        other => unreachable!("a number, not {other:?}"),
    }
}
