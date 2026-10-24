//! A predicate run over one value, rewritten as plain recursion from
//! `src/vm/predicates.rs`: `All` and `Any` short-circuit in order, a
//! limit test is its body, called with the value; the division test
//! costs one unit, paid where the VM's run owes it, before the call or
//! the verdict that follows.
use super::{calls, dispatch, fail, pay, Ctx, Faulted, Frame, Site};
use fipe::{
    ir::{self, Predicate, Type},
    mir::Terminator,
    value::{structural, Reason, Value},
};
use std::rc::Rc;

/// Whether `value` passes `predicate`, at `node`: a `Test`
/// terminator's branch, a checked conversion, an entry's component.
fn holds(
    ctx: &Ctx,
    predicate: &Predicate,
    value: &Value,
    node: usize,
    witnesses: &Witnesses,
) -> Result<bool, Faulted> {
    match predicate {
        Predicate::All(parts) => {
            for part in parts {
                if !holds(ctx, part, value, node, witnesses)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Predicate::Any(parts) => {
            for part in parts {
                if holds(ctx, part, value, node, witnesses)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Predicate::Not(inner) => {
            Ok(!holds(ctx, inner, value, node, witnesses)?)
        }
        // The division test is charged as one comparison. A divisor of
        // a generic meaning is tested only when the instance is an
        // integer; a float divisor of zero Faults `NotFinite` in the
        // division itself.
        Predicate::Nonzero { when_integer } => {
            pay(1, node)?;
            let integer = matches!(value, Value::Int(_) | Value::Big(_));
            Ok((*when_integer && !integer) || nonzero(value))
        }
        // A function value is not tested; its calls are.
        Predicate::Function { .. } => Ok(true),
        Predicate::Test { index, meanings } => {
            let Some(body) = ctx.program.tests[*index] else {
                return Err(fail(
                    node,
                    Reason::Message(format!("limit test {index} has no body")),
                ));
            };
            pay(u64::try_from(meanings.len()).unwrap_or(u64::MAX), node)?;
            let mut actual = dispatch::Types::default();
            for meaning in meanings {
                pay(u64::try_from(witnesses.len()).unwrap_or(u64::MAX), node)?;
                let bindings: Vec<_> = witnesses
                    .iter()
                    .map(|(id, ty)| (*id, ty.as_type()))
                    .collect();
                let mut copied = calls::copied_predicate_type(
                    ctx, meaning, &bindings, true, node,
                )?;
                actual.0.append(&mut copied.0);
            }
            let passed = calls::enter(
                body,
                std::slice::from_ref(value),
                &[],
                &actual.0,
            )?;
            Ok(ctx.prelude.is_true(&passed))
        }
    }
}

/// The site's `Test` terminator over `value`: whether it passes, which
/// picks the branch.
///
/// # Errors
///
/// When a limit test Faults or the budget cannot pay.
///
/// # Panics
///
/// At another terminator, a defect of the lowering.
pub fn branch(
    site: Site,
    value: &Value,
    hidden: &[Value],
) -> Result<bool, Faulted> {
    let ctx = Ctx::get();
    let node = ctx.node(site);
    let block = &ctx.program.bodies[site.body].blocks[site.block];
    let Terminator::Test { predicate, .. } = &block.terminator else {
        unreachable!("a test site")
    };
    let frame = Frame::new(site.body, hidden);
    let witnesses = capture_predicate(
        predicate,
        false,
        &ctx.program.bodies[site.body].hidden,
        &frame.meanings,
        &ctx,
        node,
    )?;
    // Match the Core owned root snapshot, then logical run preparation.
    prepare_predicate(&ctx, predicate, node)?;
    prepare_predicate(&ctx, predicate, node)?;
    holds(&ctx, predicate, value, node, &witnesses)
}

/// Test each value in order by an entry's predicate, stopping at the
/// first that fails, which Faults naming the entry and the value.
pub fn enter_each(
    ctx: &Ctx,
    predicate: &Predicate,
    entry: usize,
    values: Vec<Value>,
    node: usize,
    witnesses: &Witnesses,
) -> Result<(), Faulted> {
    for value in values {
        prepare_predicate(ctx, predicate, node)?;
        if !holds(ctx, predicate, &value, node, witnesses)? {
            let value = structural::show(&value, &ctx.tables());
            return Err(fail(node, Reason::Failed { entry, value }));
        }
    }
    Ok(())
}

/// Test one value by a predicate, a Fault naming `entry` on failure: a
/// closure's guard on an argument.
pub fn enter_by(
    ctx: &Ctx,
    predicate: &Predicate,
    entry: usize,
    value: Value,
    node: usize,
    witnesses: &Witnesses,
) -> Result<(), Faulted> {
    enter_each(ctx, predicate, entry, vec![value], node, witnesses)
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

pub(super) type Witnesses = Rc<[(usize, Rc<fipe::value::Meaning>)]>;
pub(super) fn capture_predicate(
    predicate: &ir::Predicate,
    entry: bool,
    hidden: &[usize],
    meanings: &[Rc<fipe::value::Meaning>],
    ctx: &Ctx,
    node: usize,
) -> Result<Witnesses, Faulted> {
    capture_predicate_with(
        predicate,
        entry,
        hidden,
        &|index| meanings.get(index).map(|meaning| meaning.as_type()),
        ctx,
        node,
    )
}

pub(super) fn capture_predicate_types(
    predicate: &ir::Predicate,
    entry: bool,
    hidden: &[usize],
    meanings: &[Type],
    ctx: &Ctx,
    node: usize,
) -> Result<Witnesses, Faulted> {
    capture_predicate_with(
        predicate,
        entry,
        hidden,
        &|index| meanings.get(index),
        ctx,
        node,
    )
}

fn capture_predicate_with<'a>(
    predicate: &ir::Predicate,
    entry: bool,
    hidden: &[usize],
    meaning_at: &impl Fn(usize) -> Option<&'a Type>,
    ctx: &Ctx,
    node: usize,
) -> Result<Witnesses, Faulted> {
    enum Job<'a> {
        Predicate(&'a ir::Predicate, bool),
        Type(&'a Type),
    }
    pay(1, node)?;
    let mut jobs = vec![Job::Predicate(predicate, entry)];
    let mut captured: Vec<(usize, Rc<fipe::value::Meaning>)> = Vec::new();
    while let Some(job) = jobs.pop() {
        pay(1, node)?;
        match job {
            Job::Predicate(predicate, true) => {
                if let ir::Predicate::Function { parameter, result } = predicate
                {
                    pay(2, node)?;
                    jobs.push(Job::Predicate(result, true));
                    jobs.push(Job::Predicate(
                        parameter,
                        matches!(
                            parameter.as_ref(),
                            ir::Predicate::Function { .. }
                        ),
                    ));
                }
            }
            Job::Predicate(predicate, false) => match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pay(u64::try_from(parts.len()).unwrap_or(u64::MAX), node)?;
                    jobs.extend(
                        parts
                            .iter()
                            .rev()
                            .map(|part| Job::Predicate(part, false)),
                    );
                }
                ir::Predicate::Not(inner) => {
                    pay(1, node)?;
                    jobs.push(Job::Predicate(inner, false));
                }
                ir::Predicate::Test { meanings, .. } => {
                    pay(
                        u64::try_from(meanings.len()).unwrap_or(u64::MAX),
                        node,
                    )?;
                    jobs.extend(meanings.iter().rev().map(Job::Type));
                }
                ir::Predicate::Function { .. }
                | ir::Predicate::Nonzero { .. } => {}
            },
            Job::Type(ty) => match ty {
                Type::Parameter(parameter) => {
                    capture_parameter(
                        *parameter,
                        hidden,
                        meaning_at,
                        ctx,
                        node,
                        &mut captured,
                    )?;
                }
                Type::Meaning { arguments, .. }
                | Type::Tuple(arguments)
                | Type::Packed(arguments)
                | Type::All(arguments)
                | Type::Sum(arguments) => {
                    pay(
                        u64::try_from(arguments.len()).unwrap_or(u64::MAX),
                        node,
                    )?;
                    jobs.extend(arguments.iter().rev().map(Job::Type));
                }
                Type::Array { element, length } => {
                    pay(2, node)?;
                    jobs.push(Job::Type(length));
                    jobs.push(Job::Type(element));
                }
                Type::Function { parameter, result } => {
                    pay(2, node)?;
                    jobs.push(Job::Type(result));
                    jobs.push(Job::Type(parameter));
                }
                Type::Not(inner) => {
                    pay(1, node)?;
                    jobs.push(Job::Type(inner));
                }
                Type::Unit | Type::Session(_) | Type::Number(_) => {}
            },
        }
    }
    pay(
        u64::try_from(captured.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1),
        node,
    )?;
    Ok(Rc::from(captured))
}

pub(super) fn holds_in_frame(
    ctx: &Ctx,
    frame: &Frame,
    predicate: &Predicate,
    value: &Value,
    node: usize,
) -> Result<bool, Faulted> {
    let witnesses = capture_predicate(
        predicate,
        false,
        &ctx.program.bodies[frame.body].hidden,
        &frame.meanings,
        ctx,
        node,
    )?;
    // A borrowed snapshot and a borrowed working run have the same unit events.
    prepare_predicate(ctx, predicate, node)?;
    prepare_predicate(ctx, predicate, node)?;
    holds(ctx, predicate, value, node, &witnesses)
}

fn capture_parameter<'a>(
    parameter: usize,
    hidden: &[usize],
    meaning_at: &impl Fn(usize) -> Option<&'a Type>,
    ctx: &Ctx,
    node: usize,
    captured: &mut Vec<(usize, Rc<fipe::value::Meaning>)>,
) -> Result<(), Faulted> {
    pay(u64::try_from(captured.len()).unwrap_or(u64::MAX), node)?;
    if captured.iter().any(|(id, _)| *id == parameter) {
        return Ok(());
    }
    pay(u64::try_from(hidden.len()).unwrap_or(u64::MAX), node)?;
    let actual = hidden
        .iter()
        .position(|id| *id == parameter)
        .and_then(meaning_at)
        .ok_or_else(|| {
            fail(node, Reason::Host("missing predicate witness".into()))
        })?;
    pay(1, node)?;
    let mut copied =
        calls::copied_predicate_type(ctx, actual, &[], true, node)?;
    pay(1, node)?;
    let actual = copied.0.pop().expect("one captured witness");
    captured.push((parameter, Rc::new(fipe::value::Meaning::new(actual))));
    Ok(())
}

#[derive(Default)]
pub(super) struct OwnedPredicates(pub(super) Vec<Predicate>);
impl Drop for OwnedPredicates {
    fn drop(&mut self) {
        while let Some(predicate) = self.0.pop() {
            match predicate {
                Predicate::All(parts) | Predicate::Any(parts) => {
                    self.0.extend(parts);
                }
                Predicate::Function { parameter, result } => {
                    self.0.push(*parameter);
                    self.0.push(*result);
                }
                Predicate::Not(inner) => self.0.push(*inner),
                Predicate::Test { meanings, .. } => {
                    drop(super::dispatch::Types(meanings));
                }
                Predicate::Nonzero { .. } => (),
            }
        }
    }
}
pub(super) fn copy_predicate(
    ctx: &Ctx,
    predicate: &Predicate,
    node: usize,
) -> Result<OwnedPredicates, Faulted> {
    predicate_copy(predicate, ctx, node, true)
}
pub(super) fn prepare_predicate(
    ctx: &Ctx,
    predicate: &Predicate,
    node: usize,
) -> Result<(), Faulted> {
    predicate_copy(predicate, ctx, node, false).map(|_| ())
}
pub(super) fn copy_guard(
    ctx: &Ctx,
    guard: &fipe::value::Guard,
    node: usize,
) -> Result<fipe::value::Guard, Faulted> {
    pay(2, node)?;
    let mut predicate = copy_predicate(ctx, &guard.predicate, node)?;
    Ok(fipe::value::Guard {
        spot: guard.spot,
        entry: guard.entry,
        predicate: predicate.0.pop().expect("guard predicate"),
        witnesses: guard.witnesses.clone(),
    })
}
pub(super) fn copy_guards(
    ctx: &Ctx,
    guards: &[fipe::value::Guard],
    node: usize,
) -> Result<Vec<fipe::value::Guard>, Faulted> {
    pay(u64::try_from(guards.len()).unwrap_or(u64::MAX), node)?;
    let mut copied = Vec::with_capacity(guards.len());
    for guard in guards {
        copied.push(copy_guard(ctx, guard, node)?);
    }
    Ok(copied)
}
#[derive(Clone, Copy)]
enum PredicateShape {
    All,
    Any,
    Function,
    Not,
}
enum PredicateCopy<'a> {
    Read(&'a ir::Predicate),
    Finish(PredicateShape, usize),
}

/// Exact predicate syntax copy. This never substitutes a creator parameter or Session.
fn predicate_copy(
    predicate: &ir::Predicate,
    ctx: &Ctx,
    node: usize,
    output: bool,
) -> Result<OwnedPredicates, Faulted> {
    let charge = |units| pay(units, node);
    charge(1)?;
    let mut jobs = vec![PredicateCopy::Read(predicate)];
    let mut first = true;
    let mut done = OwnedPredicates::default();
    while let Some(job) = jobs.pop() {
        if !first {
            charge(1)?;
        }
        first = false;
        match job {
            PredicateCopy::Read(predicate) => match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    charge(u64::try_from(parts.len()).unwrap_or(u64::MAX))?;
                    let shape = if matches!(predicate, ir::Predicate::All(_)) {
                        PredicateShape::All
                    } else {
                        PredicateShape::Any
                    };
                    jobs.push(PredicateCopy::Finish(shape, parts.len()));
                    jobs.extend(parts.iter().rev().map(PredicateCopy::Read));
                }
                ir::Predicate::Function { parameter, result } => {
                    charge(2)?;
                    jobs.push(PredicateCopy::Finish(
                        PredicateShape::Function,
                        2,
                    ));
                    jobs.push(PredicateCopy::Read(result));
                    jobs.push(PredicateCopy::Read(parameter));
                }
                ir::Predicate::Not(inner) => {
                    charge(1)?;
                    jobs.push(PredicateCopy::Finish(PredicateShape::Not, 1));
                    jobs.push(PredicateCopy::Read(inner));
                }
                ir::Predicate::Nonzero { when_integer } => {
                    charge(1)?;
                    if output {
                        done.0.push(ir::Predicate::Nonzero {
                            when_integer: *when_integer,
                        });
                    }
                }
                ir::Predicate::Test { index, meanings } => {
                    charge(u64::try_from(meanings.len()).unwrap_or(u64::MAX))?;
                    let mut copied = super::dispatch::Types::default();
                    for ty in meanings {
                        if output {
                            let mut one = super::calls::copied_predicate_type(
                                ctx,
                                ty,
                                &[],
                                false,
                                node,
                            )?;
                            copied
                                .0
                                .push(one.0.pop().expect("raw predicate type"));
                        } else {
                            super::calls::prepare_predicate_type(
                                ctx, ty, node,
                            )?;
                        }
                    }
                    charge(1)?;
                    if output {
                        done.0.push(ir::Predicate::Test {
                            index: *index,
                            meanings: core::mem::take(&mut copied.0),
                        });
                    }
                }
            },
            PredicateCopy::Finish(shape, count) => {
                if !output {
                    continue;
                }
                finish_predicate_copy(shape, count, &mut done);
            }
        }
    }
    Ok(done)
}

fn finish_predicate_copy(
    shape: PredicateShape,
    count: usize,
    done: &mut OwnedPredicates,
) {
    let mut parts = done.0.split_off(done.0.len() - count);
    let copied = match shape {
        PredicateShape::All => ir::Predicate::All(parts),
        PredicateShape::Any => ir::Predicate::Any(parts),
        PredicateShape::Function => {
            let result = parts.pop().expect("predicate result");
            let parameter = parts.pop().expect("predicate parameter");
            ir::Predicate::Function {
                parameter: std::boxed::Box::new(parameter),
                result: std::boxed::Box::new(result),
            }
        }
        PredicateShape::Not => ir::Predicate::Not(std::boxed::Box::new(
            parts.pop().expect("predicate part"),
        )),
    };
    done.0.push(copied);
}
