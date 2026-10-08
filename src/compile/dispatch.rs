//! Dispatch rows: the row a receiver selects. The compiler selects by a
//! concrete type; a backend selects by the witness with the same
//! function, so the two never disagree.
use super::{builder::Builder, calls::Arguments};
use crate::{
    ir::{self, Type},
    mir::{self, Callee, Operand, Rvalue},
};
use alloc::vec::Vec;

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
            element: alloc::boxed::Box::new(map(element, leaf)),
            length: alloc::boxed::Box::new(map(length, leaf)),
        },
        Type::Function { parameter, result } => Type::Function {
            parameter: alloc::boxed::Box::new(map(parameter, leaf)),
            result: alloc::boxed::Box::new(map(result, leaf)),
        },
        Type::Not(inner) => Type::Not(alloc::boxed::Box::new(map(inner, leaf))),
    }
}

/// The type of a constructor's field for a value of `ty`, the meaning's
/// parameters replaced by the value's arguments.
pub fn field_type(
    tables: &ir::Tables,
    ty: &Type,
    meaning: usize,
    constructor: usize,
    field: usize,
) -> Type {
    let declared = &tables.meanings[meaning];
    let arguments = match ty {
        Type::Meaning { index, arguments } if *index == meaning => arguments,
        _ => &Vec::new(),
    };
    let bindings: Vec<_> = declared
        .parameters
        .iter()
        .copied()
        .zip(arguments.iter().cloned())
        .collect();
    substitute(
        &declared.constructors[constructor].fields[field].ty,
        &bindings,
    )
}

impl Builder<'_, '_> {
    /// A dispatch with every spot in hand: its row's target when the
    /// receiver is concrete, else the run-time callee with every row's
    /// body lowered. A field row is read in place.
    pub(super) fn dispatched(
        &mut self,
        member: usize,
        by: &ir::Receiver,
        arguments: &Arguments,
        dest: usize,
        node: usize,
    ) -> Option<(Callee, Vec<ir::Type>)> {
        let tables = self.tables();
        let argument = arguments.argument.as_ref();
        let witness = match by {
            ir::Receiver::Meaning(ty) => self.lowering.fixed(ty),
            ir::Receiver::Value(value) => {
                self.lowering.fixed(&tables.nodes[*value].ty)
            }
        };
        let selected = concrete(&witness)
            .then(|| select(tables, member, &witness, argument))
            .flatten();
        let Some(selected) = selected else {
            for row in &tables.dispatch[member].rows {
                if let ir::Implementation::Definition { index, .. } = row.target
                {
                    self.lowering.function_body(index);
                }
            }
            let witness = match by {
                ir::Receiver::Value(_) => mir::Witness::Value(
                    arguments.receiver.expect("a value receiver is supplied"),
                ),
                ir::Receiver::Meaning(_) => mir::Witness::Meaning(witness),
            };
            return Some((Callee::Dispatch { member, witness }, Vec::new()));
        };
        let row = &tables.dispatch[member].rows[selected.row];
        Some(match &row.target {
            ir::Implementation::Field(index) => {
                let value = arguments.args[row.receiver_spot.unwrap_or(0)];
                self.assign(dest, Rvalue::Field(value, *index), node);
                return None;
            }
            ir::Implementation::Definition { index, meanings } => {
                let meanings = meanings
                    .iter()
                    .map(|ty| substitute(ty, &selected.bindings))
                    .collect();
                self.lowering.function_body(*index).map_or_else(
                    || (Callee::Value(Operand::Global(*index)), Vec::new()),
                    |body| (Callee::Body(body), meanings),
                )
            }
            ir::Implementation::Declared(index) => {
                (self.declared(*index), Vec::new())
            }
            ir::Implementation::Derived { kind, ty } => (
                Callee::Derived {
                    kind: *kind,
                    ty: substitute(ty, &selected.bindings),
                },
                Vec::new(),
            ),
        })
    }
}
