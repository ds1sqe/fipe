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
    /// Nothing on a pass; a Fault naming the entry on a failure.
    Enter(usize),
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
    /// The values still to test after this one, the next last.
    rest: Vec<Value>,
    root: RunRoot,
    witnesses: Option<super::resource::Witnesses>,
}

#[derive(Debug)]
enum RunRoot {
    Owned(crate::value::resource::Predicates),
    Guard(super::pending::GuardJob),
}
impl RunRoot {
    fn predicate(&self) -> &Predicate {
        match self {
            Self::Owned(forest) => &forest.0[0],
            Self::Guard(job) => &job.guard().predicate,
        }
    }
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
    pub(super) fn owned(
        predicate: crate::value::resource::Predicates,
        values: Vec<Value>,
        after: After,
        witnesses: super::resource::Witnesses,
        tables: &ir::Tables,
        work: &super::resource::Work,
    ) -> Result<Self, Reason> {
        Self::prepare(
            RunRoot::Owned(predicate),
            values,
            after,
            Some(witnesses),
            tables,
            work,
        )
    }

    pub(super) fn guarded(
        job: super::pending::GuardJob,
        value: Value,
        tables: &ir::Tables,
        work: &super::resource::Work,
    ) -> Result<Self, Reason> {
        let entry = job.guard().entry;
        Self::prepare(
            RunRoot::Guard(job),
            alloc::vec![value],
            After::Enter(entry),
            None,
            tables,
            work,
        )
    }

    fn prepare(
        root: RunRoot,
        mut values: Vec<Value>,
        after: After,
        witnesses: Option<super::resource::Witnesses>,
        tables: &ir::Tables,
        work: &super::resource::Work,
    ) -> Result<Self, Reason> {
        let mut copied =
            super::resource::copy_predicate(root.predicate(), tables, work)?;
        values.reverse();
        let value = values.pop().expect("a value to test");
        Ok(Self {
            stack: alloc::vec![Node {
                kind: Kind::All,
                parts: core::mem::take(&mut copied.0),
                next: 0
            }],
            value,
            after,
            verdict: None,
            rest: values,
            root,
            witnesses,
        })
    }

    fn witnesses(&self) -> &super::resource::Witnesses {
        match &self.root {
            RunRoot::Guard(job) => &job.guard().witnesses,
            RunRoot::Owned(_) => {
                self.witnesses.as_ref().expect("captured witnesses")
            }
        }
    }

    /// The verdict on one value: the next value's run begins when it
    /// passed and more remain, else the run ends.
    fn conclude(&mut self, verdict: bool, ctx: &Ctx<'_>) -> Option<Step> {
        if verdict && !self.rest.is_empty() {
            let mut copied = match super::resource::copy_predicate(
                self.root.predicate(),
                ctx.tables,
                ctx.work,
            ) {
                Ok(copied) => copied,
                Err(reason) => return Some(Step::Fault(reason)),
            };
            self.value = self.rest.pop().expect("next component");
            self.stack = alloc::vec![Node {
                kind: Kind::All,
                parts: core::mem::take(&mut copied.0),
                next: 0
            }];
            return None;
        }
        Some(self.finish(verdict, ctx))
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
                    Some(verdict) => match self.conclude(verdict, ctx) {
                        Some(step) => return step,
                        None => continue,
                    },
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
                    match self.conclude(exhausted, ctx) {
                        Some(step) => return step,
                        None => continue,
                    }
                }
                result = Some(exhausted);
                continue;
            }
            let part = core::mem::replace(
                &mut node.parts[node.next],
                Predicate::All(Vec::new()),
            );
            let mut owned =
                crate::value::resource::Predicates(alloc::vec![part]);
            node.next += 1;
            match &mut owned.0[0] {
                Predicate::All(parts) => self.stack.push(Node {
                    kind: Kind::All,
                    parts: core::mem::take(parts),
                    next: 0,
                }),
                Predicate::Any(parts) => self.stack.push(Node {
                    kind: Kind::Any,
                    parts: core::mem::take(parts),
                    next: 0,
                }),
                Predicate::Not(inner) => self.stack.push(Node {
                    kind: Kind::Not,
                    parts: alloc::vec![*core::mem::replace(
                        inner,
                        alloc::boxed::Box::new(Predicate::All(Vec::new()))
                    )],
                    next: 0,
                }),
                // The division test is charged as one comparison. A
                // divisor of a generic meaning is tested only when the
                // instance is an integer; a float divisor of zero Faults
                // `NotFinite` in the division itself.
                Predicate::Nonzero { when_integer } => {
                    if let Err(reason) = ctx.work.pay(1) {
                        return Step::Fault(reason);
                    }
                    let integer =
                        matches!(self.value, Value::Int(_) | Value::Big(_));
                    result = Some(
                        (*when_integer && !integer) || nonzero(&self.value),
                    );
                }
                // A function value is not tested; its calls are.
                Predicate::Function { .. } => result = Some(true),
                Predicate::Test { index, meanings } => {
                    let index = *index;
                    let Some(body) = ctx.program.tests[index] else {
                        return Step::Fault(Reason::Message(alloc::format!(
                            "limit test {index} has no body"
                        )));
                    };
                    let raw_meanings = crate::value::resource::Forest(
                        core::mem::take(meanings),
                    );
                    let mut meanings = match super::resource::predicate_meanings(
                        &raw_meanings.0,
                        self.witnesses(),
                        ctx.tables,
                        ctx.work,
                    ) {
                        Ok(meanings) => meanings,
                        Err(reason) => return Step::Fault(reason),
                    };
                    return Step::Call(Invoke::Body {
                        body,
                        args: alloc::vec![self.value.clone()],
                        meanings: core::mem::take(&mut meanings.0),
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
            After::Enter(_) if verdict => Step::Done(Value::Unit),
            After::Enter(entry) => Step::Fault(Reason::Failed {
                entry,
                value: crate::value::structural::show(&value, ctx.tables),
            }),
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

impl Drop for Node {
    fn drop(&mut self) {
        drop(crate::value::resource::Predicates(core::mem::take(
            &mut self.parts,
        )));
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.stack.clear();
    }
}
