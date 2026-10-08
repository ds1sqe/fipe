//! Moving the run between frames: a returned value delivered to its
//! caller, a native frame resumed, started or answered.
use super::{
    calls::Invoke,
    frames::{BodyFrame, Frame, NativeFrame},
    natives::{Ctx, Step},
    pending::Pending,
    Fault, Flow, Machine, Return,
};
use crate::{mir, value::Value};
use alloc::vec::Vec;

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
                let frame = self.top();
                frame.locals[dest] = value;
                frame.block = block;
                frame.statement = statement;
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
        }
    }

    /// Move the top native frame: start it, or give it the value it
    /// waited for; push what it calls, or pop it with its result.
    pub(super) fn resume(
        &mut self,
        program: &mir::Program,
        returned: Option<Value>,
    ) -> Result<Flow, Fault> {
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
                result: None,
            };
            (frame.node, frame.pending.step(&ctx, returned))
        };
        match step {
            Step::Call(invoke) => {
                self.invoke(program, invoke, Return::Resume, node)
            }
            Step::Start(_) => unreachable!("a frame starts once"),
            Step::Done(value) => {
                let Some(Frame::Native(frame)) = self.frames.pop() else {
                    unreachable!()
                };
                self.deliver(program, value, frame.back)
            }
            Step::Fault(reason) => Err(Fault {
                span: Self::span(program, node),
                reason,
            }),
        }
    }

    /// Start a native frame whose first move is now.
    pub(super) fn start(
        &mut self,
        program: &mir::Program,
        pending: Pending,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        self.frames.push(Frame::Native(NativeFrame {
            node,
            pending,
            back,
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
            Invoke::Value(value) => self.deliver(program, value, back),
        }
    }
}
