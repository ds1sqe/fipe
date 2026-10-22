//! What the generated code does with locals beyond copying them, copied
//! from `src/vm/assign.rs` (an rvalue into a local) and `src/vm/step.rs`
//! (a field read and a switch's key).
use super::{budget, natives::Kind, Faulted, Site, State};
use fipe::{
    mir::{Rvalue, Statement},
    value::{Record, Value},
};
use std::rc::Rc;

/// The value of the site's `Assign` of a `Construct` or a `Native`,
/// its operands read into `args` by the generated code.
///
/// # Errors
///
/// When the native Faults; the Fault is kept.
///
/// # Panics
///
/// At another statement, a defect of the lowering.
pub fn assign(
    state: &mut State,
    site: Site,
    args: Vec<Value>,
) -> Result<Value, Faulted> {
    let op = match state.statement(site) {
        Statement::Assign(
            _,
            Rvalue::Construct {
                meaning,
                constructor,
                ..
            },
        ) => {
            return Ok(Value::Record(Rc::new(Record {
                meaning: *meaning,
                constructor: *constructor,
                fields: args,
            })));
        }
        Statement::Assign(_, Rvalue::Native { op, .. }) => *op,
        other => unreachable!("the generated code assigns {other:?}"),
    };
    let Kind::Native(Some(native)) = state.natives.get(op) else {
        unreachable!("the lowering refuses this native")
    };
    let computed = state
        .pay(budget::size(native, &args))
        .and_then(|()| super::natives::immediate(native, args, &state.prelude));
    computed.map_err(|reason| state.faulted(site, reason))
}

/// A record's field, a tuple's part, a sequence's element, a packed
/// value's payload, or the bytes of text (`String { bytes }`).
///
/// # Panics
///
/// On a value with no parts, a defect of the lowering.
#[must_use]
pub fn field(value: &Value, index: usize) -> Value {
    match value {
        Value::Record(record) => record.fields[index].clone(),
        Value::Tuple(parts) | Value::Seq(parts) => parts[index].clone(),
        Value::Packed(packed) => packed.value.clone(),
        Value::Text(_) => fipe::value::text::bytes(value),
        other => unreachable!("a field of {other:?}"),
    }
}

/// The key a switch compares with its cases: a record's constructor, a
/// packed value's meaning, an integer's number.
///
/// # Panics
///
/// On another value, a defect of the lowering.
#[must_use]
pub fn switch_key(value: &Value) -> u64 {
    let key = match value {
        Value::Record(record) => record.constructor,
        Value::Packed(packed) => packed.meaning,
        Value::Int(number) => usize::try_from(*number).unwrap_or(usize::MAX),
        other => unreachable!("a switch on {other:?}"),
    };
    key as u64
}
