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

/// An outcome's good and bad constructors, by label: `Just`, `Ok` and
/// `Right` are good; `Either` declares its bad side, `Left`, first.
pub(super) fn sides(tables: &ir::Tables, ty: &ir::Type) -> (usize, usize) {
    let constructors = &tables.meanings[meaning_of(tables, ty)].constructors;
    let position = |labels: [&str; 3]| {
        constructors
            .iter()
            .position(|constructor| labels.contains(&&*constructor.label))
            .expect("an outcome's sides")
    };
    (
        position(["Just", "Ok", "Right"]),
        position(["Nothing", "Err", "Left"]),
    )
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
    ty.substituted(parameters, arguments)
}
