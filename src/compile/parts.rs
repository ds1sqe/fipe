//! The parts of a composite value: how a place splits by its shape,
//! each part read once per branch, and an irrefutable pattern bound
//! part by part.
use super::{builder::Builder, dispatch};
use crate::{
    ir::{Pattern, PatternKind, Type},
    mir::{Operand, Rvalue},
};
use alloc::vec::Vec;

/// How a composite value at a place splits into parts.
#[derive(Clone, Copy)]
pub(super) enum Shape {
    Tuple,
    Array,
    Record { meaning: usize, constructor: usize },
    Elements,
    Tag(usize),
}

impl<'a> Builder<'_, 'a> {
    /// The type of a place, with fixed session meanings substituted.
    pub(super) fn place_type(&self, place: Operand) -> Type {
        let Operand::Local(index) = place else {
            unreachable!("a place is a local")
        };
        self.lowering.fixed(&self.locals[index].ty)
    }

    /// The type of part `index` of a value of `ty` split by `shape`.
    fn part_type(&self, ty: &Type, shape: Shape, index: usize) -> Type {
        match (shape, ty) {
            (Shape::Tuple, Type::Tuple(parts)) => parts[index].clone(),
            (Shape::Array, Type::Array { element, .. }) => (**element).clone(),
            (Shape::Elements, Type::Meaning { arguments, .. }) => {
                arguments[0].clone()
            }
            (
                Shape::Record {
                    meaning,
                    constructor,
                },
                _,
            ) => dispatch::field_type(
                self.tables(),
                ty,
                meaning,
                constructor,
                index,
            ),
            (Shape::Tag(meaning), Type::Packed(members)) => members
                .iter()
                .find(|member| {
                    matches!(member, Type::Meaning { index, .. } if *index == meaning)
                })
                .cloned()
                .unwrap_or(Type::Meaning {
                    index: meaning,
                    arguments: Vec::new(),
                }),
            (_, other) => unreachable!("parts of {other:?}"),
        }
    }

    /// Part `index` of `place`, read once per branch.
    fn part(
        &mut self,
        place: Operand,
        ty: &Type,
        shape: Shape,
        index: usize,
        read: &mut Vec<(usize, Operand)>,
        node: usize,
    ) -> Operand {
        if let Some((_, operand)) = read.iter().find(|(at, _)| *at == index) {
            return *operand;
        }
        let local = self.local(self.part_type(ty, shape, index), Some(node));
        self.assign(local, Rvalue::Field(place, index), node);
        read.push((index, Operand::Local(local)));
        Operand::Local(local)
    }

    /// The sub-tests of the parts of a pattern at `place`.
    pub(super) fn parts(
        &mut self,
        place: Operand,
        ty: &Type,
        shape: Shape,
        parts: Vec<(usize, &'a Pattern)>,
        read: &mut Vec<(usize, Operand)>,
        node: usize,
    ) -> Vec<(Operand, &'a Pattern)> {
        parts
            .into_iter()
            .filter(|(_, part)| !matches!(part.kind, PatternKind::Any))
            .map(|(index, part)| {
                (self.part(place, ty, shape, index, read, node), part)
            })
            .collect()
    }

    /// An irrefutable binding pattern: its parts into locals.
    pub(super) fn bind_pattern(
        &mut self,
        pattern: &'a Pattern,
        place: Operand,
        node: usize,
    ) {
        let ty = self.place_type(place);
        let (shape, parts): (Shape, Vec<(usize, &'a Pattern)>) =
            match &pattern.kind {
                PatternKind::Any => return,
                PatternKind::Slot(slot) => {
                    let local = self.slot_local(*slot, node);
                    self.assign(local, Rvalue::Use(place), node);
                    return;
                }
                PatternKind::Tuple(parts) => {
                    (Shape::Tuple, parts.iter().enumerate().collect())
                }
                PatternKind::List(parts) => {
                    assert!(
                        matches!(ty, Type::Array { .. }),
                        "a list binding takes an array"
                    );
                    (Shape::Array, parts.iter().enumerate().collect())
                }
                PatternKind::Constructor {
                    meaning,
                    constructor,
                    parts,
                } => (
                    Shape::Record {
                        meaning: *meaning,
                        constructor: *constructor,
                    },
                    parts.iter().enumerate().collect(),
                ),
                PatternKind::Record {
                    meaning,
                    constructor,
                    fields,
                } => (
                    Shape::Record {
                        meaning: *meaning,
                        constructor: *constructor,
                    },
                    fields.iter().map(|(index, part)| (*index, part)).collect(),
                ),
                other => unreachable!("a refutable binding pattern {other:?}"),
            };
        let mut read = Vec::new();
        for (index, part) in parts {
            if matches!(part.kind, PatternKind::Any) {
                continue;
            }
            let operand = self.part(place, &ty, shape, index, &mut read, node);
            self.bind_pattern(part, operand, node);
        }
    }
}
