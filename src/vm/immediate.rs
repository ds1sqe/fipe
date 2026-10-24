//! The natives that answer at once, and the first move of those that
//! call back: the value, or the state their frame starts in.
use super::{
    natives::{Ctx, Native, Of, Step},
    order::{Count, Sort},
    pending::Pending,
    walk::Walk,
};
use crate::value::{arith, float, structural, text, Reason, Value};
use alloc::{rc::Rc, string::String, vec::Vec};

pub(super) fn seq(value: &Value) -> &Rc<Vec<Value>> {
    match value {
        Value::Seq(parts) => parts,
        other => unreachable!("a sequence, not {other:?}"),
    }
}

pub(super) const fn is_float(value: &Value) -> bool {
    matches!(value, Value::F32(_) | Value::F64(_))
}

/// A native with its arguments: the value, or the frame to start.
#[allow(clippy::too_many_lines)]
pub(super) fn immediate(
    native: Native,
    mut args: Vec<Value>,
    ctx: &Ctx<'_>,
) -> Result<Step, Reason> {
    let prelude = ctx.prelude;
    let mut arg =
        |index: usize| core::mem::replace(&mut args[index], Value::Unit);
    let done = |value: Value| Ok(Step::Done(value));
    match native {
        Native::Add | Native::Sub | Native::Mul | Native::Div | Native::Rem => {
            // `other: Self -> (self: Self)`: the receiver is spot 1.
            let (other, this) = (arg(0), arg(1));
            let value = if is_float(&this) {
                match native {
                    Native::Add => float::add(&this, &other)?,
                    Native::Sub => float::sub(&this, &other)?,
                    Native::Mul => float::mul(&this, &other)?,
                    Native::Div => float::div(&this, &other)?,
                    _ => float::rem(&this, &other)?,
                }
            } else {
                match native {
                    Native::Add => arith::add(&this, &other),
                    Native::Sub => arith::sub(&this, &other),
                    Native::Mul => arith::mul(&this, &other),
                    Native::Div => arith::div(&this, &other)?,
                    _ => arith::rem(&this, &other)?,
                }
            };
            done(value)
        }
        Native::Eq | Native::Cmp => {
            let (other, this) = (arg(0), arg(1));
            if super::step::leaf(&this) && super::step::leaf(&other) {
                return done(if native == Native::Eq {
                    prelude.bool(structural::eq(&this, &other))
                } else {
                    prelude.ordering(structural::cmp(&this, &other))
                });
            }
            let walk = if native == Native::Eq {
                Walk::eq(this, other)
            } else {
                Walk::cmp(this, other)
            };
            Ok(Step::Start(Pending::Walk(walk)))
        }
        Native::Show => {
            let value = arg(0);
            if super::step::leaf(&value) {
                return done(super::step::show_leaf(&value, ctx.tables));
            }
            Ok(Step::Start(Pending::Walk(Walk::show(value))))
        }
        Native::Len => {
            let value = arg(0);
            match &value {
                Value::Text(_) => done(text::len(&value)),
                Value::Seq(parts) => {
                    done(Value::Int(i64::try_from(parts.len()).expect("fits")))
                }
                // Any other value's meaning claims `Len` by a program's
                // member.
                _ => Ok(Pending::started(super::pending::member_call(
                    ctx,
                    ctx.natives.members.len,
                    &value,
                    Vec::new(),
                ))),
            }
        }
        Native::Not => done(prelude.bool(!prelude.is_true(&arg(0)))),
        Native::Concat => {
            let (other, this) = (arg(0), arg(1));
            done(match (&this, &other) {
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
            let value = match of {
                Of::Integer => arith::literal(&form)?,
                Of::F32 => float::literal(&form, false)?,
                Of::F64 => float::literal(&form, true)?,
                Of::Text | Of::Bytes | Of::Seq => match &form {
                    Value::Record(record) => record.fields[0].clone(),
                    other => unreachable!("a form, not {other:?}"),
                },
                other => unreachable!("a literal of {other:?}"),
            };
            done(value)
        }
        Native::Read(of) => {
            let text = arg(0);
            // The free `read` reads the meaning its call's result holds:
            // a number by its reader, a program's meaning by its member.
            let target = ctx.result.and_then(read_target);
            let of = match target {
                Some(ty) => match super::natives::of_type(ty, ctx.tables) {
                    Of::Other => {
                        if let Some(step) = written_read(ty, &text, ctx)? {
                            return Ok(Pending::started(step));
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
            done(match of {
                Of::F32 => float::read(&text, false, prelude),
                Of::F64 => float::read(&text, true, prelude),
                _ => arith::read(&text, prelude),
            })
        }
        Native::Zero(of) => done(match of {
            Of::F32 => Value::F32(0.0),
            Of::F64 => Value::F64(0.0),
            _ => arith::zero(),
        }),
        Native::Default(_) => done(Value::seq(Vec::new())),
        Native::Round => done(float::round(&arg(0))),
        Native::Trunc => done(float::trunc(&arg(0))),
        Native::Floor => done(float::floor(&arg(0))),
        Native::Ceil => done(float::ceil(&arg(0))),
        Native::Sqrt => done(float::sqrt(&arg(0))?),
        Native::Sin => done(float::sin(&arg(0))?),
        Native::Cos => done(float::cos(&arg(0))?),
        Native::Atan2 => done(float::atan2(&arg(0), &arg(1))?),
        Native::Hypot => done(float::hypot(&arg(0), &arg(1))?),
        Native::Map => {
            let (function, over) = (arg(0), arg(1));
            Ok(Pending::map(function, over))
        }
        Native::Pure(of) => done(match of {
            Of::Result => prelude.ok(arg(0)),
            _ => prelude.just(arg(0)),
        }),
        Native::Then => {
            let (function, over) = (arg(0), arg(1));
            match &over {
                Value::Record(record) if record.constructor == 0 => {
                    let payload = record.fields[0].clone();
                    Ok(Step::Start(Pending::Then(Some((function, payload)))))
                }
                _ => done(over),
            }
        }
        Native::Empty => done(prelude.nothing()),
        Native::Or => {
            let (other, this) = (arg(0), arg(1));
            done(match &this {
                Value::Record(record) if record.constructor == 0 => this,
                _ => other,
            })
        }
        Native::Fold => {
            let (init, function, over) = (arg(0), arg(1), arg(2));
            Ok(Pending::fold(init, function, &over))
        }
        Native::Filter => {
            let (function, over) = (arg(0), arg(1));
            Ok(Pending::filter(function, &over))
        }
        Native::Sum => {
            let over = arg(0);
            Ok(Pending::sum(&over))
        }
        Native::First => {
            let over = arg(0);
            done(seq(&over).first().map_or_else(
                || prelude.nothing(),
                |first| prelude.just(first.clone()),
            ))
        }
        Native::Head => {
            let over = arg(0);
            seq(&over).first().cloned().map_or_else(
                || {
                    Err(Reason::Message(String::from(
                        "head of an empty sequence",
                    )))
                },
                done,
            )
        }
        Native::Take => {
            let (count, over) = (arg(0), arg(1));
            let count = match count {
                Value::Int(count) => usize::try_from(count).unwrap_or(0),
                _ => usize::MAX,
            };
            done(Value::seq(seq(&over).iter().take(count).cloned().collect()))
        }
        Native::Fault => {
            let Value::Text(message) = arg(0) else {
                unreachable!("text")
            };
            Err(Reason::Message(String::clone(&message)))
        }
        Native::Log | Native::Sequence | Native::To => done(arg(0)),
        Native::Try => done(prelude.ok(arg(0))),
        Native::Bytes => done(text::bytes(&arg(0))),
        Native::Utf8 => done(text::utf8(&arg(0), prelude)),
        Native::Lines => done(text::lines(&arg(0))),
        Native::Words => done(text::words(&arg(0))),
        Native::Split => done(text::split(&arg(0), &arg(1))),
        Native::Join => done(text::join(&arg(0), &arg(1))),
        Native::Lower => done(text::lower(&arg(0))),
        Native::Upper => done(text::upper(&arg(0))),
        Native::Flatten => {
            let over = arg(0);
            let joined = seq(&over)
                .iter()
                .flat_map(|inner| seq(inner).iter().cloned())
                .collect();
            done(Value::seq(joined))
        }
        Native::Collect => done(collect(&arg(0), prelude)),
        Native::Drop => {
            let (count, over) = (arg(0), arg(1));
            let count = match count {
                Value::Int(count) => usize::try_from(count).unwrap_or(0),
                _ => usize::MAX,
            };
            done(Value::seq(seq(&over).iter().skip(count).cloned().collect()))
        }
        Native::Last => {
            let over = arg(0);
            done(seq(&over).last().map_or_else(
                || prelude.nothing(),
                |last| prelude.just(last.clone()),
            ))
        }
        Native::Nth => {
            let (at, over) = (arg(0), arg(1));
            let at = match at {
                Value::Int(at) => usize::try_from(at).ok(),
                _ => None,
            };
            done(
                at.and_then(|at| seq(&over).get(at).cloned()).map_or_else(
                    || prelude.nothing(),
                    |item| prelude.just(item),
                ),
            )
        }
        Native::Reverse => {
            let over = arg(0);
            done(Value::seq(seq(&over).iter().rev().cloned().collect()))
        }
        Native::Enumerate => {
            let over = arg(0);
            let pairs = seq(&over)
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let index = i64::try_from(index).expect("a position fits");
                    Value::Tuple(Rc::new(alloc::vec![
                        Value::Int(index),
                        item.clone()
                    ]))
                })
                .collect();
            done(Value::seq(pairs))
        }
        Native::Zip => {
            let (left, right) = (arg(0), arg(1));
            let pairs = seq(&left)
                .iter()
                .zip(seq(&right).iter())
                .map(|(a, b)| {
                    Value::Tuple(Rc::new(alloc::vec![a.clone(), b.clone()]))
                })
                .collect();
            done(Value::seq(pairs))
        }
        Native::Contains => {
            done(prelude.bool(text::contains(&arg(0), &arg(1))))
        }
        Native::StartsWith => {
            done(prelude.bool(text::starts_with(&arg(0), &arg(1))))
        }
        Native::EndsWith => {
            done(prelude.bool(text::ends_with(&arg(0), &arg(1))))
        }
        Native::Replace => done(text::replace(&arg(0), &arg(1), &arg(2))),
        Native::Count => Ok(Count::start(&arg(0))),
        Native::Sort => {
            let (by, desc, over) = (arg(0), arg(1), arg(2));
            Ok(Sort::start(by, prelude.is_true(&desc), &over))
        }
    }
}

/// The first `Err` of a sequence of results, else `Ok` of the payloads.
fn collect(over: &Value, prelude: &crate::value::Prelude) -> Value {
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
fn read_target(result: &crate::ir::Type) -> Option<&crate::ir::Type> {
    match result {
        crate::ir::Type::Meaning { arguments, .. } => arguments.first(),
        _ => None,
    }
}

/// A program's `read` for a meaning that claims `Read` by one, called
/// with the text; `None` for a meaning the prelude reads.
fn written_read(
    ty: &crate::ir::Type,
    text: &Value,
    ctx: &Ctx<'_>,
) -> Result<Option<Step>, Reason> {
    if !matches!(ty, crate::ir::Type::Meaning { .. }) {
        return Ok(None);
    }
    let Some(member) = ctx.natives.members.read else {
        return Ok(None);
    };
    let Some(selected) = super::dispatch::select_for(
        ctx.tables,
        ctx.prelude,
        member,
        Some(ty),
        None,
        ctx.work,
    )?
    else {
        return Ok(None);
    };
    Ok(super::pending::supplied_target(
        ctx,
        &ctx.tables.dispatch[member].rows[selected.row].target,
        &selected.bindings,
        alloc::vec![text.clone()],
        ctx.result,
    )?
    .map(Step::Call))
}
