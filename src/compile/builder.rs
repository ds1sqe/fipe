//! A body under construction: locals, blocks, statements and charges.
use super::Lowering;
use crate::{
    ir,
    mir::{self, Operand, Rvalue, Statement, Terminator},
};
use alloc::{collections::BTreeMap, vec::Vec};

/// A block under construction.
pub(super) struct Open {
    statements: Vec<Statement>,
    nodes: Vec<usize>,
    terminator: Option<(Terminator, usize)>,
    /// IR nodes lowered into this block, the charge.
    count: usize,
    /// The node that opened the block, the charge's node.
    opener: usize,
}

/// A body under construction.
pub(super) struct Builder<'l, 'a> {
    pub(super) lowering: &'l mut Lowering<'a>,
    node: usize,
    arity: usize,
    /// The hidden meaning parameters this body's frame carries.
    pub(super) hidden: Vec<usize>,
    /// The IR slots a closure of this body captures, by position.
    pub(super) captured: Vec<usize>,
    /// How many captures a closure of this body supplies; a thunk's are
    /// values, not slots, so it sets the count itself.
    pub(super) captures: usize,
    /// IR slot to local.
    pub(super) slots: BTreeMap<usize, usize>,
    /// Nodes read as another operand while a marked stage is lowered:
    /// the stage's supply of the left reads the payload.
    pub(super) substituted: BTreeMap<usize, Operand>,
    pub(super) locals: Vec<mir::Local>,
    pub(super) blocks: Vec<Open>,
    pub(super) current: usize,
}

impl<'l, 'a> Builder<'l, 'a> {
    /// A body for `node` whose `_0` has type `result`.
    pub(super) fn new(
        lowering: &'l mut Lowering<'a>,
        node: usize,
        result: ir::Type,
        hidden: Vec<usize>,
        captured: Vec<usize>,
    ) -> Self {
        Self {
            lowering,
            node,
            arity: 0,
            hidden,
            captures: captured.len(),
            captured,
            slots: BTreeMap::new(),
            substituted: BTreeMap::new(),
            locals: alloc::vec![mir::Local {
                ty: result,
                node: None
            }],
            blocks: alloc::vec![Open {
                statements: Vec::new(),
                nodes: Vec::new(),
                terminator: None,
                count: 0,
                opener: node,
            }],
            current: 0,
        }
    }

    pub(super) fn tables(&self) -> &'a ir::Tables {
        self.lowering.tables
    }

    /// The next spot, declared before any other local.
    pub(super) fn spot(
        &mut self,
        slot: Option<usize>,
        ty: ir::Type,
        node: usize,
    ) -> usize {
        debug_assert_eq!(self.locals.len(), self.arity + 1);
        let local = self.local(ty, Some(node));
        self.arity += 1;
        if let Some(slot) = slot {
            self.slots.insert(slot, local);
        }
        local
    }

    pub(super) fn local(&mut self, ty: ir::Type, node: Option<usize>) -> usize {
        self.locals.push(mir::Local { ty, node });
        self.locals.len() - 1
    }

    /// The local of an IR slot, made on first use.
    pub(super) fn slot_local(&mut self, slot: usize, node: usize) -> usize {
        if let Some(local) = self.slots.get(&slot) {
            return *local;
        }
        let ty = self.tables().slots[slot].ty.clone();
        let local = self.local(ty, Some(node));
        self.slots.insert(slot, local);
        local
    }

    /// Where a slot's value is read from in this body.
    pub(super) fn operand_of_slot(&self, slot: usize) -> Operand {
        if let Some(local) = self.slots.get(&slot) {
            return Operand::Local(*local);
        }
        self.captured
            .iter()
            .position(|captured| *captured == slot)
            .map_or_else(
                || panic!("slot {slot} is neither local nor captured"),
                Operand::Capture,
            )
    }

    /// The position of a meaning parameter among this frame's witnesses.
    pub(super) fn hidden_position(&self, parameter: usize) -> usize {
        self.hidden
            .iter()
            .position(|hidden| *hidden == parameter)
            .unwrap_or_else(|| {
                panic!("meaning parameter {parameter} has no witness")
            })
    }

    /// The meanings a closure made here hands its body: the frame's own.
    pub(super) fn inherited(&self) -> Vec<ir::Type> {
        self.hidden
            .iter()
            .copied()
            .map(ir::Type::Parameter)
            .collect()
    }

    /// One more IR node lowered into the current block.
    pub(super) fn count(&mut self) {
        self.blocks[self.current].count += 1;
    }

    pub(super) fn emit(&mut self, statement: Statement, node: usize) {
        let block = &mut self.blocks[self.current];
        block.statements.push(statement);
        block.nodes.push(node);
    }

    pub(super) fn assign(&mut self, dest: usize, rvalue: Rvalue, node: usize) {
        self.emit(Statement::Assign(dest, rvalue), node);
    }

    /// A new block, not yet current.
    pub(super) fn open(&mut self, opener: usize) -> usize {
        self.blocks.push(Open {
            statements: Vec::new(),
            nodes: Vec::new(),
            terminator: None,
            count: 0,
            opener,
        });
        self.blocks.len() - 1
    }

    pub(super) const fn switch_to(&mut self, block: usize) {
        self.current = block;
    }

    pub(super) fn terminate(&mut self, terminator: Terminator, node: usize) {
        let block = &mut self.blocks[self.current];
        assert!(block.terminator.is_none(), "block terminated twice");
        block.terminator = Some((terminator, node));
    }

    /// Place an rvalue in `dest` or a fresh temporary.
    pub(super) fn materialize(
        &mut self,
        rvalue: Rvalue,
        dest: Option<usize>,
        ty: &ir::Type,
        node: usize,
    ) -> Operand {
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        self.assign(dest, rvalue, node);
        Operand::Local(dest)
    }

    /// An operand already computed, copied into `dest` when one is asked.
    pub(super) fn place(
        &mut self,
        operand: Operand,
        dest: Option<usize>,
        node: usize,
    ) -> Operand {
        match dest {
            Some(dest) if operand != Operand::Local(dest) => {
                self.assign(dest, Rvalue::Use(operand), node);
                Operand::Local(dest)
            }
            _ => operand,
        }
    }

    /// Close the body: `Return` on the current block, charges at each
    /// block head; pushed as a new body or into a reserved index.
    pub(super) fn finish(self, reserved: Option<usize>) -> usize {
        let Self {
            lowering,
            node,
            arity,
            hidden,
            captures,
            locals,
            blocks,
            ..
        } = self;
        let blocks = blocks
            .into_iter()
            .map(|open| {
                let (terminator, last) =
                    open.terminator.expect("every block is terminated");
                let mut statements =
                    Vec::with_capacity(open.statements.len() + 1);
                let mut nodes = Vec::with_capacity(open.nodes.len() + 2);
                if open.count > 0 {
                    statements.push(Statement::Charge(open.count));
                    nodes.push(open.opener);
                }
                statements.extend(open.statements);
                nodes.extend(open.nodes);
                nodes.push(last);
                mir::Block {
                    statements,
                    terminator,
                    nodes,
                }
            })
            .collect();
        let body = mir::Body {
            node,
            arity,
            captures,
            hidden,
            locals,
            blocks,
        };
        if let Some(index) = reserved {
            lowering.bodies[index] = body;
            index
        } else {
            lowering.bodies.push(body);
            lowering.bodies.len() - 1
        }
    }
}
