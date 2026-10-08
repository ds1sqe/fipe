//! `String`: UTF-8 text. Equality and order are by bytes; `len`
//! counts Unicode scalar values, as the lexer counts columns.
use super::{Prelude, Value};
use alloc::{rc::Rc, string::String, vec::Vec};
use core::cmp::Ordering;
use core::fmt::Write;

fn text(value: &Value) -> &str {
    match value {
        Value::Text(text) => text,
        other => unreachable!("text, not {other:?}"),
    }
}

#[must_use]
pub fn concat(a: &Value, b: &Value) -> Value {
    let mut joined = String::with_capacity(text(a).len() + text(b).len());
    joined.push_str(text(a));
    joined.push_str(text(b));
    Value::Text(Rc::new(joined))
}

/// The number of Unicode scalar values.
#[must_use]
pub fn len(a: &Value) -> Value {
    Value::Int(i64::try_from(text(a).chars().count()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn eq(a: &Value, b: &Value) -> bool {
    text(a) == text(b)
}

#[must_use]
pub fn cmp(a: &Value, b: &Value) -> Ordering {
    text(a).as_bytes().cmp(text(b).as_bytes())
}

/// The text quoted with the lexer's escapes, so it reads back.
#[must_use]
pub fn show(a: &Value) -> String {
    quoted(text(a))
}

#[must_use]
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            ch if ch.is_control() => {
                for byte in ch.encode_utf8(&mut [0; 4]).bytes() {
                    let _ = write!(out, "\\x{byte:02X}");
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// The UTF-8 bytes as a `Vec<U8>`.
#[must_use]
pub fn bytes(a: &Value) -> Value {
    Value::bytes(text(a).as_bytes())
}

/// `Ok` of the text a `Vec<U8>` encodes, else `Err (Utf8Error at)` with
/// the first failing byte position.
///
/// # Panics
/// On an element that is not a byte: a checker invariant.
#[must_use]
pub fn utf8(a: &Value, prelude: &Prelude) -> Value {
    let Value::Seq(parts) = a else {
        unreachable!("a sequence of bytes")
    };
    let bytes: Vec<u8> = parts
        .iter()
        .map(|part| match part {
            Value::Int(byte) => u8::try_from(*byte).expect("a U8"),
            other => unreachable!("a byte, not {other:?}"),
        })
        .collect();
    match String::from_utf8(bytes) {
        Ok(text) => prelude.ok(Value::Text(Rc::new(text))),
        Err(error) => prelude.utf8_error(error.utf8_error().valid_up_to()),
    }
}

fn texts<'a>(parts: impl Iterator<Item = &'a str>) -> Value {
    Value::seq(parts.map(Value::text).collect())
}

/// Lines split at LF, a CR before the LF dropped, no final empty line.
#[must_use]
pub fn lines(a: &Value) -> Value {
    texts(text(a).lines())
}

/// Words split at Unicode white space, none empty.
#[must_use]
pub fn words(a: &Value) -> Value {
    texts(text(a).split_whitespace())
}

/// The parts between each occurrence of the separator.
#[must_use]
pub fn split(at: &Value, a: &Value) -> Value {
    texts(text(a).split(text(at)))
}

/// The texts of a sequence joined by the separator.
#[must_use]
pub fn join(sep: &Value, parts: &Value) -> Value {
    let Value::Seq(parts) = parts else {
        unreachable!("a sequence of texts")
    };
    let joined = parts.iter().map(text).collect::<Vec<_>>().join(text(sep));
    Value::Text(Rc::new(joined))
}

/// Unicode lowercase.
#[must_use]
pub fn lower(a: &Value) -> Value {
    Value::Text(Rc::new(text(a).to_lowercase()))
}

/// Unicode uppercase.
#[must_use]
pub fn upper(a: &Value) -> Value {
    Value::Text(Rc::new(text(a).to_uppercase()))
}
