//! The expression walk: one IR node to statements and blocks.
use super::{builder::Builder, Lowering};
use crate::{
    ir,
    mir::{self, Operand, Rvalue, Statement, Terminator},
};
use alloc::vec::Vec;

impl Builder<'_, '_> {
    pub(super) fn lower(&mut self, node: usize) -> Operand {
        self.lower_to(node, None)
    }

    /// Lower a node; with `dest`, its value is left in that local.
    pub(super) fn lower_to(
        &mut self,
        node: usize,
        dest: Option<usize>,
    ) -> Operand {
        // The node was evaluated before the switch; this read is not an
        // evaluation, so it charges nothing.
        if let Some(operand) = self.substituted.get(&node).copied() {
            return self.place(operand, dest, node);
        }
        let tables = self.tables();
        let ir::Node { ty, kind, .. } = &tables.nodes[node];
        match kind {
            ir::Kind::Timing { value, .. } => {
                self.count();
                self.lower_to(*value, dest)
            }
            ir::Kind::Literal { form, reader } => {
                self.count();
                self.literal(node, form, *reader, dest)
            }
            // A member root names a definition by its slot.
            ir::Kind::Local { slot, meanings }
            | ir::Kind::Capture { slot, meanings }
                if !self.slots.contains_key(slot)
                    && !self.captured.contains(slot)
                    && tables.slots[*slot].definition.is_some() =>
            {
                let index = tables.slots[*slot].definition.expect("checked");
                self.definition(index, meanings, ty, node, dest)
            }
            ir::Kind::Local { slot, .. } | ir::Kind::Capture { slot, .. } => {
                self.count();
                let operand = self.operand_of_slot(*slot);
                self.place(operand, dest, node)
            }
            ir::Kind::Definition { index, meanings } => {
                self.definition(*index, meanings, ty, node, dest)
            }
            ir::Kind::Declared(_) | ir::Kind::Constructor { .. } => {
                self.count();
                if let Some(value) = self.nullary(node, dest) {
                    return value;
                }
                let closure = self.thunk_of(node);
                self.materialize(closure, dest, ty, node)
            }
            ir::Kind::Derived { .. } | ir::Kind::Dispatch { .. } => {
                todo!("08.1 phase 2: a member or derived operation as a value")
            }
            ir::Kind::Construct { .. }
            | ir::Kind::Field { .. }
            | ir::Kind::Tuple(_)
            | ir::Kind::Array(_)
            | ir::Kind::Repeat { .. } => {
                self.count();
                let rvalue = self.container(kind);
                self.materialize(rvalue, dest, ty, node)
            }
            ir::Kind::Call {
                callee,
                slots,
                supplies,
                defaults,
                open,
            } => {
                let site = super::calls::Site {
                    node,
                    callee: *callee,
                    slots,
                    supplies,
                    defaults,
                    open,
                };
                self.call(&site, dest)
            }
            ir::Kind::Function {
                hidden,
                captures,
                arms,
                ..
            } => {
                self.count();
                let closure = self.function(node, hidden, captures, arms);
                self.materialize(closure, dest, ty, node)
            }
            _ => self.control(node, kind, dest),
        }
    }

    /// Control: branches, sequences, bindings, environments, pipes.
    fn control(
        &mut self,
        node: usize,
        kind: &ir::Kind,
        dest: Option<usize>,
    ) -> Operand {
        match kind {
            ir::Kind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.count();
                self.conditional(node, *condition, *then_body, *else_body, dest)
            }
            ir::Kind::Block(parts) => {
                self.count();
                self.sequence(node, parts, dest)
            }
            ir::Kind::Bind { pattern, value } => {
                self.count();
                self.bind(node, pattern, *value);
                let unit = self.lowering.constant(crate::value::Value::Unit);
                self.place(Operand::Const(unit), dest, node)
            }
            ir::Kind::With { key, value, body } => {
                self.count();
                let value = self.lower(*value);
                self.emit(Statement::Push { key: *key, value }, node);
                let result = self.lower_to(*body, dest);
                self.emit(Statement::Pop, node);
                result
            }
            ir::Kind::Pipe { stage, .. } => {
                self.count();
                self.lower_to(*stage, dest)
            }
            ir::Kind::Ordering { .. }
            | ir::Kind::Lift { .. }
            | ir::Kind::Marked { .. }
            | ir::Kind::Or { .. }
            | ir::Kind::And { .. }
            | ir::Kind::Cast { .. }
            | ir::Kind::Pack { .. }
            | ir::Kind::Enter { .. }
            | ir::Kind::Convert { .. } => self.outcome(node, kind, dest),
            other => unreachable!("a value of {other:?}"),
        }
    }

    /// A value built from parts: its parts in order, then the rvalue.
    fn container(&mut self, kind: &ir::Kind) -> Rvalue {
        match kind {
            ir::Kind::Construct {
                meaning,
                constructor,
                fields,
            } => Rvalue::Construct {
                meaning: *meaning,
                constructor: *constructor,
                fields: fields.iter().map(|field| self.lower(*field)).collect(),
            },
            ir::Kind::Field { value, index } => {
                Rvalue::Field(self.lower(*value), *index)
            }
            ir::Kind::Tuple(parts) => Rvalue::Tuple(
                parts.iter().map(|part| self.lower(*part)).collect(),
            ),
            ir::Kind::Array(parts) => Rvalue::Array(
                parts.iter().map(|part| self.lower(*part)).collect(),
            ),
            ir::Kind::Repeat { value, length } => {
                let length = match length {
                    ir::Type::Number(number) => mir::Length::Fixed(
                        number.to_u64().expect("checked array length"),
                    ),
                    ir::Type::Parameter(parameter) => {
                        mir::Length::Hidden(self.hidden_position(*parameter))
                    }
                    other => unreachable!("array length {other:?}"),
                };
                Rvalue::Repeat {
                    value: self.lower(*value),
                    length,
                }
            }
            other => unreachable!("a container of {other:?}"),
        }
    }

    /// A plain lambda: one arm of slot patterns and no guard.
    fn function(
        &mut self,
        node: usize,
        hidden: &[usize],
        captures: &[usize],
        arms: &[ir::Arm],
    ) -> Rvalue {
        assert!(
            hidden.is_empty(),
            "a definition's function is reached through its definition"
        );
        let body = build_function(
            self.lowering,
            node,
            &self.hidden,
            captures,
            arms,
            None,
        );
        Rvalue::Closure {
            body,
            captures: captures
                .iter()
                .map(|slot| self.operand_of_slot(*slot))
                .collect(),
            meanings: self.inherited(),
        }
    }

    /// A definition as a value: a closure of its body with the given
    /// witnesses, or its global.
    fn definition(
        &mut self,
        index: usize,
        meanings: &[ir::Type],
        ty: &ir::Type,
        node: usize,
        dest: Option<usize>,
    ) -> Operand {
        self.count();
        match self.lowering.function_body(index) {
            Some(body) => {
                let closure = Rvalue::Closure {
                    body,
                    captures: Vec::new(),
                    meanings: meanings.to_vec(),
                };
                self.materialize(closure, dest, ty, node)
            }
            None => self.place(Operand::Global(index), dest, node),
        }
    }

    fn conditional(
        &mut self,
        node: usize,
        condition: usize,
        then_body: usize,
        else_body: usize,
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let ir::Type::Meaning { index: bool, .. } = &tables.nodes[condition].ty
        else {
            unreachable!("a condition is a Bool")
        };
        let constructor = |label: &str| {
            tables.meanings[*bool]
                .constructors
                .iter()
                .position(|constructor| constructor.label == label)
                .expect("Bool's constructors")
        };
        let value = self.lower(condition);
        let dest = dest
            .unwrap_or_else(|| self.local(tables.nodes[node].ty.clone(), None));
        let then_block = self.open(node);
        let else_block = self.open(node);
        let join = self.open(node);
        self.terminate(
            Terminator::Switch {
                value,
                cases: alloc::vec![
                    (constructor("True"), then_block),
                    (constructor("False"), else_block),
                ],
                otherwise: None,
            },
            node,
        );
        for (block, body) in [(then_block, then_body), (else_block, else_body)]
        {
            self.switch_to(block);
            self.lower_to(body, Some(dest));
            self.terminate(Terminator::Goto(join), node);
        }
        self.switch_to(join);
        Operand::Local(dest)
    }

    /// A block's parts in order; its value is the last part's.
    pub(super) fn sequence(
        &mut self,
        node: usize,
        parts: &[usize],
        dest: Option<usize>,
    ) -> Operand {
        let Some((last, earlier)) = parts.split_last() else {
            let unit = self.lowering.constant(crate::value::Value::Unit);
            return self.place(Operand::Const(unit), dest, node);
        };
        for part in earlier {
            self.lower(*part);
        }
        self.lower_to(*last, dest)
    }

    fn bind(&mut self, node: usize, pattern: &ir::Pattern, value: usize) {
        match &pattern.kind {
            ir::PatternKind::Any => {
                self.lower(value);
            }
            ir::PatternKind::Slot(slot) => {
                if let Some(definition) = self.tables().slots[*slot].definition
                {
                    // A function definition's body is lowered on reference;
                    // a value definition's is computed here, once.
                    if self.lowering.function_body(definition).is_none() {
                        let value = self.lower(value);
                        self.emit(Statement::Store { definition, value }, node);
                    }
                } else {
                    let local = self.slot_local(*slot, node);
                    self.lower_to(value, Some(local));
                }
            }
            _ => todo!("08.1 phase 2: a binding pattern with parts"),
        }
    }
}

/// The spot types of a function type, in order, then its result.
fn signature(ty: &ir::Type, arity: usize) -> (Vec<ir::Type>, ir::Type) {
    let mut spots = Vec::with_capacity(arity);
    let mut ty = ty;
    while spots.len() < arity {
        let ir::Type::Function { parameter, result } = ty else {
            unreachable!("a function of {arity} spots")
        };
        spots.push((**parameter).clone());
        ty = result;
    }
    (spots, ty.clone())
}

/// Build a function body from its arms; into `reserved` for a definition.
pub(super) fn build_function(
    lowering: &mut Lowering<'_>,
    node: usize,
    hidden: &[usize],
    captures: &[usize],
    arms: &[ir::Arm],
    reserved: Option<usize>,
) -> usize {
    let [arm] = arms else {
        todo!("08.1 phase 2: a function with several arms")
    };
    assert!(arm.guard.is_none(), "08.1 phase 2: a guarded arm");
    let (spots, result) =
        signature(&lowering.tables.nodes[node].ty, arm.patterns.len());
    let mut builder = Builder::new(
        lowering,
        node,
        result,
        hidden.to_vec(),
        captures.to_vec(),
    );
    for (pattern, ty) in arm.patterns.iter().zip(spots) {
        let slot = match &pattern.kind {
            ir::PatternKind::Slot(slot) => Some(*slot),
            ir::PatternKind::Any => None,
            _ => todo!("08.1 phase 2: a spot pattern with parts"),
        };
        builder.spot(slot, ty, node);
    }
    builder.lower_to(arm.body, Some(0));
    builder.terminate(Terminator::Return, node);
    builder.finish(reserved)
}
