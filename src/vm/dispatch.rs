//! Run-time dispatch: a row by a witness type, or by a value's tag.
use crate::{
    compile::dispatch::{select, select_by_head, Selected},
    ir,
    value::{Prelude, Value},
};

/// The meaning a value's tag names: a record's, or the prelude's for a
/// number, text or sequence.
pub(super) fn tag(value: &Value, prelude: &Prelude) -> Option<usize> {
    match value {
        Value::Record(record) => Some(record.meaning),
        Value::Packed(packed) => Some(packed.meaning),
        Value::Int(_) | Value::Big(_) => prelude.integer,
        Value::F32(_) => prelude.f32,
        Value::F64(_) => prelude.f64,
        Value::Text(_) => prelude.string,
        Value::Seq(_) => prelude.vec,
        Value::Unit
        | Value::Tuple(_)
        | Value::Function(_)
        | Value::Meaning(_) => None,
    }
}

/// The row for a receiver: by its full type when one is known and
/// concrete, else by the value's tag.
pub(super) fn select_for(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    ty: Option<&ir::Type>,
    value: Option<&Value>,
) -> Option<Selected> {
    if let Some(ty) = ty {
        if crate::compile::dispatch::concrete(ty) {
            if let Some(selected) = select(tables, member, ty, None) {
                return Some(selected);
            }
        }
    }
    let head = match ty {
        Some(ir::Type::Meaning { index, .. }) => Some(*index),
        _ => None,
    };
    let meaning = value.and_then(|value| tag(value, prelude)).or(head)?;
    select_by_head(tables, member, meaning)
}

/// The row a value's tag selects for a member, by head meaning alone.
pub(super) fn by_tag(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    value: &Value,
) -> Option<Selected> {
    let meaning = tag(value, prelude)?;
    select_by_head(tables, member, meaning)
}
