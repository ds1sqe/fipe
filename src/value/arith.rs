//! `Integer`: exact and unbounded. `Int` by checked operations, `Big`
//! over the lexer's magnitude; a result that fits is always `Int`.
use super::{Prelude, Reason, Value};
use crate::token::Magnitude;
use alloc::{format, string::String};
use core::cmp::Ordering;

/// A sign and a magnitude, the slow path's view of either form.
struct Signed {
    negative: bool,
    magnitude: Magnitude,
}

impl Signed {
    fn of(value: &Value) -> Self {
        match value {
            Value::Int(small) => Self {
                negative: *small < 0,
                magnitude: Magnitude::from(small.unsigned_abs()),
            },
            Value::Big(big) => Self {
                negative: big.negative,
                magnitude: big.magnitude.clone(),
            },
            other => unreachable!("an integer, not {other:?}"),
        }
    }

    fn value(self) -> Value {
        Value::integer(self.negative, self.magnitude)
    }

    /// Signed addition by magnitudes.
    fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            return Self {
                negative: self.negative,
                magnitude: self.magnitude.add(&other.magnitude),
            };
        }
        match self.magnitude.cmp(&other.magnitude) {
            Ordering::Less => Self {
                negative: other.negative,
                magnitude: other
                    .magnitude
                    .checked_sub(&self.magnitude)
                    .expect("the larger magnitude"),
            },
            _ => Self {
                negative: self.negative,
                magnitude: self
                    .magnitude
                    .checked_sub(&other.magnitude)
                    .expect("the larger magnitude"),
            },
        }
    }

    const fn negated(mut self) -> Self {
        self.negative = !self.negative;
        self
    }
}

const fn pair(a: &Value, b: &Value) -> Option<(i64, i64)> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some((*a, *b)),
        _ => None,
    }
}

#[must_use]
pub fn add(a: &Value, b: &Value) -> Value {
    if let Some((a, b)) = pair(a, b) {
        if let Some(sum) = a.checked_add(b) {
            return Value::Int(sum);
        }
    }
    Signed::of(a).add(&Signed::of(b)).value()
}

#[must_use]
pub fn sub(a: &Value, b: &Value) -> Value {
    if let Some((a, b)) = pair(a, b) {
        if let Some(difference) = a.checked_sub(b) {
            return Value::Int(difference);
        }
    }
    Signed::of(a).add(&Signed::of(b).negated()).value()
}

#[must_use]
pub fn mul(a: &Value, b: &Value) -> Value {
    if let Some((a, b)) = pair(a, b) {
        if let Some(product) = a.checked_mul(b) {
            return Value::Int(product);
        }
    }
    let (a, b) = (Signed::of(a), Signed::of(b));
    Value::integer(
        a.negative != b.negative
            && !a.magnitude.is_zero()
            && !b.magnitude.is_zero(),
        a.magnitude.multiply(&b.magnitude),
    )
}

/// Quotient toward zero and remainder with the dividend's sign.
fn div_rem(a: &Value, b: &Value) -> Result<(Value, Value), Reason> {
    if let Some((a, b)) = pair(a, b) {
        if b == 0 {
            return Err(Reason::DivideByZero);
        }
        if let (Some(quotient), Some(remainder)) =
            (a.checked_div(b), a.checked_rem(b))
        {
            return Ok((Value::Int(quotient), Value::Int(remainder)));
        }
    }
    let (a, b) = (Signed::of(a), Signed::of(b));
    let (quotient, remainder) = a
        .magnitude
        .div_rem(&b.magnitude)
        .ok_or(Reason::DivideByZero)?;
    Ok((
        Value::integer(a.negative != b.negative, quotient),
        Value::integer(a.negative, remainder),
    ))
}

/// # Errors
/// `DivideByZero` for a zero divisor.
pub fn div(a: &Value, b: &Value) -> Result<Value, Reason> {
    div_rem(a, b).map(|(quotient, _)| quotient)
}

/// # Errors
/// `DivideByZero` for a zero divisor.
pub fn rem(a: &Value, b: &Value) -> Result<Value, Reason> {
    div_rem(a, b).map(|(_, remainder)| remainder)
}

#[must_use]
pub fn neg(a: &Value) -> Value {
    match a {
        Value::Int(small) => small
            .checked_neg()
            .map_or_else(|| Signed::of(a).negated().value(), Value::Int),
        _ => Signed::of(a).negated().value(),
    }
}

#[must_use]
pub fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Big(a), Value::Big(b)) => a == b,
        _ => false,
    }
}

#[must_use]
pub fn cmp(a: &Value, b: &Value) -> Ordering {
    if let Some((a, b)) = pair(a, b) {
        return a.cmp(&b);
    }
    let (a, b) = (Signed::of(a), Signed::of(b));
    match (a.negative, b.negative) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => a.magnitude.cmp(&b.magnitude),
        (true, true) => b.magnitude.cmp(&a.magnitude),
    }
}

#[must_use]
pub const fn zero() -> Value {
    Value::Int(0)
}

/// Decimal digits with a leading `-` when negative.
#[must_use]
pub fn show(a: &Value) -> String {
    match a {
        Value::Int(small) => format!("{small}"),
        Value::Big(big) => {
            format!("{}{}", if big.negative { "-" } else { "" }, big.magnitude)
        }
        other => unreachable!("an integer, not {other:?}"),
    }
}

/// `Ok` of the integer a decimal spelling with an optional sign
/// denotes, else `Err (ParseError message)`.
#[must_use]
pub fn read(text: &str, prelude: &Prelude) -> Value {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let magnitude = (!digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| Magnitude::parse(digits, 10))
    .flatten();
    magnitude.map_or_else(
        || prelude.parse_error(&format!("not an integer: {text:?}")),
        |magnitude| {
            prelude.ok(Value::integer(text.starts_with('-'), magnitude))
        },
    )
}

/// The integer a written form denotes: a whole form's value, or a
/// fraction form that denotes a whole number exactly.
///
/// # Errors
/// `NotExact` for a fraction with a fractional part.
pub fn literal(form: &Value) -> Result<Value, Reason> {
    let Value::Record(record) = form else {
        unreachable!("a form is a record")
    };
    match (record.constructor, record.fields.as_slice()) {
        (0, [value, ..]) => Ok(value.clone()),
        (1, [digits, exponent, ..]) => {
            let spelling = format!("{}e{}", show(digits), show(exponent));
            let (negative, magnitude) = Magnitude::whole_of_fraction(&spelling)
                .ok_or_else(|| Reason::NotExact {
                    value: spelling.clone(),
                    target: "Integer".into(),
                })?;
            Ok(Value::integer(negative, magnitude))
        }
        _ => unreachable!("a number form"),
    }
}
