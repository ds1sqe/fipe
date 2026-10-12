//! Calls: the callee kinds, eval/apply of a function value, the casts.
use super::{
    drive::Start,
    frames::{BodyFrame, Frame},
    natives::{Ctx, Step},
    pending::Pending,
    walk::Walk,
    Fault, Flow, Machine, Return,
};
use crate::{
    compile::dispatch::{self, Selected},
    ir, mir,
    mir::Callee,
    value::{Closure, Reason, Value},
};
use alloc::{format, rc::Rc, vec::Vec};

/// What a native or a dispatch asks the machine to run.
#[derive(Debug)]
pub(super) enum Invoke {
    /// A function value applied to arguments, by eval/apply.
    Function(Value, Vec<Value>),
    /// A body entered with its spots and witnesses.
    Body {
        body: usize,
        args: Vec<Value>,
        meanings: Vec<ir::Type>,
    },
    /// A value already in hand.
    Value(Value),
    /// A native in its own frame, answering to the one that asked.
    Native(alloc::boxed::Box<super::pending::Pending>),
}

impl Machine {
    /// A call terminator's callee with its arguments in hand.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call(
        &mut self,
        program: &mir::Program,
        callee: &Callee,
        args: Vec<Value>,
        meanings: Vec<ir::Type>,
        result: ir::Type,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let fault = |reason| Fault {
            span: Self::span(program, node),
            reason,
        };
        match callee {
            Callee::Body(body) => {
                let frame = BodyFrame::new(
                    program,
                    *body,
                    args,
                    Vec::new(),
                    meanings,
                    back,
                );
                self.frames.push(Frame::Body(frame));
                Ok(Flow::Continue)
            }
            Callee::Native(op) => {
                self.native(program, *op, args, Some(result), back, node)
            }
            Callee::Host(declaration) => {
                Ok(self.host(*declaration, args, back, node))
            }
            Callee::Derived { kind, ty } => {
                let ty = {
                    let Some(Frame::Body(frame)) = self.frames.last() else {
                        unreachable!()
                    };
                    Self::substitute(program, frame, ty)
                };
                let walk = Walk::derived(*kind, args, ty);
                let start = Start::new(back, node, Some(result));
                self.start(program, Pending::Walk(walk), start)
            }
            Callee::Dispatch { member, witness } => {
                let (receiver, selected) = {
                    let tables = program.tables.borrow();
                    let (ty, receiver) = match witness {
                        mir::Witness::Value(operand) => {
                            let value = self.operand(program, *operand);
                            let ty = match operand {
                                mir::Operand::Local(local) => {
                                    Some(self.local_type(program, *local))
                                }
                                _ => None,
                            };
                            (ty, Some(value))
                        }
                        mir::Witness::Meaning(ty) => {
                            let Some(Frame::Body(frame)) = self.frames.last()
                            else {
                                unreachable!()
                            };
                            (Some(Self::substitute(program, frame, ty)), None)
                        }
                    };
                    let selected = super::dispatch::select_for(
                        &tables,
                        &self.prelude(),
                        *member,
                        ty.as_ref(),
                        receiver.as_ref(),
                    );
                    (receiver, selected)
                };
                let Some(selected) = selected else {
                    let label =
                        program.tables.borrow().dispatch[*member].label.clone();
                    return Err(fault(Reason::Message(format!(
                        "no implementation of {label} for the value"
                    ))));
                };
                let _ = receiver;
                self.row(program, *member, &selected, args, result, back, node)
            }
            Callee::Value(operand) => {
                let function = self.operand(program, *operand);
                self.call_value(program, &function, args, back, node)
            }
        }
    }

    /// A native: its value at once, or its frame started.
    pub(super) fn native(
        &mut self,
        program: &mir::Program,
        op: usize,
        args: Vec<Value>,
        result: Option<ir::Type>,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let natives = self.natives();
        // A declared row of a host's namespace is a host call.
        let Some(native) = natives.get(op) else {
            return Ok(self.host(op, args, back, node));
        };
        let units = super::budget::size(native, &args);
        self.pay(units).map_err(|reason| Fault {
            span: Self::span(program, node),
            reason,
        })?;
        let step = {
            let prelude = self.prelude();
            let tables = program.tables.borrow();
            let ctx = Ctx {
                program,
                tables: &tables,
                prelude: &prelude,
                natives: &natives,
                globals: &self.globals,
                result: result.clone(),
            };
            natives.call(op, args, &ctx)
        };
        match step {
            Ok(Step::Done(value)) => self.deliver(program, value, back),
            Ok(Step::Start(pending)) => {
                self.start(program, pending, Start::new(back, node, result))
            }
            Ok(Step::Call(_)) => unreachable!("a native starts as a frame"),
            Ok(Step::Fault(reason)) | Err(reason) => Err(Fault {
                span: Self::span(program, node),
                reason,
            }),
        }
    }

    /// A selected row's target with the arguments in hand.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row(
        &mut self,
        program: &mir::Program,
        member: usize,
        selected: &Selected,
        mut args: Vec<Value>,
        result: ir::Type,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let eq = self.natives().members.eq == Some(member);
        let (target, receiver_spot, unpacked) = {
            let tables = program.tables.borrow();
            let row = &tables.dispatch[member].rows[selected.row];
            let unpacked = super::dispatch::unpack(
                &mut args,
                row.receiver_spot.unwrap_or(0),
                &row.receiver,
                eq,
            );
            (row.target.clone(), row.receiver_spot, unpacked)
        };
        if let Err(equal) = unpacked {
            let value = self.prelude().bool(equal);
            return self.deliver(program, value, back);
        }
        match target {
            ir::Implementation::Field(index) => {
                let receiver = &args[receiver_spot.unwrap_or(0)];
                let value = super::step::field(receiver, index);
                self.deliver(program, value, back)
            }
            ir::Implementation::Declared(declaration) => self.native(
                program,
                declaration,
                args,
                Some(result),
                back,
                node,
            ),
            ir::Implementation::Derived { kind, ty } => {
                let ty = dispatch::substitute(&ty, &selected.bindings);
                let walk = Walk::derived(kind, args, ty);
                let start = Start::new(back, node, Some(result));
                self.start(program, Pending::Walk(walk), start)
            }
            ir::Implementation::Definition { index, meanings } => {
                let meanings = meanings
                    .iter()
                    .map(|ty| dispatch::substitute(ty, &selected.bindings))
                    .collect();
                let invoke = definition_invoke(
                    program,
                    &self.globals,
                    index,
                    args,
                    meanings,
                );
                self.invoke(program, invoke, back, node)
            }
        }
    }

    /// Eval/apply: fewer arguments than the closure still needs fill it
    /// into a new closure; equal enter its body; more enter with the
    /// first and apply the rest to the result.
    pub(super) fn call_value(
        &mut self,
        program: &mir::Program,
        function: &Value,
        mut args: Vec<Value>,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let Value::Function(closure) = function else {
            unreachable!("a function value, not {function:?}")
        };
        let arity = program.bodies[closure.body].arity;
        let remaining = arity - closure.filled.len();
        if args.len() < remaining {
            let mut filled = closure.filled.clone();
            filled.append(&mut args);
            let partial = Value::Function(Rc::new(Closure {
                body: closure.body,
                captures: closure.captures.clone(),
                meanings: closure.meanings.clone(),
                filled,
            }));
            return self.deliver(program, partial, back);
        }
        let rest = args.split_off(remaining);
        let mut spots = closure.filled.clone();
        spots.append(&mut args);
        let back = if rest.is_empty() {
            back
        } else {
            self.frames.push(Frame::Native(super::frames::NativeFrame {
                node,
                pending: Pending::Apply { rest: Some(rest) },
                back,
                result: None,
            }));
            Return::Resume
        };
        let frame = BodyFrame::new(
            program,
            closure.body,
            spots,
            closure.captures.clone(),
            closure.meanings.clone(),
            back,
        );
        self.frames.push(Frame::Body(frame));
        Ok(Flow::Continue)
    }
}

/// How a definition row runs: its body with the arguments, or its
/// stored value applied to them (a member written as a value).
pub(super) fn definition_invoke(
    program: &mir::Program,
    globals: &[Option<Value>],
    index: usize,
    args: Vec<Value>,
    meanings: Vec<ir::Type>,
) -> Invoke {
    if let Some(body) = program.definitions[index] {
        return Invoke::Body {
            body,
            args,
            meanings,
        };
    }
    // A value member no line stores, unset yet: its body computes it.
    if let (Some(None) | None, Some(body)) =
        (globals.get(index), program.globals[index])
    {
        if args.is_empty() {
            return Invoke::Body {
                body,
                args,
                meanings,
            };
        }
    }
    let value =
        globals.get(index).cloned().flatten().unwrap_or_else(|| {
            panic!("definition {index} has a body or a value")
        });
    if args.is_empty() {
        Invoke::Value(value)
    } else {
        Invoke::Function(value, args)
    }
}
