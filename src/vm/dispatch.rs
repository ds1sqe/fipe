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

/// The arguments of a row written for a packed receiver's tag: the row
/// reads the tag's own value, so the payload replaces the receiver. For
/// `eq` the other side is packed too and compares tags first, then
/// values (`04-members.md` §Packed types): `Err(false)` when the tags
/// differ. A row of the packed meaning itself keeps the packed value.
pub(super) fn unpack(
    args: &mut [Value],
    spot: usize,
    row: &ir::Type,
    eq: bool,
) -> Result<(), bool> {
    let Some(Value::Packed(packed)) = args.get(spot) else {
        return Ok(());
    };
    let tag = packed.meaning;
    if !matches!(row, ir::Type::Meaning { index, .. } if *index == tag) {
        return Ok(());
    }
    let payload = packed.value.clone();
    args[spot] = payload;
    if eq {
        for other in args.iter_mut() {
            if let Value::Packed(packed) = other {
                if packed.meaning != tag {
                    return Err(false);
                }
                let payload = packed.value.clone();
                *other = payload;
            }
        }
    }
    Ok(())
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
