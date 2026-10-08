//! The natives that answer at once, and the first move of those that
//! call back: the value, or the state their frame starts in.
use super::{
    natives::{Ctx, Native, Of, Step},
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
            done(match &value {
                Value::Text(_) => text::len(&value),
                Value::Seq(parts) => {
                    Value::Int(i64::try_from(parts.len()).expect("fits"))
                }
                other => unreachable!("a length of {other:?}"),
            })
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
            let Value::Text(text) = arg(0) else {
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
        Native::Log | Native::Sequence => done(arg(0)),
        Native::Conversion => {
            unreachable!("a conversion is lowered, never called as a native")
        }
        Native::Pending(label) => todo!("08.2 phase 3: {label}"),
    }
}
