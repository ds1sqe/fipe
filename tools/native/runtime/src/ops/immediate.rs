//! What each native does, copied from `src/vm/immediate.rs`: the value
//! at once, or for a native that calls back into the program (`map`, a
//! walk meeting a written member), the rewrite in `higher` or `walk`
//! that calls machine code for each value it needs.
use super::{
    calls, fail, higher, natives::Native, natives::Of, walk, Ctx, Faulted,
};
use fipe::{
    ir,
    value::{arith, float, structural, text, Prelude, Reason, Value},
};
use std::rc::Rc;

pub fn seq(value: &Value) -> &Rc<Vec<Value>> {
    match value {
        Value::Seq(parts) => parts,
        other => unreachable!("a sequence, not {other:?}"),
    }
}

pub const fn is_float(value: &Value) -> bool {
    matches!(value, Value::F32(_) | Value::F64(_))
}

/// Whether a value is a leaf no written member can be met under
/// (`src/vm/step.rs`).
pub const fn leaf(value: &Value) -> bool {
    !matches!(
        value,
        Value::Record(_) | Value::Seq(_) | Value::Tuple(_) | Value::Packed(_)
    )
}

/// A native with its arguments at `node`, `result` its call's result
/// type: the value.
#[allow(clippy::too_many_lines)]
pub fn immediate(
    ctx: &Ctx,
    native: Native,
    mut args: Vec<Value>,
    result: Option<&ir::Type>,
    node: usize,
) -> Result<Value, Faulted> {
    let prelude = &*ctx.prelude;
    let mut arg =
        |index: usize| std::mem::replace(&mut args[index], Value::Unit);
    let at = |reason: Reason| fail(node, reason);
    match native {
        Native::Add | Native::Sub | Native::Mul | Native::Div | Native::Rem => {
            // `other: Self -> (self: Self)`: the receiver is spot 1.
            let (other, this) = (arg(0), arg(1));
            if is_float(&this) {
                match native {
                    Native::Add => float::add(&this, &other),
                    Native::Sub => float::sub(&this, &other),
                    Native::Mul => float::mul(&this, &other),
                    Native::Div => float::div(&this, &other),
                    _ => float::rem(&this, &other),
                }
                .map_err(at)
            } else {
                match native {
                    Native::Add => Ok(arith::add(&this, &other)),
                    Native::Sub => Ok(arith::sub(&this, &other)),
                    Native::Mul => Ok(arith::mul(&this, &other)),
                    Native::Div => arith::div(&this, &other),
                    _ => arith::rem(&this, &other),
                }
                .map_err(at)
            }
        }
        Native::Eq | Native::Cmp => {
            let (other, this) = (arg(0), arg(1));
            if leaf(&this) && leaf(&other) {
                return Ok(if native == Native::Eq {
                    prelude.bool(structural::eq(&this, &other))
                } else {
                    prelude.ordering(structural::cmp(&this, &other))
                });
            }
            if native == Native::Eq {
                walk::eq(ctx, this, other, node)
            } else {
                walk::cmp(ctx, this, other, node)
            }
        }
        Native::Show => {
            let value = arg(0);
            if leaf(&value) {
                return Ok(Value::text(&structural::show(
                    &value,
                    &ctx.tables(),
                )));
            }
            walk::show(ctx, value, node)
        }
        Native::Len => {
            let value = arg(0);
            match &value {
                Value::Text(_) => Ok(text::len(&value)),
                Value::Seq(parts) => {
                    Ok(Value::Int(i64::try_from(parts.len()).expect("fits")))
                }
                // Any other value's meaning claims `Len` by a program's
                // member.
                _ => {
                    let member = ctx.natives.members.len;
                    let call = walk::member_call(ctx, member, &value, vec![]);
                    calls::invoke(ctx, call.map_err(at)?, node)
                }
            }
        }
        Native::Not => Ok(prelude.bool(!prelude.is_true(&arg(0)))),
        Native::Concat => {
            let (other, this) = (arg(0), arg(1));
            Ok(match (&this, &other) {
                (Value::Text(_), Value::Text(_)) => text::concat(&this, &other),
                (Value::Seq(a), Value::Seq(b)) => {
                    let mut joined = Vec::with_capacity(a.len() + b.len());
                    joined.extend(a.iter().cloned());
                    joined.extend(b.iter().cloned());
                    Value::seq(joined)
                }
                _ => unreachable!("a concat of one meaning"),
            })
        }
        Native::Literal(of) => {
            let form = arg(0);
            match of {
                Of::Integer => arith::literal(&form).map_err(at),
                Of::F32 => float::literal(&form, false).map_err(at),
                Of::F64 => float::literal(&form, true).map_err(at),
                Of::Text | Of::Bytes | Of::Seq => match &form {
                    Value::Record(record) => Ok(record.fields[0].clone()),
                    other => unreachable!("a form, not {other:?}"),
                },
                other => unreachable!("a literal of {other:?}"),
            }
        }
        Native::Read(of) => read(ctx, of, arg(0), result, node),
        Native::Zero(of) => Ok(match of {
            Of::F32 => Value::F32(0.0),
            Of::F64 => Value::F64(0.0),
            _ => arith::zero(),
        }),
        Native::Default(_) => Ok(Value::seq(Vec::new())),
        Native::Round => Ok(float::round(&arg(0))),
        Native::Trunc => Ok(float::trunc(&arg(0))),
        Native::Floor => Ok(float::floor(&arg(0))),
        Native::Ceil => Ok(float::ceil(&arg(0))),
        Native::Sqrt => float::sqrt(&arg(0)).map_err(at),
        Native::Sin => float::sin(&arg(0)).map_err(at),
        Native::Cos => float::cos(&arg(0)).map_err(at),
        Native::Atan2 => float::atan2(&arg(0), &arg(1)).map_err(at),
        Native::Hypot => float::hypot(&arg(0), &arg(1)).map_err(at),
        Native::Map => {
            let (function, over) = (arg(0), arg(1));
            higher::map(ctx, &function, over, node)
        }
        Native::Pure(of) => Ok(match of {
            Of::Result => prelude.ok(arg(0)),
            _ => prelude.just(arg(0)),
        }),
        Native::Then => {
            let (function, over) = (arg(0), arg(1));
            match &over {
                Value::Record(record) if record.constructor == 0 => {
                    let payload = record.fields[0].clone();
                    calls::call_value(ctx, &function, vec![payload], node)
                }
                _ => Ok(over),
            }
        }
        Native::Empty => Ok(prelude.nothing()),
        Native::Or => {
            let (other, this) = (arg(0), arg(1));
            Ok(match &this {
                Value::Record(record) if record.constructor == 0 => this,
                _ => other,
            })
        }
        Native::Fold => {
            let (init, function, over) = (arg(0), arg(1), arg(2));
            higher::fold(ctx, init, &function, &over, node)
        }
        Native::Filter => {
            let (function, over) = (arg(0), arg(1));
            higher::filter(ctx, &function, &over, node)
        }
        Native::Sum => higher::sum(ctx, &arg(0), result, node),
        Native::First => {
            let over = arg(0);
            Ok(seq(&over).first().map_or_else(
                || prelude.nothing(),
                |first| prelude.just(first.clone()),
            ))
        }
        Native::Head => {
            let over = arg(0);
            seq(&over).first().cloned().ok_or_else(|| {
                at(Reason::Message(String::from("head of an empty sequence")))
            })
        }
        Native::Take => {
            let (count, over) = (arg(0), arg(1));
            let count = match count {
                Value::Int(count) => usize::try_from(count).unwrap_or(0),
                _ => usize::MAX,
            };
            Ok(Value::seq(seq(&over).iter().take(count).cloned().collect()))
        }
        Native::Fault => {
            let Value::Text(message) = arg(0) else {
                unreachable!("text")
            };
            Err(at(Reason::Message(String::clone(&message))))
        }
        Native::Log | Native::Sequence | Native::To => Ok(arg(0)),
        Native::Try => Ok(prelude.ok(arg(0))),
        Native::Bytes => Ok(text::bytes(&arg(0))),
        Native::Utf8 => Ok(text::utf8(&arg(0), prelude)),
        Native::Lines => Ok(text::lines(&arg(0))),
        Native::Words => Ok(text::words(&arg(0))),
        Native::Split => Ok(text::split(&arg(0), &arg(1))),
        Native::Join => Ok(text::join(&arg(0), &arg(1))),
        Native::Lower => Ok(text::lower(&arg(0))),
        Native::Upper => Ok(text::upper(&arg(0))),
        Native::Flatten => {
            let over = arg(0);
            let joined = seq(&over)
                .iter()
                .flat_map(|inner| seq(inner).iter().cloned())
                .collect();
            Ok(Value::seq(joined))
        }
        Native::Collect => Ok(collect(&arg(0), prelude)),
        Native::Count => higher::count(ctx, &arg(0), node),
        Native::Sort => {
            let (by, desc, over) = (arg(0), arg(1), arg(2));
            higher::sort(ctx, &by, prelude.is_true(&desc), &over, node)
        }
    }
}

/// The free `read` reads the meaning its call's result holds: a number
/// by its reader, a program's meaning by its member.
fn read(
    ctx: &Ctx,
    of: Of,
    text: Value,
    result: Option<&ir::Type>,
    node: usize,
) -> Result<Value, Faulted> {
    let target = result.and_then(read_target);
    let of = match target {
        Some(ty) => match super::natives::of_type(ty, &ctx.tables()) {
            Of::Other => {
                if let Some(call) = written_read(ctx, ty, &text) {
                    return calls::invoke(ctx, call, node);
                }
                Of::Other
            }
            of => of,
        },
        None => of,
    };
    let Value::Text(text) = text else {
        unreachable!("text")
    };
    let prelude = &*ctx.prelude;
    Ok(match of {
        Of::F32 => float::read(&text, false, prelude),
        Of::F64 => float::read(&text, true, prelude),
        _ => arith::read(&text, prelude),
    })
}

/// The first `Err` of a sequence of results, else `Ok` of the payloads.
fn collect(over: &Value, prelude: &Prelude) -> Value {
    let mut payloads = Vec::with_capacity(seq(over).len());
    for item in seq(over).iter() {
        match item {
            Value::Record(record) if record.constructor == 0 => {
                payloads.push(record.fields[0].clone());
            }
            Value::Record(_) => return item.clone(),
            other => unreachable!("a result, not {other:?}"),
        }
    }
    prelude.ok(Value::seq(payloads))
}

/// The meaning `read` produces: `T` of its result `Result<T, ParseError>`.
fn read_target(result: &ir::Type) -> Option<&ir::Type> {
    match result {
        ir::Type::Meaning { arguments, .. } => arguments.first(),
        _ => None,
    }
}

/// A program's `read` for a meaning that claims `Read` by one, called
/// with the text; `None` for a meaning the prelude reads.
fn written_read(
    ctx: &Ctx,
    ty: &ir::Type,
    text: &Value,
) -> Option<calls::Invoke> {
    let ir::Type::Meaning { index, .. } = ty else {
        return None;
    };
    let member = ctx.natives.members.read?;
    let (index, meanings) = {
        let tables = ctx.tables();
        let selected =
            super::dispatch::select_by_head(&tables, member, *index)?;
        let ir::Implementation::Definition { index, meanings } =
            &tables.dispatch[member].rows[selected.row].target
        else {
            return None;
        };
        (*index, meanings.clone())
    };
    Some(calls::definition_invoke(
        ctx,
        index,
        vec![text.clone()],
        meanings,
    ))
}
