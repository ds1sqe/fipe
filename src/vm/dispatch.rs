//! Full-type dispatch and checked opening of immutable Packed layers.
use super::resource::{self, Selection, Work};
use crate::{
    ir,
    value::resource::Forest,
    value::{Prelude, Reason, Value},
};
use alloc::vec::Vec;

pub(super) fn tag(value: &Value, prelude: &Prelude) -> Option<usize> {
    match value {
        Value::Record(x) => Some(x.meaning),
        Value::Packed(x) => match x.source_type() {
            ir::Type::Meaning { index, .. } => Some(*index),
            _ => None,
        },
        Value::Int(_) | Value::Big(_) => prelude.integer,
        Value::F32(_) => prelude.f32,
        Value::F64(_) => prelude.f64,
        Value::Text(_) => prelude.string,
        Value::Seq(_) => prelude.vec,
        _ => None,
    }
}
fn full_select(
    tables: &ir::Tables,
    member: usize,
    ty: &ir::Type,
    work: &Work,
) -> Result<Option<Selection>, Reason> {
    resource::select(tables, member, ty, None, work)
}
fn packed_select(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    explicit: Option<&ir::Type>,
    value: &Value,
    work: &Work,
) -> Result<Option<Selection>, Reason> {
    preflight(value, tables, work)?;
    let mut value = value;
    let mut explicit = explicit;
    while let Value::Packed(packed) = value {
        let target = resource::packed_type(packed, true, tables, work)?;
        let source = resource::packed_type(packed, false, tables, work)?;
        for ty in explicit.into_iter().chain(core::iter::once(&target.0[0])) {
            if let Some(selected) = full_select(tables, member, ty, work)? {
                let row = &tables.dispatch[member].rows[selected.row];
                let receiver = resource::substitute_bindings(
                    &row.receiver,
                    &selected.bindings,
                    tables,
                    work,
                )?;
                let receiver = Forest(alloc::vec![receiver]);
                if super::fits::retained_receiver(
                    value,
                    &receiver.0[0],
                    matches!(row.target, ir::Implementation::Field(_)),
                    tables,
                    prelude,
                    work,
                )? {
                    return Ok(Some(selected));
                }
            }
        }
        if super::fits::source_receiver(
            packed.value(),
            &source.0[0],
            tables,
            prelude,
            work,
        )? {
            if let Some(selected) =
                full_select(tables, member, &source.0[0], work)?
            {
                return Ok(Some(selected));
            }
        }
        let Value::Packed(inner) = packed.value() else {
            return Ok(None);
        };
        if !resource::packed_link(packed, inner, work)? {
            return Err(Reason::Host("packed layers do not agree".into()));
        }
        if !super::fits::source_receiver(
            packed.value(),
            &source.0[0],
            tables,
            prelude,
            work,
        )? {
            return Err(Reason::Host(
                "packed source has no valid receiver".into(),
            ));
        }
        value = packed.value();
        explicit = None;
    }
    Ok(None)
}
pub(super) fn select_for(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    ty: Option<&ir::Type>,
    value: Option<&Value>,
    work: &Work,
) -> Result<Option<Selection>, Reason> {
    let ordinary = Work::new(None);
    let work = if tables.has_resources {
        work
    } else {
        &ordinary
    };
    if let Some(value @ Value::Packed(_)) = value {
        return packed_select(tables, prelude, member, ty, value, work);
    }
    if let Some(Value::Resource(reference)) = value {
        let current =
            resource::current(reference.resource_type(), tables, work)?;
        return full_select(tables, member, &current.0[0], work);
    }
    if let Some(ty) = ty {
        if resource::concrete(ty, work)? {
            if let Some(selected) = full_select(tables, member, ty, work)? {
                return Ok(Some(selected));
            }
        } else if matches!(ty,ir::Type::Meaning{index,..} if tables.meanings.get(*index).is_some_and(|m|m.resource))
        {
            return Err(Reason::Host("nonconcrete resource receiver".into()));
        }
    }
    let head = value.and_then(|v| tag(v, prelude)).or(match ty {
        Some(ir::Type::Meaning { index, .. }) => Some(*index),
        _ => None,
    });
    Ok(head
        .and_then(|head| {
            crate::compile::dispatch::select_by_head(tables, member, head)
        })
        .map(Selection))
}
pub(super) fn by_tag(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    value: &Value,
    work: &Work,
) -> Result<Option<Selection>, Reason> {
    select_for(tables, prelude, member, None, Some(value), work)
}
fn open_for(
    value: &Value,
    receiver: &ir::Type,
    field: bool,
    tables: &ir::Tables,
    prelude: &Prelude,
    work: &Work,
) -> Result<Option<(Value, bool)>, Reason> {
    let mut value = value;
    while let Value::Packed(packed) = value {
        let target = resource::packed_type(packed, true, tables, work)?;
        let source = resource::packed_type(packed, false, tables, work)?;
        if resource::equal_current(receiver, &target.0[0], work)?
            && super::fits::retained_receiver(
                value, receiver, field, tables, prelude, work,
            )?
        {
            return Ok(Some((value.clone(), true)));
        }
        if !super::fits::source_receiver(
            packed.value(),
            &source.0[0],
            tables,
            prelude,
            work,
        )? {
            return Ok(None);
        }
        if let Value::Packed(inner) = packed.value() {
            if !resource::packed_link(packed, inner, work)? {
                return Ok(None);
            }
            value = packed.value();
        } else {
            return if resource::equal_current(receiver, &source.0[0], work)? {
                Ok(Some((packed.value().clone(), false)))
            } else {
                Ok(None)
            };
        }
    }
    Ok(Some((value.clone(), false)))
}
/// Preflight both metadata roots of each actual layer before any priority return.
fn preflight(
    value: &Value,
    tables: &ir::Tables,
    work: &Work,
) -> Result<(), Reason> {
    resource::validate_current(value, tables, work)?;
    let mut value = value;
    while let Value::Packed(packed) = value {
        drop(resource::packed_type(packed, false, tables, work)?);
        drop(resource::packed_type(packed, true, tables, work)?);
        value = packed.value();
    }
    Ok(())
}
/// Validate operands before changing any; bindings belong to the actual selected row.
pub(super) fn unpack(
    args: &mut [Value],
    spot: usize,
    row: &ir::DispatchRow,
    bindings: &[(usize, ir::Type)],
    comparison: (bool, bool),
    tables: &ir::Tables,
    work: &Work,
) -> Result<bool, Reason> {
    let (eq, cmp) = comparison;
    let ordinary = Work::new(None);
    let work = if tables.has_resources {
        work
    } else {
        &ordinary
    };
    let receiver = Forest(alloc::vec![resource::substitute_bindings(
        &row.receiver,
        bindings,
        tables,
        work
    )?]);
    let prelude = Prelude::new(tables);
    for value in args.iter() {
        preflight(value, tables, work)?;
    }
    let Some(value) = args.get(spot) else {
        return Ok(true);
    };
    let Some((opened, retained)) = open_for(
        value,
        &receiver.0[0],
        matches!(row.target, ir::Implementation::Field(_)),
        tables,
        &prelude,
        work,
    )?
    else {
        return Err(Reason::Host(
            "packed receiver does not fit its selected row".into(),
        ));
    };
    let mut replacements = alloc::vec![(spot, opened)];
    if (eq || cmp)
        && !comparison_operands(
            args,
            spot,
            &receiver.0[0],
            (eq, cmp, retained),
            (tables, &prelude, work),
            &mut replacements,
        )?
    {
        return Ok(false);
    }
    for (index, value) in replacements {
        args[index] = value;
    }
    Ok(true)
}
fn comparison_operands(
    args: &[Value],
    spot: usize,
    receiver: &ir::Type,
    comparison: (bool, bool, bool),
    context: (&ir::Tables, &Prelude, &Work),
    replacements: &mut Vec<(usize, Value)>,
) -> Result<bool, Reason> {
    let (eq, _cmp, retained) = comparison;
    let (tables, prelude, work) = context;
    let value = &args[spot];
    // An unopened opaque wrapper does not expose the source layout. Its
    // paired operand must still fit the same retained boundary and indices.
    if retained {
        for (index, other) in args.iter().enumerate() {
            if index == spot {
                continue;
            }
            if matches!(other, Value::Packed(_)) {
                let Some((opened, true)) =
                    open_for(other, receiver, false, tables, prelude, work)?
                else {
                    return Err(Reason::Host(
                        "packed comparison operand does not fit its wrapper"
                            .into(),
                    ));
                };
                replacements.push((index, opened));
            } else if !super::fits::retained_receiver(
                other, receiver, false, tables, prelude, work,
            )? {
                return Err(Reason::Host(
                    "comparison operand does not fit its wrapper".into(),
                ));
            }
        }
    } else {
        comparison_sources(args, spot, tables, prelude, work)?;
        for (index, other) in args.iter().enumerate() {
            if index == spot {
                continue;
            }
            if resource_pair(value, other, tables, work)? == Some(false) {
                return if eq {
                    Ok(false)
                } else {
                    Err(Reason::Message(
                        "different resource types have no intrinsic ordering"
                            .into(),
                    ))
                };
            }
            if let Value::Packed(other_packed) = other {
                if eq {
                    if let Value::Packed(this) = value {
                        if !resource::equal_current(
                            this.source_type(),
                            other_packed.source_type(),
                            work,
                        )? {
                            return Ok(false);
                        }
                    }
                }
                let Some((opened, _)) =
                    open_for(other, receiver, false, tables, prelude, work)?
                else {
                    return if eq {
                        Ok(false)
                    } else {
                        Err(Reason::Message("different structural sources have no intrinsic ordering".into()))
                    };
                };
                replacements.push((index, opened));
            } else if !super::fits::source_receiver(
                other, receiver, tables, prelude, work,
            )? {
                return if eq {
                    Ok(false)
                } else {
                    Err(Reason::Message(
                        "different sources have no intrinsic ordering".into(),
                    ))
                };
            }
        }
    }
    Ok(true)
}
fn comparison_sources(
    args: &[Value],
    spot: usize,
    tables: &ir::Tables,
    prelude: &Prelude,
    work: &Work,
) -> Result<(), Reason> {
    // Check each other source before an intrinsic mismatch can return false.
    for (index, other) in args.iter().enumerate() {
        if index == spot {
            continue;
        }
        if let Value::Packed(other) = other {
            let source = resource::packed_type(other, false, tables, work)?;
            if !super::fits::source_receiver(
                other.value(),
                &source.0[0],
                tables,
                prelude,
                work,
            )? {
                return Err(Reason::Host(
                    "packed comparison source has no valid receiver".into(),
                ));
            }
        }
    }
    Ok(())
}
pub(super) fn resource_pair(
    a: &Value,
    b: &Value,
    tables: &ir::Tables,
    work: &Work,
) -> Result<Option<bool>, Reason> {
    let ordinary = Work::new(None);
    let work = if tables.has_resources {
        work
    } else {
        &ordinary
    };
    if resource::data_only(a, work)? && resource::data_only(b, work)? {
        return Ok(None);
    }
    resource::validate_current(a, tables, work)?;
    resource::validate_current(b, tables, work)?;
    let current = |value: &Value| -> Result<Option<Forest>, Reason> {
        match value {
            Value::Resource(x) => {
                resource::current(x.resource_type(), tables, work).map(Some)
            }
            Value::Packed(x) => {
                resource::packed_type(x, false, tables, work).map(Some)
            }
            _ => Ok(None),
        }
    };
    match (current(a)?, current(b)?) {
        (Some(a), Some(b)) => {
            resource::equal_current(&a.0[0], &b.0[0], work).map(Some)
        }
        (None, None) => Ok(None),
        _ => Ok(Some(false)),
    }
}
