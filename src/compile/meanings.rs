//! Meanings read from types: outcome and `Bool` indices, field types
//! under an outcome's arguments.
use crate::ir;

/// The meaning index of an outcome or `Bool` type, through a fixed
/// session slot.
pub(super) fn meaning_of(tables: &ir::Tables, ty: &ir::Type) -> usize {
    match ty {
        ir::Type::Meaning { index, .. } => *index,
        ir::Type::Session(slot) => meaning_of(
            tables,
            tables.session_meanings[*slot]
                .as_ref()
                .expect("a switched session meaning is fixed"),
        ),
        other => unreachable!("a meaning of {other:?}"),
    }
}

/// `Bool`'s `True` and `False` constructors.
pub(super) fn bool_cases(tables: &ir::Tables, ty: &ir::Type) -> (usize, usize) {
    let constructors = &tables.meanings[meaning_of(tables, ty)].constructors;
    let position = |label: &str| {
        constructors
            .iter()
            .position(|constructor| constructor.label == label)
            .expect("Bool's constructors")
    };
    (position("True"), position("False"))
}

/// A field's type with the meaning's parameters replaced by the
/// outcome's arguments.
pub(super) fn instantiate(
    ty: &ir::Type,
    parameters: &[usize],
    outcome: &ir::Type,
) -> ir::Type {
    let ir::Type::Meaning { arguments, .. } = outcome else {
        unreachable!("an outcome is a meaning")
    };
    replace(ty, parameters, arguments)
}

pub(super) fn replace(
    ty: &ir::Type,
    parameters: &[usize],
    arguments: &[ir::Type],
) -> ir::Type {
    let each = |parts: &[ir::Type]| {
        parts
            .iter()
            .map(|part| replace(part, parameters, arguments))
            .collect()
    };
    let one = |part: &ir::Type| {
        alloc::boxed::Box::new(replace(part, parameters, arguments))
    };
    match ty {
        ir::Type::Parameter(parameter) => parameters
            .iter()
            .position(|known| known == parameter)
            .and_then(|position| arguments.get(position))
            .map_or_else(|| ty.clone(), Clone::clone),
        ir::Type::Unit | ir::Type::Session(_) | ir::Type::Number(_) => {
            ty.clone()
        }
        ir::Type::Meaning {
            index,
            arguments: inner,
        } => ir::Type::Meaning {
            index: *index,
            arguments: each(inner),
        },
        ir::Type::Tuple(parts) => ir::Type::Tuple(each(parts)),
        ir::Type::Packed(parts) => ir::Type::Packed(each(parts)),
        ir::Type::All(parts) => ir::Type::All(each(parts)),
        ir::Type::Sum(parts) => ir::Type::Sum(each(parts)),
        ir::Type::Array { element, length } => ir::Type::Array {
            element: one(element),
            length: one(length),
        },
        ir::Type::Function { parameter, result } => ir::Type::Function {
            parameter: one(parameter),
            result: one(result),
        },
        ir::Type::Not(inner) => ir::Type::Not(one(inner)),
    }
}
