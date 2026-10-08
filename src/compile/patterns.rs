//! Arm lists and patterns: a decision tree of switches and tests over
//! the spots, binds into locals, each arm's body lowered once. The
//! columns' switches are in `cases`, their literal and gate tests in
//! `columns`.
use super::{builder::Builder, dispatch, Lowering};
use crate::{
    ir::{self, Pattern, PatternKind, Type},
    mir::{Operand, Rvalue, Terminator},
};
use alloc::vec::Vec;

/// An arm still possible at a point of the tree: the tests left, in
/// column order, and which arm it is.
#[derive(Clone)]
pub(super) struct Row<'a> {
    pub(super) tests: Vec<(Operand, &'a Pattern)>,
    pub(super) arm: usize,
}

/// The arm list under lowering and each arm's body block, built once.
pub(super) struct Match<'a> {
    pub(super) node: usize,
    pub(super) arms: &'a [ir::Arm],
    pub(super) bodies: Vec<Option<usize>>,
}

/// A switch under construction: what it reads, the place its cases
/// specialize, the keys in order, and whether they leave no other.
pub(super) struct Cases {
    pub(super) value: Operand,
    pub(super) place: Operand,
    pub(super) ty: Type,
    pub(super) keys: Vec<usize>,
    pub(super) covered: bool,
}

/// How a composite value at a place splits into parts.
#[derive(Clone, Copy)]
pub(super) enum Shape {
    Tuple,
    Array,
    Record { meaning: usize, constructor: usize },
    Elements,
    Tag(usize),
}

/// The spot types of a function type, in order, then its result.
fn signature(ty: &Type, arity: usize) -> (Vec<Type>, Type) {
    let mut spots = Vec::with_capacity(arity);
    let mut ty = ty;
    while spots.len() < arity {
        let Type::Function { parameter, result } = ty else {
            unreachable!("a function of {arity} spots")
        };
        spots.push((**parameter).clone());
        ty = result;
    }
    (spots, ty.clone())
}

/// `Bool`'s `True` and `False` constructors: by the unit's layout, or
/// by the prelude's declaration order when the unit never constructed
/// or matched a `Bool` and the checker left the layout empty.
pub(super) fn true_false(
    tables: &ir::Tables,
    meaning: usize,
) -> (usize, usize) {
    let layout = &tables.meanings[meaning].constructors;
    if layout.is_empty() {
        return (0, 1);
    }
    let constructor = |label: &str| {
        layout
            .iter()
            .position(|constructor| constructor.label == label)
            .expect("Bool's constructors")
    };
    (constructor("True"), constructor("False"))
}

/// The meaning a type names, through a function's results.
pub(super) fn result_meaning(ty: &Type) -> usize {
    match ty {
        Type::Function { result, .. } => result_meaning(result),
        Type::Meaning { index, .. } => *index,
        other => unreachable!("a meaning, found {other:?}"),
    }
}

const fn irrefutable(pattern: &Pattern) -> bool {
    matches!(pattern.kind, PatternKind::Any | PatternKind::Slot(_))
}

/// Replace the test at `place` by `with`, keeping column order.
pub(super) fn replace<'a>(
    tests: &mut Vec<(Operand, &'a Pattern)>,
    place: Operand,
    with: Vec<(Operand, &'a Pattern)>,
) {
    let index = tests
        .iter()
        .position(|(at, _)| *at == place)
        .expect("a row tests the place");
    tests.remove(index);
    for (offset, test) in with.into_iter().enumerate() {
        tests.insert(index + offset, test);
    }
}

/// Build a function body from its arms; into `reserved` for a definition.
pub(super) fn build_function<'a>(
    lowering: &mut Lowering<'a>,
    node: usize,
    hidden: &[usize],
    captures: &[usize],
    arms: &'a [ir::Arm],
    reserved: Option<usize>,
) -> usize {
    let arity = arms.first().map_or(0, |arm| arm.patterns.len());
    let (spots, result) = signature(&lowering.tables.nodes[node].ty, arity);
    let mut builder = Builder::new(
        lowering,
        node,
        result,
        hidden.to_vec(),
        captures.to_vec(),
    );
    let plain = match arms {
        [arm] => arm.guard.is_none() && arm.patterns.iter().all(irrefutable),
        _ => false,
    };
    if plain {
        let arm = &arms[0];
        for (pattern, ty) in arm.patterns.iter().zip(spots) {
            let slot = match &pattern.kind {
                PatternKind::Slot(slot) => Some(*slot),
                _ => None,
            };
            builder.spot(slot, ty, node);
        }
        builder.lower_to(arm.body, Some(0));
        builder.terminate(Terminator::Return, node);
        return builder.finish(reserved);
    }
    let places: Vec<Operand> = spots
        .into_iter()
        .map(|ty| Operand::Local(builder.spot(None, ty, node)))
        .collect();
    let rows = arms
        .iter()
        .enumerate()
        .map(|(arm, written)| Row {
            tests: places.iter().copied().zip(&written.patterns).collect(),
            arm,
        })
        .collect();
    let mut context = Match {
        node,
        arms,
        bodies: alloc::vec![None; arms.len()],
    };
    builder.decide(rows, None, &mut context);
    builder.finish(reserved)
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

    /// A branch on `Bool`: `True` to `pass`, `False` to `fail`.
    pub(super) fn switch_bool(
        &mut self,
        value: Operand,
        meaning: usize,
        pass: usize,
        fail: usize,
        node: usize,
    ) {
        let (yes, no) = true_false(self.tables(), meaning);
        self.terminate(
            Terminator::Switch {
                value,
                cases: alloc::vec![(yes, pass), (no, fail)],
                otherwise: None,
            },
            node,
        );
    }

    /// A block no run reaches, or the given miss.
    pub(super) fn miss_block(
        &mut self,
        miss: Option<usize>,
        node: usize,
    ) -> usize {
        miss.unwrap_or_else(|| {
            let saved = self.current;
            let block = self.open(node);
            self.switch_to(block);
            self.terminate(Terminator::Unreachable, node);
            self.switch_to(saved);
            block
        })
    }

    /// Bind each slot pattern to its place.
    fn binds(&mut self, tests: &[(Operand, &Pattern)], node: usize) {
        for (place, pattern) in tests {
            match &pattern.kind {
                PatternKind::Any => (),
                PatternKind::Slot(slot) => {
                    let local = self.slot_local(*slot, node);
                    self.assign(local, Rvalue::Use(*place), node);
                }
                other => unreachable!("a test left to bind: {other:?}"),
            }
        }
    }

    /// An arm's body block, lowered once.
    fn arm_body(&mut self, arms: &mut Match<'a>, arm: usize) -> usize {
        if let Some(block) = arms.bodies[arm] {
            return block;
        }
        let saved = self.current;
        let block = self.open(arms.node);
        arms.bodies[arm] = Some(block);
        self.switch_to(block);
        self.lower_to(arms.arms[arm].body, Some(0));
        self.terminate(Terminator::Return, arms.node);
        self.switch_to(saved);
        block
    }

    /// The tree for the rows still possible, in the current block; a
    /// row that fails every test goes to `miss`.
    pub(super) fn decide(
        &mut self,
        mut rows: Vec<Row<'a>>,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let Some(first) = rows.first() else {
            match miss {
                Some(block) => self.terminate(Terminator::Goto(block), node),
                None => self.terminate(Terminator::Unreachable, node),
            }
            return;
        };
        let Some(column) = first
            .tests
            .iter()
            .position(|(_, pattern)| !irrefutable(pattern))
        else {
            let row = rows.remove(0);
            self.binds(&row.tests, node);
            if let Some(guard) = arms.arms[row.arm].guard {
                let value = self.lower(guard);
                let meaning = result_meaning(&self.tables().nodes[guard].ty);
                let pass = self.open(node);
                let fail = self.open(node);
                self.switch_bool(value, meaning, pass, fail, node);
                self.switch_to(fail);
                self.decide(rows, miss, arms);
                self.switch_to(pass);
            }
            let body = self.arm_body(arms, row.arm);
            self.terminate(Terminator::Goto(body), node);
            return;
        };
        let (place, pattern) = first.tests[column];
        let ty = self.place_type(place);
        let tables = self.tables();
        match &pattern.kind {
            PatternKind::Tuple(_) => {
                self.expand_column(rows, place, Shape::Tuple, miss, arms);
            }
            PatternKind::List(_) if matches!(ty, Type::Array { .. }) => {
                self.expand_column(rows, place, Shape::Array, miss, arms);
            }
            PatternKind::List(_) => {
                self.length_column(&rows, place, miss, arms);
            }
            PatternKind::Constructor {
                meaning,
                constructor,
                ..
            }
            | PatternKind::Record {
                meaning,
                constructor,
                ..
            } if tables.meanings[*meaning].constructors.len() == 1 => {
                let shape = Shape::Record {
                    meaning: *meaning,
                    constructor: *constructor,
                };
                self.expand_column(rows, place, shape, miss, arms);
            }
            PatternKind::Constructor { .. }
            | PatternKind::Record { .. }
            | PatternKind::Side { .. }
            | PatternKind::Tag { .. } => {
                self.switch_column(&rows, place, miss, arms);
            }
            PatternKind::Literal { .. } => {
                self.literal_column(&rows, place, miss, arms);
            }
            PatternKind::Gate { .. } => {
                self.gate_column(&rows, place, miss, arms);
            }
            PatternKind::Any | PatternKind::Slot(_) => unreachable!(),
        }
    }

    /// The pattern a row tests at `place`, if any.
    pub(super) fn at(row: &Row<'a>, place: Operand) -> Option<&'a Pattern> {
        row.tests
            .iter()
            .find(|(at, _)| *at == place)
            .map(|(_, pattern)| *pattern)
    }

    /// Whether a row passes `place` whatever its value.
    pub(super) fn passes(row: &Row<'a>, place: Operand) -> bool {
        Self::at(row, place).is_none_or(irrefutable)
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
