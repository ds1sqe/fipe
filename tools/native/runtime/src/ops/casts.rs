//! The casts' rows, copied from `src/vm/casts.rs`: a number exact or
//! `NotExact`, a reader's result unwrapped, text to bytes, and the rows
//! that pass the value.
use fipe::{
    ir,
    token::Magnitude,
    value::{arith, float, Prelude, Reason, Value},
};

/// A cast by its row to the target type.
///
/// # Errors
/// `NotExact` on the number row, naming the value and the target, `Cast` on a reader's `Err`.
pub(super) fn cast(
    row: ir::CastRow,
    to: &ir::Type,
    value: Value,
    prelude: &Prelude,
) -> Result<Value, Reason> {
    match row {
        ir::CastRow::Number => number(to, value, prelude),
        ir::CastRow::Read => match &value {
            Value::Record(record) if record.constructor == 0 => {
                Ok(record.fields[0].clone())
            }
            _ => Err(Reason::Cast),
        },
        // A shown or encoded value is text by now: at `Vec<U8>` it
        // enters as its bytes, as text does.
        ir::CastRow::Bytes | ir::CastRow::Show | ir::CastRow::Codec => {
            Ok(bytes_at(to, value, prelude))
        }
    }
}

/// Text at a `Vec<U8>` place becomes its bytes; anything else passes.
fn bytes_at(to: &ir::Type, value: Value, prelude: &Prelude) -> Value {
    match (&value, to) {
        (Value::Text(_), ir::Type::Meaning { index, .. })
            if Some(*index) == prelude.vec =>
        {
            fipe::value::text::bytes(&value)
        }
        _ => value,
    }
}

/// The number row: exact, or `NotExact`.
fn number(
    to: &ir::Type,
    value: Value,
    prelude: &Prelude,
) -> Result<Value, Reason> {
    let ir::Type::Meaning { index, .. } = to else {
        unreachable!("a number meaning")
    };
    let index = Some(*index);
    if index == prelude.integer {
        return match &value {
            Value::Int(_) | Value::Big(_) => Ok(value),
            Value::F32(_) | Value::F64(_) => {
                exact_integer(float::Float::of(&value).wide())
                    .ok_or_else(|| not_exact(&value, "Integer"))
            }
            other => unreachable!("a number, not {other:?}"),
        };
    }
    let wide = index == prelude.f64;
    let target = if wide { "F64" } else { "F32" };
    let converted = match &value {
        Value::Int(small) => {
            #[allow(clippy::cast_precision_loss)] // checked below
            let float = *small as f64;
            (float, exact_integer(float) == Some(value.clone()))
        }
        Value::Big(_) => {
            let float: f64 =
                arith::show(&value).parse().unwrap_or(f64::INFINITY);
            (float, exact_integer(float) == Some(value.clone()))
        }
        Value::F32(_) | Value::F64(_) => {
            (float::Float::of(&value).wide(), true)
        }
        other => unreachable!("a number, not {other:?}"),
    };
    let (float, exact) = converted;
    if !exact {
        return Err(not_exact(&value, target));
    }
    if wide {
        return float::Float::F64(float).finite();
    }
    #[allow(clippy::cast_possible_truncation)] // checked below
    let narrow = float as f32;
    if f64::from(narrow).to_bits() == float.to_bits() {
        float::Float::F32(narrow).finite()
    } else {
        Err(not_exact(&value, target))
    }
}

/// `NotExact` naming a number in the language's spelling and the
/// target it does not fit exactly.
fn not_exact(value: &Value, target: &str) -> Reason {
    let value = match value {
        Value::F32(_) | Value::F64(_) => float::show(value),
        _ => arith::show(value),
    };
    Reason::NotExact {
        value,
        target: target.into(),
    }
}

/// The integer a float holds exactly, when it is integral.
fn exact_integer(float: f64) -> Option<Value> {
    if !float.is_finite() || float::trunc64(float).to_bits() != float.to_bits()
    {
        return None;
    }
    let bits = float.to_bits();
    let negative = bits >> 63 == 1;
    let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("11 bits");
    let fraction = bits & ((1 << 52) - 1);
    if exponent == 0 {
        return Some(Value::Int(0));
    }
    let mantissa = fraction | (1 << 52);
    let shift = exponent - 1075;
    if shift <= 0 {
        let magnitude = mantissa >> (-shift).min(63);
        return Some(Value::integer(negative, Magnitude::from(magnitude)));
    }
    let mut magnitude = Magnitude::from(mantissa);
    let mut shift = u32::try_from(shift).expect("positive");
    while shift > 0 {
        let step = shift.min(32);
        magnitude = magnitude.multiply(&Magnitude::from(1u64 << step));
        shift -= step;
    }
    Some(Value::integer(negative, magnitude))
}
