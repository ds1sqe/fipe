//! One step: a statement or the terminator of the top body frame, or
//! one move of the top native frame.
use super::{
    frames::{BodyFrame, Frame, NativeFrame},
    pending::Pending,
    Fault, Flow, Machine, Return,
};
use crate::{
    ir, mir,
    mir::{Operand, Statement, Terminator},
    value::{structural, Reason, Value},
};
use alloc::{rc::Rc, vec::Vec};

impl Machine {
    /// One step of the top frame.
    pub(super) fn step(
        &mut self,
        program: &mir::Program,
    ) -> Result<Flow, Fault> {
        match self.frames.last() {
            Some(Frame::Body(_)) => self.step_body(program),
            Some(Frame::Native(_)) => self.resume(program, None),
            None => unreachable!("a run has a frame"),
        }
    }

    pub(super) fn top(&mut self) -> &mut BodyFrame {
        match self.frames.last_mut() {
            Some(Frame::Body(frame)) => frame,
            _ => unreachable!("a body frame is on top"),
        }
    }

    fn step_body(&mut self, program: &mir::Program) -> Result<Flow, Fault> {
        let frame = self.top();
        let body = &program.bodies[frame.body];
        let block = &body.blocks[frame.block];
        let index = frame.statement;
        // A global no line stored runs its body first; the step then
        // runs again with the value in place.
        let reads = block.statements.get(index).map_or_else(
            || super::globals::in_terminator(&block.terminator),
            super::globals::in_statement,
        );
        if let Some(definition) = reads
            .into_iter()
            .find(|definition| self.globals[*definition].is_none())
        {
            // A global only its own line stores: a session withdraws a
            // line whose run ended before storing it.
            let body = program.globals[definition].expect(
                "a global with no value has a body, or its line was withdrawn",
            );
            let frame = BodyFrame::new(
                program,
                body,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Return::Store(definition),
            );
            self.frames.push(Frame::Body(frame));
            return Ok(Flow::Continue);
        }
        let frame = self.top();
        let body = &program.bodies[frame.body];
        let block = &body.blocks[frame.block];
        if index < block.statements.len() {
            let node = block.nodes[index];
            frame.statement += 1;
            if let Statement::Charge(nodes) = &block.statements[index] {
                return self.pay_nodes(program, nodes).map(|()| Flow::Continue);
            }
            return self
                .statement(program, &block.statements[index], node)
                .map_err(|reason| Self::fault(program, node, reason));
        }
        let node = *block.nodes.last().expect("a terminator's node");
        self.terminator(program, &block.terminator, node)
    }

    /// A statement of the top body frame; `Ok(Continue)` unless it
    /// started a native frame.
    fn statement(
        &mut self,
        program: &mir::Program,
        statement: &Statement,
        node: usize,
    ) -> Result<Flow, Reason> {
        match statement {
            Statement::Charge(_) => unreachable!("paid by the step"),
            Statement::Assign(dest, rvalue) => {
                return self.assign(program, *dest, rvalue, node);
            }
            Statement::Store { definition, value } => {
                let value = self.operand(program, *value);
                self.globals[*definition] = Some(value);
            }
            Statement::Enter {
                entry,
                value: operand,
            } => {
                let value = self.operand(program, *operand);
                // A function value is not tested; its calls are.
                if let (Value::Function(closure), Operand::Local(local)) =
                    (&value, operand)
                {
                    let guarded = {
                        let tables = program.tables.borrow();
                        let predicate = &tables.entries[*entry].predicate;
                        matches!(predicate, ir::Predicate::Function { .. })
                            .then(|| Self::guard(closure, *entry, predicate))
                    };
                    if let Some(guarded) = guarded {
                        self.top().locals[*local] = guarded;
                        return Ok(Flow::Continue);
                    }
                }
                return Ok(self.enter(program, *entry, value, node));
            }
            Statement::Push { key, value } => {
                let value = self.operand(program, *value);
                self.environment.push((*key, value));
            }
            Statement::Pop => {
                self.environment.pop();
            }
            Statement::Line(value) => {
                let value = self.operand(program, *value);
                self.lines.push((node, value));
            }
        }
        Ok(Flow::Continue)
    }

    /// Read an operand in the top body frame.
    pub(super) fn operand(
        &self,
        program: &mir::Program,
        operand: Operand,
    ) -> Value {
        let Some(Frame::Body(frame)) = self.frames.last() else {
            unreachable!("a body frame is on top")
        };
        match operand {
            Operand::Local(index) => frame.locals[index].clone(),
            Operand::Const(index) => program.constants[index].clone(),
            Operand::Capture(index) => frame.captures[index].clone(),
            Operand::Global(index) => self.globals[index]
                .clone()
                .unwrap_or_else(|| panic!("global {index} is stored first")),
            Operand::Hidden(index) => {
                Value::Meaning(Rc::new(frame.meanings[index].clone()))
            }
        }
    }

    pub(super) fn operands(
        &self,
        program: &mir::Program,
        operands: &[Operand],
    ) -> Vec<Value> {
        operands
            .iter()
            .map(|operand| self.operand(program, *operand))
            .collect()
    }

    /// The type of a local of the top frame, with the witnesses in.
    pub(super) fn local_type(
        &self,
        program: &mir::Program,
        local: usize,
    ) -> ir::Type {
        let Some(Frame::Body(frame)) = self.frames.last() else {
            unreachable!("a body frame is on top")
        };
        let ty = &program.bodies[frame.body].locals[local].ty;
        Self::substitute(program, frame, ty)
    }

    fn terminator(
        &mut self,
        program: &mir::Program,
        terminator: &Terminator,
        node: usize,
    ) -> Result<Flow, Fault> {
        match terminator {
            Terminator::Goto(block) => {
                let frame = self.top();
                frame.block = *block;
                frame.statement = 0;
                Ok(Flow::Continue)
            }
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                let key = match self.operand(program, *value) {
                    Value::Record(record) => record.constructor,
                    Value::Packed(packed) => packed.meaning,
                    Value::Int(number) => {
                        usize::try_from(number).unwrap_or(usize::MAX)
                    }
                    other => unreachable!("a switch on {other:?}"),
                };
                let block = cases
                    .iter()
                    .find(|(case, _)| *case == key)
                    .map(|(_, block)| *block)
                    .or(*otherwise);
                let Some(block) = block else {
                    return Err(Self::fault(
                        program,
                        node,
                        Reason::Unreachable,
                    ));
                };
                let frame = self.top();
                frame.block = block;
                frame.statement = 0;
                Ok(Flow::Continue)
            }
            Terminator::Test {
                predicate,
                value,
                pass,
                fail,
            } => {
                let value = self.operand(program, *value);
                let run = super::predicates::Run::new(
                    predicate.clone(),
                    value,
                    super::predicates::After::Branch,
                );
                self.frames.push(Frame::Native(NativeFrame {
                    node,
                    pending: Pending::Predicate(run),
                    back: Return::Branch {
                        pass: *pass,
                        fail: *fail,
                    },
                    result: None,
                }));
                self.resume(program, None)
            }
            Terminator::Call {
                callee,
                args,
                meanings,
                dest,
                target,
            } => self.call_terminator(
                program, callee, args, meanings, *dest, *target, node,
            ),
            Terminator::Return => {
                let Some(Frame::Body(frame)) = self.frames.pop() else {
                    unreachable!()
                };
                let value = frame.locals.into_iter().next().expect("_0");
                self.deliver(program, value, frame.back)
            }
            Terminator::Unreachable => {
                Err(Self::fault(program, node, Reason::Unreachable))
            }
        }
    }
}

impl Machine {
    /// A call terminator: the callee with its arguments and witnesses,
    /// its result into `dest`, then `target`.
    #[allow(clippy::too_many_arguments)]
    fn call_terminator(
        &mut self,
        program: &mir::Program,
        callee: &mir::Callee,
        args: &[mir::Operand],
        meanings: &[ir::Type],
        dest: usize,
        target: usize,
        node: usize,
    ) -> Result<Flow, Fault> {
        let args = self.operands(program, args);
        let (meanings, result) = {
            let Some(Frame::Body(frame)) = self.frames.last() else {
                unreachable!()
            };
            let meanings = meanings
                .iter()
                .map(|ty| Self::substitute(program, frame, ty))
                .collect();
            (meanings, self.local_type(program, dest))
        };
        let back = Return::Into {
            dest,
            block: target,
            statement: 0,
        };
        let tail = dest == 0 && {
            let Some(Frame::Body(frame)) = self.frames.last() else {
                unreachable!()
            };
            returns(&program.bodies[frame.body], target)
        };
        let depth = self.frames.len();
        let flow =
            self.call(program, callee, args, meanings, result, back, node)?;
        if tail && self.frames.len() == depth + 1 {
            self.replace_caller();
        }
        Ok(flow)
    }

    /// A call in tail position: the body frame it pushed takes its
    /// caller's place and returns where the caller would have, so a
    /// recursion as iteration keeps one frame.
    fn replace_caller(&mut self) {
        let Some(Frame::Body(_)) = self.frames.last() else {
            return;
        };
        let Some(Frame::Body(mut entered)) = self.frames.pop() else {
            unreachable!()
        };
        let Some(Frame::Body(replaced)) = self.frames.pop() else {
            unreachable!("a call's caller is a body")
        };
        entered.back = replaced.back;
        self.frames.push(Frame::Body(entered));
    }
}

/// Whether a call's result in `_0` is all that is left: from `block`,
/// empty blocks reach `Return`.
fn returns(body: &mir::Body, mut block: usize) -> bool {
    for _ in 0..body.blocks.len() {
        let next = &body.blocks[block];
        if !next.statements.is_empty() {
            return false;
        }
        match next.terminator {
            Terminator::Return => return true,
            Terminator::Goto(target) => block = target,
            _ => return false,
        }
    }
    false
}

/// A record's field, a tuple's part, a sequence's element, a packed
/// value's payload, or the bytes of text (`String { bytes }`).
pub(super) fn field(value: &Value, index: usize) -> Value {
    match value {
        Value::Record(record) => record.fields[index].clone(),
        Value::Tuple(parts) | Value::Seq(parts) => parts[index].clone(),
        Value::Packed(packed) => packed.value.clone(),
        Value::Text(_) => crate::value::text::bytes(value),
        other => unreachable!("a field of {other:?}"),
    }
}

/// Whether a value is a leaf no written member can be met under.
pub(super) const fn leaf(value: &Value) -> bool {
    !matches!(
        value,
        Value::Record(_) | Value::Seq(_) | Value::Tuple(_) | Value::Packed(_)
    )
}

/// The language's spelling of a leaf, for the natives' fast path.
pub(super) fn show_leaf(value: &Value, tables: &ir::Tables) -> Value {
    Value::text(&structural::show(value, tables))
}
