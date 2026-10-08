//! The tests of a decision tree: a literal column compared by its
//! recorded equality, a gate column tested by its predicate.
use super::{
    builder::Builder,
    calls::Arguments,
    patterns::{replace, result_meaning, Match, Row},
};
use crate::{
    ir::{self, Pattern, PatternKind, Type},
    mir::{Operand, Terminator},
};
use alloc::vec::Vec;

impl<'a> Builder<'_, 'a> {
    /// Equality with the literal at `place` by its recorded operation,
    /// then a branch on the `Bool`.
    pub(super) fn literal_column(
        &mut self,
        rows: &[Row<'a>],
        place: Operand,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let first = Self::at(&rows[0], place).expect("the column's pattern");
        let PatternKind::Literal {
            value,
            equality,
            self_spot,
        } = &first.kind
        else {
            unreachable!("a literal column")
        };
        let tables = self.tables();
        let form = |pattern: &Pattern| match &pattern.kind {
            PatternKind::Literal { value, .. } => {
                match &tables.nodes[*value].kind {
                    ir::Kind::Literal { form, .. } => Some(form),
                    _ => None,
                }
            }
            _ => None,
        };
        let written = form(first);
        let literal = self.lower(*value);
        let target = self.callee(*equality);
        let args = if *self_spot == 0 {
            alloc::vec![place, literal]
        } else {
            alloc::vec![literal, place]
        };
        let ty = self.place_type(place);
        let result = &tables.nodes[*equality].ty;
        let bool = self.local(
            Type::Meaning {
                index: result_meaning(result),
                arguments: Vec::new(),
            },
            None,
        );
        let arguments = Arguments {
            args,
            receiver: Some(literal),
            argument: Some(ty),
        };
        self.apply(target, arguments, bool, node);
        let pass = self.open(node);
        let fail = self.open(node);
        self.switch_bool(
            Operand::Local(bool),
            result_meaning(result),
            pass,
            fail,
            node,
        );
        let same = |row: &Row<'a>| {
            Self::at(row, place).is_some_and(|pattern| form(pattern) == written)
        };
        let passing: Vec<Row<'a>> = rows
            .iter()
            .filter(|row| same(row) || Self::passes(row, place))
            .map(|row| {
                let mut row = row.clone();
                if same(&row) {
                    replace(&mut row.tests, place, Vec::new());
                }
                row
            })
            .collect();
        let failing: Vec<Row<'a>> =
            rows.iter().filter(|row| !same(row)).cloned().collect();
        self.switch_to(pass);
        self.decide(passing, miss, arms);
        self.switch_to(fail);
        self.decide(failing, miss, arms);
    }

    /// The gate at `place`: its predicate tested, the payload matched
    /// on the pass edge.
    pub(super) fn gate_column(
        &mut self,
        rows: &[Row<'a>],
        place: Operand,
        miss: Option<usize>,
        arms: &mut Match<'a>,
    ) {
        let node = arms.node;
        let first = Self::at(&rows[0], place).expect("the column's pattern");
        let PatternKind::Gate { predicate, .. } = &first.kind else {
            unreachable!("a gate column")
        };
        self.lowering.reach_predicate(predicate);
        let pass = self.open(node);
        let fail = self.open(node);
        self.terminate(
            Terminator::Test {
                predicate: predicate.clone(),
                value: place,
                pass,
                fail,
            },
            node,
        );
        let same = |row: &Row<'a>| {
            matches!(
                Self::at(row, place).map(|pattern| &pattern.kind),
                Some(PatternKind::Gate { predicate: written, .. }) if written == predicate
            )
        };
        let passing: Vec<Row<'a>> = rows
            .iter()
            .filter(|row| same(row) || Self::passes(row, place))
            .map(|row| {
                let mut row = row.clone();
                if let Some(PatternKind::Gate { payload, .. }) =
                    Self::at(&row, place).map(|pattern| &pattern.kind)
                {
                    let payload: &'a Pattern = payload;
                    let with = if matches!(payload.kind, PatternKind::Any) {
                        Vec::new()
                    } else {
                        alloc::vec![(place, payload)]
                    };
                    replace(&mut row.tests, place, with);
                }
                row
            })
            .collect();
        let failing: Vec<Row<'a>> =
            rows.iter().filter(|row| !same(row)).cloned().collect();
        self.switch_to(pass);
        self.decide(passing, miss, arms);
        self.switch_to(fail);
        self.decide(failing, miss, arms);
    }
}
