//! `F32` and `F64`: IEEE 754 binary32 and binary64, ties to even.
//!
//! No value is infinite or NaN: an operation whose result would be one
//! Faults `NotFinite`. The roundings and the functions `core` lacks
//! without `std` are written here.
mod kernels;

use super::{Prelude, Reason, Value};
use alloc::{format, string::String};
use core::cmp::Ordering;
pub use kernels::{atan2_64, atan64, cos64, hypot64, sin64, sqrt64};

/// A float of either width, for the operations that share a rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Float {
    F32(f32),
    F64(f64),
}

impl Float {
    #[must_use]
    pub fn of(value: &Value) -> Self {
        match value {
            Value::F32(float) => Self::F32(*float),
            Value::F64(float) => Self::F64(*float),
            other => unreachable!("a float, not {other:?}"),
        }
    }

    /// The value when finite, else `NotFinite`.
    /// # Errors
    /// `NotFinite` for an infinity or a NaN.
    pub const fn finite(self) -> Result<Value, Reason> {
        match self {
            Self::F32(float) if float.is_finite() => Ok(Value::F32(float)),
            Self::F64(float) if float.is_finite() => Ok(Value::F64(float)),
            _ => Err(Reason::NotFinite),
        }
    }

    #[must_use]
    pub fn wide(self) -> f64 {
        match self {
            Self::F32(float) => f64::from(float),
            Self::F64(float) => float,
        }
    }

    /// The result of an `f64` computation at this width; exact for the
    /// roundings, whose results are representable.
    #[allow(clippy::cast_possible_truncation)] // the rounding is the point
    const fn narrowed(self, result: f64) -> Self {
        match self {
            Self::F32(_) => Self::F32(result as f32),
            Self::F64(_) => Self::F64(result),
        }
    }
}

/// An arithmetic member on two floats of one width.
fn binary(
    a: &Value,
    b: &Value,
    narrow: fn(f32, f32) -> f32,
    wide: fn(f64, f64) -> f64,
) -> Result<Value, Reason> {
    match (Float::of(a), Float::of(b)) {
        (Float::F32(a), Float::F32(b)) => Float::F32(narrow(a, b)).finite(),
        (Float::F64(a), Float::F64(b)) => Float::F64(wide(a, b)).finite(),
        _ => unreachable!("floats of one width"),
    }
}

/// # Errors
/// `NotFinite` when the sum overflows.
pub fn add(a: &Value, b: &Value) -> Result<Value, Reason> {
    binary(a, b, |a, b| a + b, |a, b| a + b)
}

/// # Errors
/// `NotFinite` when the difference overflows.
pub fn sub(a: &Value, b: &Value) -> Result<Value, Reason> {
    binary(a, b, |a, b| a - b, |a, b| a - b)
}

/// # Errors
/// `NotFinite` when the product overflows.
pub fn mul(a: &Value, b: &Value) -> Result<Value, Reason> {
    binary(a, b, |a, b| a * b, |a, b| a * b)
}

/// # Errors
/// `NotFinite` for a zero divisor or an overflow.
pub fn div(a: &Value, b: &Value) -> Result<Value, Reason> {
    binary(a, b, |a, b| a / b, |a, b| a / b)
}

/// The remainder with the dividend's sign, `core`'s `%`.
///
/// # Errors
/// `NotFinite` for a zero divisor.
pub fn rem(a: &Value, b: &Value) -> Result<Value, Reason> {
    binary(a, b, |a, b| a % b, |a, b| a % b)
}

#[must_use]
pub fn neg(a: &Value) -> Value {
    match Float::of(a) {
        Float::F32(float) => Value::F32(-float),
        Float::F64(float) => Value::F64(-float),
    }
}

/// `-0.0` equals `0.0`.
#[must_use]
pub fn eq(a: &Value, b: &Value) -> bool {
    Float::of(a).wide() == Float::of(b).wide()
}

/// Total on finite values; `-0.0` and `0.0` are `Equal`.
#[must_use]
pub fn cmp(a: &Value, b: &Value) -> Ordering {
    Float::of(a)
        .wide()
        .partial_cmp(&Float::of(b).wide())
        .unwrap_or(Ordering::Equal)
}

#[must_use]
pub const fn zero(wide: bool) -> Value {
    if wide {
        Value::F64(0.0)
    } else {
        Value::F32(0.0)
    }
}

/// The shortest decimal that reads back to the value, with a fraction
/// part and no exponent; `-0.0` keeps its sign.
#[must_use]
pub fn show(a: &Value) -> String {
    let text = match Float::of(a) {
        Float::F32(float) => format!("{float}"),
        Float::F64(float) => format!("{float}"),
    };
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// `Ok` of the nearest float of the width a decimal spelling denotes,
/// else `Err (ParseError message)`; a non-finite spelling is an error.
#[must_use]
pub fn read(text: &str, wide: bool, prelude: &Prelude) -> Value {
    let parsed = if wide {
        text.parse::<f64>().ok().map(Float::F64)
    } else {
        text.parse::<f32>().ok().map(Float::F32)
    };
    match parsed.map(Float::finite) {
        Some(Ok(value)) => prelude.ok(value),
        _ => prelude.parse_error(&format!("not a number: {text:?}")),
    }
}

/// The nearest float of the width to a written whole or fraction form.
///
/// # Errors
/// `NotFinite` when the form is beyond the width's range.
pub fn literal(form: &Value, wide: bool) -> Result<Value, Reason> {
    let Value::Record(record) = form else {
        unreachable!("a form is a record")
    };
    let spelling = match (record.constructor, record.fields.as_slice()) {
        (0, [value, ..]) => super::arith::show(value),
        (1, [digits, exponent, ..]) => format!(
            "{}e{}",
            super::arith::show(digits),
            super::arith::show(exponent)
        ),
        _ => unreachable!("a number form"),
    };
    let parsed = if wide {
        Float::F64(spelling.parse().unwrap_or(f64::NAN))
    } else {
        Float::F32(spelling.parse().unwrap_or(f32::NAN))
    };
    parsed.finite()
}

/// A rounding applied at the value's width through `f64`, exact since
/// every `f32` is an `f64` and the result is an integer the width holds.
fn rounding(a: &Value, round: fn(f64) -> f64) -> Value {
    let float = Float::of(a);
    match float.narrowed(round(float.wide())) {
        Float::F32(result) => Value::F32(result),
        Float::F64(result) => Value::F64(result),
    }
}

#[must_use]
pub fn trunc(a: &Value) -> Value {
    rounding(a, trunc64)
}

#[must_use]
pub fn floor(a: &Value) -> Value {
    rounding(a, floor64)
}

#[must_use]
pub fn ceil(a: &Value) -> Value {
    rounding(a, ceil64)
}

/// Ties to even.
#[must_use]
pub fn round(a: &Value) -> Value {
    rounding(a, round64)
}

/// Beyond this every `f64` is an integer.
const INTEGRAL: f64 = 4_503_599_627_370_496.0;

#[must_use]
pub fn trunc64(x: f64) -> f64 {
    if x.abs() >= INTEGRAL {
        return x;
    }
    // Below 2^52 the cast to an integer truncates toward zero exactly
    // and the cast back is exact; the sign of a zero is restored.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let whole = (x as i64) as f64;
    whole.copysign(x)
}

#[must_use]
pub fn floor64(x: f64) -> f64 {
    let t = trunc64(x);
    if x < t {
        t - 1.0
    } else {
        t
    }
}

#[must_use]
pub fn ceil64(x: f64) -> f64 {
    let t = trunc64(x);
    if x > t {
        t + 1.0
    } else {
        t
    }
}

/// Ties to even; the sign of a zero result is the argument's.
#[must_use]
pub fn round64(x: f64) -> f64 {
    if x.abs() >= INTEGRAL {
        return x;
    }
    let f = floor64(x);
    let d = x - f;
    let r = if d < 0.5 {
        f
    } else if d > 0.5 {
        f + 1.0
    } else if f % 2.0 == 0.0 {
        f
    } else {
        f + 1.0
    };
    r.copysign(x)
}

/// A unary function on `F64` with the finiteness test.
///
/// # Errors
/// `NotFinite` when the result is not a number or not finite.
fn unary(a: &Value, function: fn(f64) -> f64) -> Result<Value, Reason> {
    Float::F64(function(Float::of(a).wide())).finite()
}

/// # Errors
/// `NotFinite` for a negative argument.
pub fn sqrt(a: &Value) -> Result<Value, Reason> {
    unary(a, sqrt64)
}

/// # Errors
/// Never for a finite argument; the signature is the members'.
pub fn sin(a: &Value) -> Result<Value, Reason> {
    unary(a, sin64)
}

/// # Errors
/// Never for a finite argument; the signature is the members'.
pub fn cos(a: &Value) -> Result<Value, Reason> {
    unary(a, cos64)
}

/// # Errors
/// Never for finite arguments; the signature is the members'.
pub fn atan2(y: &Value, x: &Value) -> Result<Value, Reason> {
    Float::F64(atan2_64(Float::of(y).wide(), Float::of(x).wide())).finite()
}

/// # Errors
/// `NotFinite` when the length overflows.
pub fn hypot(x: &Value, y: &Value) -> Result<Value, Reason> {
    Float::F64(hypot64(Float::of(x).wide(), Float::of(y).wide())).finite()
}
