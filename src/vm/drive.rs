//! Moving the run between frames: a returned value delivered to its
//! caller, a native frame resumed, started or answered.
use super::{
    calls::Invoke,
    frames::{BodyFrame, Frame, NativeFrame},
    natives::{Ctx, Step},
    pending::Pending,
    Fault, Flow, Machine, Return,
};
use crate::{ir, mir, value::Value};
use alloc::vec::Vec;

/// Where a native frame answers, its call site, and its call's result
/// type.
pub(super) struct Start {
    back: Return,
    node: usize,
    result: Option<ir::Type>,
}

impl Start {
    pub(super) const fn new(
        back: Return,
        node: usize,
        result: Option<ir::Type>,
    ) -> Self {
        Self { back, node, result }
    }
}

impl Machine {
    /// Hand a finished frame's value to where its caller wanted it.
    pub(super) fn deliver(
        &mut self,
        program: &mir::Program,
        value: Value,
        back: Return,
    ) -> Result<Flow, Fault> {
        match back {
            Return::Finish => Ok(Flow::Done(value)),
            Return::Into {
                dest,
                block,
                statement,
            } => {
                if self.stepping {
                    self.landed(program, block, statement, &value);
                }
                let frame = self.top();
                frame.locals[dest] = value;
                frame.block = block;
                frame.statement = statement;
                Ok(Flow::Continue)
            }
            Return::Arguments { body, arity, node } => {
                let invalid = || {
                    Self::fault(
                        program,
                        node,
                        crate::value::Reason::Host(
                            "invalid guarded arguments".into(),
                        ),
                    )
                };
                let Value::Tuple(values) = value else {
                    return Err(invalid());
                };
                let values =
                    alloc::rc::Rc::try_unwrap(values).map_err(|_| invalid())?;
                let end = arity.checked_add(1).ok_or_else(invalid)?;
                let shape = program.bodies.get(body).ok_or_else(invalid)?;
                if shape.arity != arity || values.len() != arity {
                    return Err(invalid());
                }
                let Some(Frame::Body(frame)) = self.frames.last_mut() else {
                    return Err(invalid());
                };
                if frame.body != body || end > frame.locals.len() {
                    return Err(invalid());
                }
                for (spot, value) in values.into_iter().enumerate() {
                    frame.locals[spot + 1] = value;
                }
                Ok(Flow::Continue)
            }
            Return::Branch { pass, fail } => {
                let passed = self.prelude().is_true(&value);
                let frame = self.top();
                frame.block = if passed { pass } else { fail };
                frame.statement = 0;
                Ok(Flow::Continue)
            }
            Return::Resume => self.resume(program, Some(value)),
            Return::Continue => Ok(Flow::Continue),
            Return::Store(definition) => {
                self.globals[definition] = Some(value);
                Ok(Flow::Continue)
            }
        }
    }

    /// Move the top native frame: start it, or give it the value it
    /// waited for; push what it calls, or pop it with its result.
    pub(super) fn resume(
        &mut self,
        program: &mir::Program,
        returned: Option<Value>,
    ) -> Result<Flow, Fault> {
        let work = self.resource_work();
        let (node, step) = {
            let natives = self.natives();
            let prelude = self.prelude();
            let tables = program.tables.borrow();
            let Some(Frame::Native(frame)) = self.frames.last_mut() else {
                unreachable!("a native frame is on top")
            };
            let ctx = Ctx {
                program,
                tables: &tables,
                prelude: &prelude,
                natives: &natives,
                globals: &self.globals,
                result: frame.result.as_ref(),
                work: &work,
            };
            let step = frame.pending.step(&ctx, returned);
            (frame.node, step)
        };
        if let Err(reason) = self.pay(work.spent()) {
            if let Step::Call(super::calls::Invoke::Body { meanings, .. }) =
                step
            {
                drop(crate::value::resource::Forest(meanings));
            }
            return Err(Self::fault(program, node, reason));
        }
        match step {
            Step::Call(invoke) => {
                self.invoke(program, invoke, Return::Resume, node)
            }
            Step::Start(_) => unreachable!("a frame starts once"),
            Step::AdmitArgument { value, guard } => self
                .admit_argument(program, guard, value, node)
                .map_err(|reason| Self::fault(program, node, reason)),
            Step::Done(value) => {
                let Some(Frame::Native(frame)) = self.frames.pop() else {
                    unreachable!()
                };
                self.deliver(program, value, frame.back)
            }
            Step::Fault(reason) => Err(Self::fault(program, node, reason)),
        }
    }

    /// Start a native frame whose first move is now.
    pub(super) fn start(
        &mut self,
        program: &mir::Program,
        pending: Pending,
        start: Start,
    ) -> Result<Flow, Fault> {
        self.frames.push(Frame::Native(NativeFrame {
            node: start.node,
            pending,
            back: start.back,
            result: start.result,
        }));
        self.resume(program, None)
    }

    /// Push what a native asked for.
    pub(super) fn invoke(
        &mut self,
        program: &mir::Program,
        invoke: Invoke,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        match invoke {
            Invoke::Function(function, args) => {
                self.call_value(program, &function, args, back, node)
            }
            Invoke::Body {
                body,
                args,
                meanings,
            } => {
                let frame = BodyFrame::new(
                    program,
                    body,
                    args,
                    Vec::new(),
                    meanings,
                    back,
                );
                self.frames.push(Frame::Body(frame));
                Ok(Flow::Continue)
            }
            Invoke::Declared {
                declaration,
                args,
                mut result,
            } => self.native(
                program,
                declaration,
                args,
                Some(result.0.pop().expect("supplied result")),
                back,
                node,
            ),
            Invoke::Value(value) => self.deliver(program, value, back),
            Invoke::Native(pending) => {
                self.start(program, *pending, Start::new(back, node, None))
            }
        }
    }
}
