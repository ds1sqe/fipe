//! Run-time dispatch, copied from `src/compile/dispatch.rs` (a row by a
//! concrete type or by a head meaning, a type's parameters bound or its
//! fixed session meanings substituted: the witness substitution) and
//! `src/vm/dispatch.rs` (a row by a witness or by a value's tag, a
//! packed receiver unpacked).
use fipe::{
    ir::{self, Type},
    value::{Prelude, Value},
};

/// A selected row and the row's parameters bound by the match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub row: usize,
    pub bindings: Vec<(usize, Type)>,
}

/// The row of `member` for a receiver and the first other argument:
/// matched by head meaning, then by the argument's meaning; a field row
/// wins over a member row.
pub fn select(
    tables: &ir::Tables,
    member: usize,
    receiver: &Type,
    argument: Option<&Type>,
) -> Option<Selected> {
    let mut found = None;
    for (index, row) in tables.dispatch[member].rows.iter().enumerate() {
        let mut bindings = Vec::new();
        if !unify(&row.receiver, receiver, &mut bindings) {
            continue;
        }
        if let (Some(pattern), Some(actual)) = (&row.argument, argument) {
            if !unify(pattern, actual, &mut bindings) {
                continue;
            }
        }
        let selected = Selected {
            row: index,
            bindings,
        };
        if matches!(row.target, ir::Implementation::Field(_)) {
            return Some(selected);
        }
        found.get_or_insert(selected);
    }
    found
}

/// Whether a row's type matches an actual one, binding the row's
/// parameters consistently.
fn unify(
    pattern: &Type,
    actual: &Type,
    bindings: &mut Vec<(usize, Type)>,
) -> bool {
    match (pattern, actual) {
        (Type::Parameter(parameter), actual) => {
            if let Some((_, bound)) =
                bindings.iter().find(|(bound, _)| bound == parameter)
            {
                bound == actual
            } else {
                bindings.push((*parameter, actual.clone()));
                true
            }
        }
        (
            Type::Meaning {
                index: a,
                arguments: x,
            },
            Type::Meaning {
                index: b,
                arguments: y,
            },
        ) => a == b && all(x, y, bindings),
        (Type::Tuple(x), Type::Tuple(y))
        | (Type::Packed(x), Type::Packed(y))
        | (Type::All(x), Type::All(y))
        | (Type::Sum(x), Type::Sum(y)) => all(x, y, bindings),
        (
            Type::Array {
                element: a,
                length: n,
            },
            Type::Array {
                element: b,
                length: m,
            },
        ) => unify(a, b, bindings) && unify(n, m, bindings),
        (
            Type::Function {
                parameter: a,
                result: x,
            },
            Type::Function {
                parameter: b,
                result: y,
            },
        ) => unify(a, b, bindings) && unify(x, y, bindings),
        (Type::Not(a), Type::Not(b)) => unify(a, b, bindings),
        (Type::Unit, Type::Unit) => true,
        (Type::Number(a), Type::Number(b)) => a == b,
        (Type::Session(a), Type::Session(b)) => a == b,
        _ => false,
    }
}

fn all(
    patterns: &[Type],
    actuals: &[Type],
    bindings: &mut Vec<(usize, Type)>,
) -> bool {
    patterns.len() == actuals.len()
        && patterns
            .iter()
            .zip(actuals)
            .all(|(pattern, actual)| unify(pattern, actual, bindings))
}

/// The row of `member` for a value known only by its tag, the head
/// meaning of its type: the first row whose receiver is that meaning,
/// a field row winning; the row's parameters stay unbound.
#[must_use]
pub fn select_by_head(
    tables: &ir::Tables,
    member: usize,
    meaning: usize,
) -> Option<Selected> {
    let mut found = None;
    for (index, row) in tables.dispatch[member].rows.iter().enumerate() {
        let head = match &row.receiver {
            Type::Meaning { index, .. } => *index,
            _ => continue,
        };
        if head != meaning {
            continue;
        }
        let selected = Selected {
            row: index,
            bindings: Vec::new(),
        };
        if matches!(row.target, ir::Implementation::Field(_)) {
            return Some(selected);
        }
        found.get_or_insert(selected);
    }
    found
}

/// Whether a type names no parameter and no unfixed session meaning,
/// so a dispatch on it is decided at compile time.
pub fn concrete(ty: &Type) -> bool {
    match ty {
        Type::Parameter(_) | Type::Session(_) => false,
        Type::Unit | Type::Number(_) => true,
        Type::Meaning { arguments, .. } => arguments.iter().all(concrete),
        Type::Tuple(parts)
        | Type::Packed(parts)
        | Type::All(parts)
        | Type::Sum(parts) => parts.iter().all(concrete),
        Type::Array { element, length } => {
            concrete(element) && concrete(length)
        }
        Type::Function { parameter, result } => {
            concrete(parameter) && concrete(result)
        }
        Type::Not(inner) => concrete(inner),
    }
}

/// A type with each parameter replaced by its binding.
pub fn substitute(ty: &Type, bindings: &[(usize, Type)]) -> Type {
    map(ty, &|ty| match ty {
        Type::Parameter(parameter) => bindings
            .iter()
            .find(|(bound, _)| bound == parameter)
            .map(|(_, ty)| ty.clone()),
        _ => None,
    })
}

/// A type with every fixed session meaning substituted.
pub fn fixed(tables: &ir::Tables, ty: &Type) -> Type {
    map(ty, &|ty| match ty {
        Type::Session(index) => tables.session_meanings[*index]
            .as_ref()
            .map(|fixed| self::fixed(tables, fixed)),
        _ => None,
    })
}

/// A type rebuilt bottom-up, `leaf` replacing the nodes it answers.
fn map(ty: &Type, leaf: &dyn Fn(&Type) -> Option<Type>) -> Type {
    if let Some(replaced) = leaf(ty) {
        return replaced;
    }
    let parts = |parts: &[Type]| parts.iter().map(|ty| map(ty, leaf)).collect();
    match ty {
        Type::Unit
        | Type::Parameter(_)
        | Type::Session(_)
        | Type::Number(_) => ty.clone(),
        Type::Meaning { index, arguments } => Type::Meaning {
            index: *index,
            arguments: parts(arguments),
        },
        Type::Tuple(inner) => Type::Tuple(parts(inner)),
        Type::Packed(inner) => Type::Packed(parts(inner)),
        Type::All(inner) => Type::All(parts(inner)),
        Type::Sum(inner) => Type::Sum(parts(inner)),
        Type::Array { element, length } => Type::Array {
            element: Box::new(map(element, leaf)),
            length: Box::new(map(length, leaf)),
        },
        Type::Function { parameter, result } => Type::Function {
            parameter: Box::new(map(parameter, leaf)),
            result: Box::new(map(result, leaf)),
        },
        Type::Not(inner) => Type::Not(Box::new(map(inner, leaf))),
    }
}

/// The meaning a value's tag names: a record's, or the prelude's for a
/// number, text or sequence.
pub fn tag(value: &Value, prelude: &Prelude) -> Option<usize> {
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
pub fn select_for(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    ty: Option<&ir::Type>,
    value: Option<&Value>,
) -> Option<Selected> {
    if let Some(ty) = ty {
        if concrete(ty) {
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
pub fn unpack(
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
pub fn by_tag(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    value: &Value,
) -> Option<Selected> {
    let meaning = tag(value, prelude)?;
    select_by_head(tables, member, meaning)
}
