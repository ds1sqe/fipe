//! A predicate run over one value, rewritten as plain recursion from
//! `src/vm/predicates.rs`: `All` and `Any` short-circuit in order, a
//! limit test is its body, called with the value; the division test
//! costs one unit, paid where the VM's run owes it, before the call or
//! the verdict that follows.
use super::{calls, fail, pay, Ctx, Faulted, Site};
use fipe::{
    ir::Predicate,
    mir::Terminator,
    value::{structural, Reason, Value},
};

/// Whether `value` passes `predicate`, at `node`: a `Test`
/// terminator's branch, a checked conversion, an entry's component.
pub fn holds(
    ctx: &Ctx,
    predicate: &Predicate,
    value: &Value,
    node: usize,
) -> Result<bool, Faulted> {
    match predicate {
        Predicate::All(parts) => {
            for part in parts {
                if !holds(ctx, part, value, node)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Predicate::Any(parts) => {
            for part in parts {
                if holds(ctx, part, value, node)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Predicate::Not(inner) => Ok(!holds(ctx, inner, value, node)?),
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
            let passed =
                calls::enter(body, std::slice::from_ref(value), &[], meanings)?;
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
pub fn branch(site: Site, value: &Value) -> Result<bool, Faulted> {
    let ctx = Ctx::get();
    let node = ctx.node(site);
    let block = &ctx.program.bodies[site.body].blocks[site.block];
    let Terminator::Test { predicate, .. } = &block.terminator else {
        unreachable!("a test site")
    };
    holds(&ctx, predicate, value, node)
}

/// Test each value in order by an entry's predicate, stopping at the
/// first that fails, which Faults naming the entry and the value.
pub fn enter_each(
    ctx: &Ctx,
    predicate: &Predicate,
    entry: usize,
    values: Vec<Value>,
    node: usize,
) -> Result<(), Faulted> {
    for value in values {
        if !holds(ctx, predicate, &value, node)? {
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
) -> Result<(), Faulted> {
    enter_each(ctx, predicate, entry, vec![value], node)
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
