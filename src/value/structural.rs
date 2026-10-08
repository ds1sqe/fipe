//! Equality, order, display and defaults of any value by its tag,
//! walking with an explicit work stack: the derived members, and the
//! pure fallback when no meaning on the walk has a written member.
use super::{Prelude, Reason, Record, Value};
use crate::ir;
use alloc::{rc::Rc, string::String, vec::Vec};
use core::cmp::Ordering;

/// The children of two values of one shape, or `None` when they differ
/// at this node; `Some(empty)` when they are equal leaves.
fn pairs<'a>(
    a: &'a Value,
    b: &'a Value,
) -> Option<Vec<(&'a Value, &'a Value)>> {
    match (a, b) {
        (Value::Unit, Value::Unit) => Some(Vec::new()),
        (Value::Int(_) | Value::Big(_), Value::Int(_) | Value::Big(_)) => {
            super::arith::eq(a, b).then(Vec::new)
        }
        (Value::F32(_) | Value::F64(_), Value::F32(_) | Value::F64(_)) => {
            super::float::eq(a, b).then(Vec::new)
        }
        (Value::Text(_), Value::Text(_)) => {
            super::text::eq(a, b).then(Vec::new)
        }
        (Value::Seq(a), Value::Seq(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
            (a.len() == b.len()).then(|| a.iter().zip(b.iter()).collect())
        }
        (Value::Record(a), Value::Record(b)) => (a.meaning == b.meaning
            && a.constructor == b.constructor
            && a.fields.len() == b.fields.len())
        .then(|| a.fields.iter().zip(b.fields.iter()).collect()),
        (Value::Packed(a), Value::Packed(b)) => {
            (a.meaning == b.meaning).then(|| alloc::vec![(&a.value, &b.value)])
        }
        (Value::Function(a), Value::Function(b)) => {
            Rc::ptr_eq(a, b).then(Vec::new)
        }
        (Value::Meaning(a), Value::Meaning(b)) => (a == b).then(Vec::new),
        _ => None,
    }
}

/// Structural equality: records by meaning, constructor and fields,
/// sequences and tuples by elements, numbers by value, text by bytes.
#[must_use]
pub fn eq(a: &Value, b: &Value) -> bool {
    let mut work = alloc::vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        match pairs(a, b) {
            None => return false,
            Some(children) => work.extend(children.into_iter().rev()),
        }
    }
    true
}

/// Work for the order: a pair still to compare, or an order decided
/// at a node that counts once its children are equal.
enum Work<'a> {
    Pair(&'a Value, &'a Value),
    Decided(Ordering),
}

/// The order at one node, and the work its children leave: the
/// children first, then the order decided here.
fn order<'a>(a: &'a Value, b: &'a Value, work: &mut Vec<Work<'a>>) -> Ordering {
    let mut children = |a: &'a [Value], b: &'a [Value], after: Ordering| {
        work.push(Work::Decided(after));
        work.extend(
            a.iter().zip(b.iter()).rev().map(|(a, b)| Work::Pair(a, b)),
        );
        Ordering::Equal
    };
    match (a, b) {
        (Value::Int(_) | Value::Big(_), Value::Int(_) | Value::Big(_)) => {
            super::arith::cmp(a, b)
        }
        (Value::F32(_) | Value::F64(_), Value::F32(_) | Value::F64(_)) => {
            super::float::cmp(a, b)
        }
        (Value::Text(_), Value::Text(_)) => super::text::cmp(a, b),
        (Value::Seq(a), Value::Seq(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
            // Elements first, in order; a shorter sequence with the same
            // prefix orders before a longer one.
            children(a, b, a.len().cmp(&b.len()))
        }
        (Value::Record(a), Value::Record(b)) => {
            match a.constructor.cmp(&b.constructor) {
                Ordering::Equal => {
                    children(&a.fields, &b.fields, Ordering::Equal)
                }
                decided => decided,
            }
        }
        (Value::Packed(a), Value::Packed(b)) => match a.meaning.cmp(&b.meaning)
        {
            Ordering::Equal => {
                work.push(Work::Pair(&a.value, &b.value));
                Ordering::Equal
            }
            decided => decided,
        },
        (Value::Unit, Value::Unit)
        | (Value::Function(_), Value::Function(_))
        | (Value::Meaning(_), Value::Meaning(_)) => Ordering::Equal,
        _ => unreachable!("values of one meaning"),
    }
}

/// Structural order: constructors by index then fields, sequences and
/// tuples element by element then by length.
#[must_use]
pub fn cmp(a: &Value, b: &Value) -> Ordering {
    let mut work = alloc::vec![Work::Pair(a, b)];
    while let Some(item) = work.pop() {
        let ordering = match item {
            Work::Decided(ordering) => ordering,
            Work::Pair(a, b) => order(a, b, &mut work),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// A piece of the display still to write: a value, as an argument or
/// not, or text already decided.
enum Piece<'a> {
    Value(&'a Value, bool),
    Text(&'a str),
}

/// The language's spelling: `Just 2`, `Point 1 2`, `[1, 2]`, `(1, "a")`,
/// `"a"`, `0.5`, `()`, a function `<function>`; a record argument that
/// has spaces is parenthesized.
#[must_use]
pub fn show<'a>(value: &'a Value, tables: &'a ir::Tables) -> String {
    let mut out = String::new();
    let mut work = alloc::vec![Piece::Value(value, false)];
    while let Some(piece) = work.pop() {
        let (value, argument) = match piece {
            Piece::Text(text) => {
                out.push_str(text);
                continue;
            }
            Piece::Value(value, argument) => (value, argument),
        };
        match value {
            Value::Unit => out.push_str("()"),
            Value::Int(_) | Value::Big(_) => {
                out.push_str(&super::arith::show(value));
            }
            Value::F32(_) | Value::F64(_) => {
                out.push_str(&super::float::show(value));
            }
            Value::Text(_) => out.push_str(&super::text::show(value)),
            Value::Function(_) => out.push_str("<function>"),
            Value::Meaning(_) => out.push_str("<meaning>"),
            Value::Packed(packed) => {
                work.push(Piece::Value(&packed.value, argument));
            }
            Value::Seq(parts) | Value::Tuple(parts) => {
                let (open, close) = if matches!(value, Value::Seq(_)) {
                    ("[", "]")
                } else {
                    ("(", ")")
                };
                work.push(Piece::Text(close));
                for (index, part) in parts.iter().enumerate().rev() {
                    work.push(Piece::Value(part, false));
                    if index > 0 {
                        work.push(Piece::Text(", "));
                    }
                }
                work.push(Piece::Text(open));
            }
            Value::Record(record) => {
                let parenthesize = argument && !record.fields.is_empty();
                if parenthesize {
                    work.push(Piece::Text(")"));
                }
                for field in record.fields.iter().rev() {
                    work.push(Piece::Value(field, true));
                    work.push(Piece::Text(" "));
                }
                // An opaque type, `LimitError`, has no constructors.
                let meaning = &tables.meanings[record.meaning];
                let label = meaning
                    .constructors
                    .get(record.constructor)
                    .map_or(&meaning.label, |constructor| &constructor.label);
                work.push(Piece::Text(label));
                if parenthesize {
                    work.push(Piece::Text("("));
                }
            }
        }
    }
    out
}

/// The default of a type: `0`, `0.0`, `""`, `[]`, `Nothing`, `False`,
/// `()`, a tuple or array of defaults, a record of its fields' defaults.
///
/// # Errors
/// `Message` for a type with no default the checker could have derived,
/// which is a crate defect in disguise.
pub fn default(
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
) -> Result<Value, Reason> {
    match ty {
        ir::Type::Unit => Ok(Value::Unit),
        ir::Type::Tuple(parts) => Ok(Value::Tuple(Rc::new(
            parts
                .iter()
                .map(|part| default(part, tables, prelude))
                .collect::<Result<_, _>>()?,
        ))),
        ir::Type::Array { element, length } => {
            let ir::Type::Number(length) = &**length else {
                return Err(Reason::Message(String::from(
                    "an array length is a number",
                )));
            };
            let length = usize::try_from(length.to_u64().unwrap_or(u64::MAX))
                .unwrap_or(usize::MAX);
            let element = default(element, tables, prelude)?;
            Ok(Value::seq(alloc::vec![element; length]))
        }
        ir::Type::Meaning { index, arguments } => {
            let index = *index;
            if Some(index) == prelude.integer {
                return Ok(Value::Int(0));
            }
            if Some(index) == prelude.f32 {
                return Ok(Value::F32(0.0));
            }
            if Some(index) == prelude.f64 {
                return Ok(Value::F64(0.0));
            }
            if Some(index) == prelude.string {
                return Ok(Value::text(""));
            }
            if Some(index) == prelude.vec || Some(index) == prelude.deque {
                return Ok(Value::seq(Vec::new()));
            }
            if Some(index) == prelude.maybe {
                return Ok(prelude.nothing());
            }
            if Some(index) == prelude.bool {
                return Ok(prelude.bool(false));
            }
            let meaning = &tables.meanings[index];
            let [constructor] = meaning.constructors.as_slice() else {
                return Err(Reason::Message(alloc::format!(
                    "{} has no default",
                    meaning.label
                )));
            };
            let fields = constructor
                .fields
                .iter()
                .map(|field| {
                    let ty =
                        field.ty.substituted(&meaning.parameters, arguments);
                    default(&ty, tables, prelude)
                })
                .collect::<Result<_, _>>()?;
            Ok(Value::Record(Rc::new(Record {
                meaning: index,
                constructor: 0,
                fields,
            })))
        }
        other => {
            Err(Reason::Message(alloc::format!("no default for {other:?}")))
        }
    }
}
