//! Outcomes, fallbacks and orderings: switches on a side or a `Bool`.
use super::{
    builder::Builder,
    meanings::{bool_cases, instantiate, meaning_of, sides},
};
use crate::{
    ir,
    mir::{Callee, Operand, Rvalue, Terminator},
};
use alloc::{vec, vec::Vec};

impl<'a> Builder<'_, 'a> {
    /// An outcome operation or a comparison, or an adapter. A branching
    /// node is counted once on each path, in the branch, so every
    /// evaluation pays it once.
    pub(super) fn outcome(
        &mut self,
        node: usize,
        kind: &'a ir::Kind,
        dest: Option<usize>,
    ) -> Operand {
        match kind {
            ir::Kind::Marked {
                good,
                left,
                stage,
                rebuild,
            } => self.marked(node, *good, *left, *stage, *rebuild, dest),
            ir::Kind::Lift {
                pattern,
                value,
                body,
                rebuild,
            } => self.lift(node, pattern, *value, *body, *rebuild, dest),
            ir::Kind::Or { mode, left, right } => {
                self.fallback(node, *mode, *left, *right, dest)
            }
            ir::Kind::And {
                mode,
                rebuild,
                left,
                right,
            } => self.sequencing(node, *mode, *rebuild, *left, *right, dest),
            ir::Kind::Ordering { value, accepted } => {
                self.ordering(node, *value, accepted, dest)
            }
            ir::Kind::Cast { .. }
            | ir::Kind::Convert { .. }
            | ir::Kind::Pack { .. }
            | ir::Kind::Enter { .. } => self.adapter(node, kind, dest),
            other => unreachable!("an outcome of {other:?}"),
        }
    }

    /// A marked pipe: the inactive side keeps its value; the marked side's
    /// payload goes through the stage and the result is rebuilt.
    fn marked(
        &mut self,
        node: usize,
        good: bool,
        left: usize,
        stage: usize,
        rebuild: ir::Rebuild,
        dest: Option<usize>,
    ) -> Operand {
        let ty = &self.tables().nodes[node].ty;
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let value = self.lower(left);
        let (good_side, bad_side) =
            sides(self.tables(), &self.tables().nodes[left].ty);
        let (side, other) = if good {
            (good_side, bad_side)
        } else {
            (bad_side, good_side)
        };
        let marked = self.open(node);
        let kept = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value,
                cases: vec![(side, marked), (other, kept)],
                otherwise: None,
            },
            node,
        );
        self.switch_to(kept);
        self.count();
        self.assign(dest, Rvalue::Use(value), node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(marked);
        let result = self.stage(left, value, side, stage);
        self.count();
        self.rebuild(rebuild, side, ty, result, dest, node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(join);
        Operand::Local(dest)
    }

    /// The marked side's stage: a value when the side has no payload, a
    /// call whose supply of the left reads the payload instead, or a
    /// function applied to the payload. The payload read is the stage's.
    fn stage(
        &mut self,
        left: usize,
        value: Operand,
        side: usize,
        stage: usize,
    ) -> Operand {
        let tables = self.tables();
        let outcome = &tables.nodes[left].ty;
        let meaning = meaning_of(tables, outcome);
        let Some(field) =
            tables.meanings[meaning].constructors[side].fields.first()
        else {
            return self.lower(stage);
        };
        let payload_ty = instantiate(
            &field.ty,
            &tables.meanings[meaning].parameters,
            outcome,
        );
        let payload = self.local(payload_ty, None);
        self.assign(payload, Rvalue::Field(value, 0), stage);
        let supplied = matches!(
            &tables.nodes[stage].kind,
            ir::Kind::Call { supplies, .. }
                if supplies.iter().any(|supply| supply.value == left)
        );
        if supplied {
            self.substituted.insert(left, Operand::Local(payload));
            let result = self.lower(stage);
            self.substituted.remove(&left);
            return result;
        }
        let function = self.lower(stage);
        let ir::Type::Function { result, .. } = &tables.nodes[stage].ty else {
            unreachable!("a stage takes the payload")
        };
        let dest = self.local((**result).clone(), None);
        let target = self.open(stage);
        self.terminate(
            Terminator::Call {
                callee: Callee::Value(function),
                args: vec![Operand::Local(payload)],
                meanings: Vec::new(),
                dest,
                target,
            },
            stage,
        );
        self.switch_to(target);
        Operand::Local(dest)
    }

    /// A stage's result into the outcome `ty` by the recorded rebuild.
    fn rebuild(
        &mut self,
        rebuild: ir::Rebuild,
        side: usize,
        ty: &ir::Type,
        result: Operand,
        dest: usize,
        node: usize,
    ) {
        let rvalue = match rebuild {
            ir::Rebuild::Direct => Rvalue::Use(result),
            ir::Rebuild::Wrap => Rvalue::Wrap {
                meaning: meaning_of(self.tables(), ty),
                constructor: side,
                value: result,
            },
            ir::Rebuild::Discard => Rvalue::Construct {
                meaning: meaning_of(self.tables(), ty),
                constructor: sides(self.tables(), ty).1,
                fields: Vec::new(),
            },
        };
        self.assign(dest, rvalue, node);
    }

    /// A pattern binding on an outcome: the hit binds the payload and
    /// runs the rest of the body; a miss ends the body with the value.
    fn lift(
        &mut self,
        node: usize,
        pattern: &'a ir::Pattern,
        value: usize,
        body: usize,
        rebuild: ir::Rebuild,
        dest: Option<usize>,
    ) -> Operand {
        let ty = &self.tables().nodes[node].ty;
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let outcome = self.lower(value);
        let (constructor, payload) = match &pattern.kind {
            ir::PatternKind::Side { good, payload } => {
                let (good_side, bad_side) =
                    sides(self.tables(), &self.tables().nodes[value].ty);
                (if *good { good_side } else { bad_side }, payload.as_deref())
            }
            ir::PatternKind::Constructor {
                constructor, parts, ..
            } => match parts.as_slice() {
                [] => (*constructor, None),
                [part] => (*constructor, Some(part)),
                _ => todo!("08.1 phase 2: a lift pattern with several parts"),
            },
            other => todo!("08.1 phase 2: a lift pattern {other:?}"),
        };
        let hit = self.open(node);
        let miss = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value: outcome,
                cases: vec![(constructor, hit)],
                otherwise: Some(miss),
            },
            node,
        );
        self.switch_to(miss);
        self.count();
        self.assign(dest, Rvalue::Use(outcome), node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(hit);
        match payload.map(|payload| &payload.kind) {
            None | Some(ir::PatternKind::Any) => (),
            Some(ir::PatternKind::Slot(slot))
                if self.tables().slots[*slot].definition.is_none() =>
            {
                let local = self.slot_local(*slot, node);
                self.assign(local, Rvalue::Field(outcome, 0), node);
            }
            Some(_) => {
                let payload = payload.expect("a payload pattern");
                let tables = self.tables();
                let outcome_ty = &tables.nodes[value].ty;
                let meaning = meaning_of(tables, outcome_ty);
                let field = &tables.meanings[meaning].constructors[constructor]
                    .fields[0];
                let payload_ty = instantiate(
                    &field.ty,
                    &tables.meanings[meaning].parameters,
                    outcome_ty,
                );
                let local = self.local(payload_ty, None);
                self.assign(local, Rvalue::Field(outcome, 0), node);
                self.bind_pattern(payload, Operand::Local(local), node);
            }
        }
        let result = self.lower(body);
        self.count();
        self.rebuild(rebuild, constructor, ty, result, dest, node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(join);
        Operand::Local(dest)
    }

    /// `||`: a true or good left is kept (its payload in payload mode);
    /// otherwise the right runs. A skipped right is an edge not taken.
    fn fallback(
        &mut self,
        node: usize,
        mode: ir::Mode,
        left: usize,
        right: usize,
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let value = self.lower(left);
        let (keep, take) = match mode {
            ir::Mode::Bool => bool_cases(tables, &tables.nodes[left].ty),
            ir::Mode::Family | ir::Mode::Payload => {
                sides(tables, &tables.nodes[left].ty)
            }
        };
        let kept = self.open(node);
        let taken = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value,
                cases: vec![(keep, kept), (take, taken)],
                otherwise: None,
            },
            node,
        );
        self.switch_to(kept);
        self.count();
        let rvalue = match mode {
            ir::Mode::Payload => Rvalue::Field(value, 0),
            ir::Mode::Bool | ir::Mode::Family => Rvalue::Use(value),
        };
        self.assign(dest, rvalue, node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(taken);
        self.lower_to(right, Some(dest));
        self.count();
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(join);
        Operand::Local(dest)
    }

    /// `&&`: a false or bad left is kept; otherwise the right runs and is
    /// rebuilt into the outcome.
    fn sequencing(
        &mut self,
        node: usize,
        mode: ir::Mode,
        rebuild: Option<ir::Rebuild>,
        left: usize,
        right: usize,
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let value = self.lower(left);
        let (take, keep) = match mode {
            ir::Mode::Bool => bool_cases(tables, &tables.nodes[left].ty),
            ir::Mode::Family | ir::Mode::Payload => {
                sides(tables, &tables.nodes[left].ty)
            }
        };
        let taken = self.open(node);
        let kept = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value,
                cases: vec![(take, taken), (keep, kept)],
                otherwise: None,
            },
            node,
        );
        self.switch_to(kept);
        self.count();
        self.assign(dest, Rvalue::Use(value), node);
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(taken);
        match rebuild {
            None => {
                self.lower_to(right, Some(dest));
                self.count();
            }
            Some(rebuild) => {
                let result = self.lower(right);
                self.count();
                self.rebuild(rebuild, take, ty, result, dest, node);
            }
        }
        self.terminate(Terminator::Goto(join), node);
        self.switch_to(join);
        Operand::Local(dest)
    }

    /// A comparison: the `Ordering` switched to `True` on an accepted
    /// constructor, else `False`.
    fn ordering(
        &mut self,
        node: usize,
        value: usize,
        accepted: &[usize],
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let bool = meaning_of(tables, ty);
        let (yes, no) = bool_cases(tables, ty);
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let ordering = self.lower(value);
        let accepted_block = self.open(node);
        let refused = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value: ordering,
                cases: accepted
                    .iter()
                    .map(|constructor| (*constructor, accepted_block))
                    .collect(),
                otherwise: Some(refused),
            },
            node,
        );
        for (block, constructor) in [(accepted_block, yes), (refused, no)] {
            self.switch_to(block);
            self.count();
            let rvalue = Rvalue::Construct {
                meaning: bool,
                constructor,
                fields: Vec::new(),
            };
            self.assign(dest, rvalue, node);
            self.terminate(Terminator::Goto(join), node);
        }
        self.switch_to(join);
        Operand::Local(dest)
    }
}
