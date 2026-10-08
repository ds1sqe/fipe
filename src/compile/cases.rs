//! The switches of a decision tree: a constructor, side, tag or length
//! column split into one branch per key, each row specialized to it.
use super::{
    builder::Builder,
    patterns::{replace, result_meaning, Cases, Match, Row, Shape},
};
use crate::{
    ir::{Pattern, PatternKind, Type},
    mir::{Operand, Rvalue, Terminator},
};
use alloc::vec::Vec;

impl<'a> Builder<'_, 'a> {
    /// Split an irrefutable composite at `place` into its parts, in the
    /// current block, for every row that writes one.
    pub(super) fn expand_column(
        &mut self,
        mut rows: Vec<Row<'a>>,
        place: Operand,
        shape: Shape,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let ty = self.place_type(place);
        let mut read = Vec::new();
        for row in &mut rows {
            let Some(pattern) = Self::at(row, place) else {
                continue;
            };
            let parts: Vec<(usize, &'a Pattern)> = match &pattern.kind {
                PatternKind::Any | PatternKind::Slot(_) => continue,
                PatternKind::Tuple(parts)
                | PatternKind::List(parts)
                | PatternKind::Constructor { parts, .. } => {
                    parts.iter().enumerate().collect()
                }
                PatternKind::Record { fields, .. } => fields
                    .iter()
                    .map(|(index, pattern)| (*index, pattern))
                    .collect(),
                other => unreachable!("a composite, found {other:?}"),
            };
            let with = self.parts(place, &ty, shape, parts, &mut read, node);
            replace(&mut row.tests, place, with);
        }
        self.decide(rows, miss, arms);
    }

    /// A row specialized to the constructor it tests at `place`: its
    /// parts become tests on the parts read in the branch.
    fn specialize(
        &mut self,
        row: &Row<'a>,
        place: Operand,
        ty: &Type,
        read: &mut Vec<(usize, Operand)>,
        node: usize,
    ) -> Row<'a> {
        let mut row = row.clone();
        let Some(pattern) = Self::at(&row, place) else {
            return row;
        };
        let (shape, parts): (Shape, Vec<(usize, &'a Pattern)>) =
            match &pattern.kind {
                PatternKind::Any | PatternKind::Slot(_) => return row,
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
                PatternKind::Side { good, payload } => (
                    Shape::Record {
                        meaning: result_meaning(ty),
                        constructor: usize::from(!*good),
                    },
                    payload.iter().map(|part| (0, &**part)).collect(),
                ),
                PatternKind::Tag { meaning, payload } => {
                    (Shape::Tag(*meaning), alloc::vec![(0, &**payload)])
                }
                PatternKind::List(parts) => {
                    (Shape::Elements, parts.iter().enumerate().collect())
                }
                other => unreachable!("a case, found {other:?}"),
            };
        let with = self.parts(place, ty, shape, parts, read, node);
        replace(&mut row.tests, place, with);
        row
    }

    /// The key a pattern switches on: a constructor, a side, a tag.
    fn key(pattern: &Pattern) -> Option<usize> {
        match &pattern.kind {
            PatternKind::Constructor { constructor, .. }
            | PatternKind::Record { constructor, .. } => Some(*constructor),
            PatternKind::Side { good, .. } => Some(usize::from(!*good)),
            PatternKind::Tag { meaning, .. } => Some(*meaning),
            PatternKind::List(parts) => Some(parts.len()),
            _ => None,
        }
    }

    /// A switch on the constructor, side or tag at `place`, one branch
    /// per key written, in order; `otherwise` only when a key is missing.
    pub(super) fn switch_column(
        &mut self,
        rows: &[Row<'a>],
        place: Operand,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let ty = self.place_type(place);
        let tables = self.tables();
        let mut keys: Vec<usize> = Vec::new();
        let mut total = None;
        for row in rows {
            let Some(pattern) = Self::at(row, place) else {
                continue;
            };
            let Some(key) = Self::key(pattern) else {
                continue;
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
            total = Some(match &pattern.kind {
                PatternKind::Constructor { meaning, .. }
                | PatternKind::Record { meaning, .. } => {
                    tables.meanings[*meaning].constructors.len()
                }
                PatternKind::Side { .. } => 2,
                _ => match &ty {
                    Type::Packed(members) => members.len(),
                    _ => usize::MAX,
                },
            });
        }
        let covered = total == Some(keys.len());
        let cases = Cases {
            value: place,
            place,
            ty,
            keys,
            covered,
        };
        self.branches(cases, rows, miss, arms);
    }

    /// A switch on the length of the sequence at `place`; a length never
    /// completes a column, so the default always exists.
    pub(super) fn length_column(
        &mut self,
        rows: &[Row<'a>],
        place: Operand,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let ty = self.place_type(place);
        let tables = self.tables();
        let integer = tables
            .meanings
            .iter()
            .position(|meaning| {
                Some(meaning.namespace) == self.lowering.prelude
                    && meaning.label == "Integer"
            })
            .expect("a unit with a sequence names Integer");
        let length = self.local(
            Type::Meaning {
                index: integer,
                arguments: Vec::new(),
            },
            Some(node),
        );
        self.assign(length, Rvalue::Length(place), node);
        let mut keys = Vec::new();
        for row in rows {
            if let Some(key) = Self::at(row, place).and_then(Self::key) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        let cases = Cases {
            value: Operand::Local(length),
            place,
            ty,
            keys,
            covered: false,
        };
        self.branches(cases, rows, miss, arms);
    }

    /// One block per key, the rows that pass or test that key in it; the
    /// rows that pass whatever the key in `otherwise`.
    fn branches(
        &mut self,
        cases: Cases,
        rows: &[Row<'a>],
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let Cases {
            value,
            place,
            ty,
            keys,
            covered,
        } = cases;
        let defaults: Vec<Row<'a>> = rows
            .iter()
            .filter(|row| Self::passes(row, place))
            .cloned()
            .collect();
        let blocks: Vec<usize> = keys.iter().map(|_| self.open(node)).collect();
        let otherwise = if covered {
            None
        } else if defaults.is_empty() {
            Some(self.miss_block(miss, node))
        } else {
            Some(self.open(node))
        };
        self.terminate(
            Terminator::Switch {
                value,
                cases: keys
                    .iter()
                    .copied()
                    .zip(blocks.iter().copied())
                    .collect(),
                otherwise,
            },
            node,
        );
        for (key, block) in keys.iter().zip(blocks) {
            self.switch_to(block);
            let mut read = Vec::new();
            let mut branch = Vec::new();
            for row in rows {
                match Self::at(row, place).and_then(Self::key) {
                    Some(written) if written == *key => {
                        branch.push(
                            self.specialize(row, place, &ty, &mut read, node),
                        );
                    }
                    Some(_) => (),
                    None => branch.push(row.clone()),
                }
            }
            self.decide(branch, miss, arms);
        }
        if let Some(block) = otherwise {
            if !defaults.is_empty() {
                self.switch_to(block);
                self.decide(defaults, miss, arms);
            }
        }
    }
}
