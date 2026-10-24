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
    compile::dispatch::Selected,
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
    /// A supplied Host row with the exact result known by its native caller.
    Declared {
        declaration: usize,
        args: Vec<Value>,
        result: crate::value::resource::Forest,
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
        receiver_spot: Option<usize>,
        args: Vec<Value>,
        meanings: Vec<ir::Type>,
        result: ir::Type,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let mut result = crate::value::resource::Forest(alloc::vec![result]);
        let mut meanings = crate::value::resource::Forest(meanings);
        let fault = |reason| Self::fault(program, node, reason);
        match callee {
            Callee::Body(body) => {
                let frame = BodyFrame::new(
                    program,
                    *body,
                    args,
                    Vec::new(),
                    core::mem::take(&mut meanings.0),
                    back,
                );
                self.frames.push(Frame::Body(frame));
                Ok(Flow::Continue)
            }
            Callee::Native(op) => self.native(
                program,
                *op,
                args,
                Some(result.0.pop().expect("result")),
                back,
                node,
            ),
            Callee::Host(declaration) => self.host(
                program,
                *declaration,
                args,
                result.0.pop().expect("result"),
                back,
                node,
            ),
            Callee::Derived { kind, ty } => {
                let ty = { self.substitute(program, ty).map_err(&fault)? };
                let walk = Walk::derived(*kind, args, ty);
                let start = Start::new(
                    back,
                    node,
                    Some(result.0.pop().expect("result")),
                );
                self.start(program, Pending::Walk(walk), start)
            }
            Callee::Dispatch { member, witness } => {
                let mut types = crate::value::resource::Forest::default();
                let receiver = match witness {
                    mir::Witness::Value(operand) => {
                        let value =
                            self.operand(program, *operand).map_err(&fault)?;
                        if let mir::Operand::Local(local) = operand {
                            types.0.push(
                                self.local_type(program, *local)
                                    .map_err(&fault)?,
                            );
                        }
                        Some(value)
                    }
                    mir::Witness::Meaning(ty) => {
                        types.0.push(
                            self.substitute(program, ty).map_err(&fault)?,
                        );
                        None
                    }
                };
                let work = self.resource_work();
                let selected = super::dispatch::select_for(
                    &program.tables.borrow(),
                    &self.prelude(),
                    *member,
                    types.0.first(),
                    receiver.as_ref(),
                    &work,
                );
                self.pay(work.spent()).map_err(&fault)?;
                let selected = selected.map_err(&fault)?;
                let Some(selected) = selected else {
                    let label =
                        program.tables.borrow().dispatch[*member].label.clone();
                    return Err(fault(Reason::Message(format!(
                        "no implementation of {label} for the value"
                    ))));
                };
                self.row(
                    program,
                    *member,
                    &selected,
                    receiver_spot,
                    args,
                    result.0.pop().expect("result"),
                    meanings,
                    back,
                    node,
                )
            }
            Callee::Value(operand) => {
                let function =
                    self.operand(program, *operand).map_err(&fault)?;
                self.call_value(program, &function, args, back, node)
            }
        }
    }

    fn align_receiver(
        args: &mut [Value],
        source: Option<usize>,
        row: &crate::ir::DispatchRow,
    ) {
        if let Some(from) = source {
            let to = row.receiver_spot.expect("a value row has a receiver");
            if from < to {
                args[from..=to].rotate_left(1);
            } else {
                args[to..=from].rotate_right(1);
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
        let mut result =
            crate::value::resource::Forest(result.into_iter().collect());
        let natives = self.natives();
        // A declared row of a host's namespace is a host call.
        let Some(native) = natives.get(op) else {
            let ty = result.0.pop().expect("a host call has a result type");
            return self.host(program, op, args, ty, back, node);
        };
        self.pay_native(native, &args)
            .map_err(|reason| Self::fault(program, node, reason))?;
        let work = self.resource_work();
        let step = {
            let prelude = self.prelude();
            let tables = program.tables.borrow();
            let ctx = Ctx {
                program,
                tables: &tables,
                prelude: &prelude,
                natives: &natives,
                globals: &self.globals,
                result: result.0.first(),
                work: &work,
            };
            natives.call(op, args, &ctx)
        };
        self.pay(work.spent())
            .map_err(|reason| Self::fault(program, node, reason))?;
        match step {
            Ok(Step::Done(value)) => self.deliver(program, value, back),
            Ok(Step::Start(pending)) => self.start(
                program,
                pending,
                Start::new(back, node, result.0.pop()),
            ),
            Ok(Step::Call(_)) => unreachable!("a native starts as a frame"),
            Ok(Step::AdmitArgument { .. }) => unreachable!(
                "argument guards start only in their private queue"
            ),
            Ok(Step::Fault(reason)) | Err(reason) => {
                Err(Self::fault(program, node, reason))
            }
        }
    }

    /// A selected row's target with the arguments in hand.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row(
        &mut self,
        program: &mir::Program,
        member: usize,
        selected: &Selected,
        source_receiver: Option<usize>,
        mut args: Vec<Value>,
        result: ir::Type,
        mut callable: crate::value::resource::Forest,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let mut result = crate::value::resource::Forest(alloc::vec![result]);
        let eq = self.natives().members.eq == Some(member);
        let cmp = self.natives().members.cmp == Some(member);
        let tables = program.tables.borrow();
        let row = &tables.dispatch[member].rows[selected.row];
        if let Some(from) = source_receiver {
            if let Some(adapter) =
                program.receiver_adapters.iter().find(|adapter| {
                    adapter.member == member && adapter.row == selected.row
                })
            {
                let mut types = self.adapter_meanings(
                    program,
                    adapter,
                    row,
                    selected,
                    &mut callable,
                    node,
                )?;
                args[..=from].rotate_right(1);
                let function = self
                    .receiver_closure(
                        program,
                        Closure {
                            body: adapter.body,
                            captures: Vec::new(),
                            meanings: core::mem::take(&mut types.0),
                            filled: Vec::new(),
                            guards: Vec::new(),
                        },
                    )
                    .map_err(|reason| Self::fault(program, node, reason))?;
                return self.call_value(program, &function, args, back, node);
            }
        }
        Self::align_receiver(&mut args, source_receiver, row);
        let receiver_spot = row.receiver_spot;
        let unpacked = self.unpack_selected_row(
            program,
            row,
            selected,
            &mut args,
            (eq, cmp),
            node,
        )?;
        if !unpacked {
            return self.deliver(program, self.prelude().bool(false), back);
        }
        match &row.target {
            ir::Implementation::Field(index) => self.field_call(
                program,
                *index,
                args,
                receiver_spot.unwrap_or(0),
                back,
                node,
            ),
            ir::Implementation::Declared(declaration) => self.native(
                program,
                *declaration,
                args,
                Some(result.0.pop().expect("result")),
                back,
                node,
            ),
            ir::Implementation::Derived { kind, ty } => {
                let ty = self
                    .bound_types(
                        program,
                        core::slice::from_ref(ty),
                        &selected.bindings,
                    )
                    .map_err(|reason| Self::fault(program, node, reason))?
                    .pop()
                    .expect("derived type");
                let walk = Walk::derived(*kind, args, ty);
                let start = Start::new(
                    back,
                    node,
                    Some(result.0.pop().expect("result")),
                );
                self.start(program, Pending::Walk(walk), start)
            }
            ir::Implementation::Definition { index, meanings } => {
                let meanings = self
                    .bound_types(program, meanings, &selected.bindings)
                    .map_err(|reason| Self::fault(program, node, reason))?;
                let invoke = definition_invoke(
                    program,
                    &self.globals,
                    *index,
                    args,
                    meanings,
                );
                self.invoke(program, invoke, back, node)
            }
        }
    }

    fn unpack_selected_row(
        &mut self,
        program: &mir::Program,
        row: &ir::DispatchRow,
        selected: &Selected,
        args: &mut [Value],
        comparisons: (bool, bool),
        node: usize,
    ) -> Result<bool, Fault> {
        let work = self.resource_work();
        let unpacked = super::dispatch::unpack(
            args,
            row.receiver_spot.unwrap_or(0),
            row,
            &selected.bindings,
            comparisons,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())
            .map_err(|reason| Self::fault(program, node, reason))?;
        unpacked.map_err(|reason| Self::fault(program, node, reason))
    }

    fn field_call(
        &mut self,
        program: &mir::Program,
        index: usize,
        mut args: Vec<Value>,
        receiver_spot: usize,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let receiver = args.remove(receiver_spot);
        let value = super::step::field(&receiver, index);
        if args.is_empty() {
            self.deliver(program, value, back)
        } else {
            self.call_value(program, &value, args, back, node)
        }
    }

    fn bound_types(
        &mut self,
        program: &mir::Program,
        types: &[ir::Type],
        bindings: &[(usize, ir::Type)],
    ) -> Result<Vec<ir::Type>, Reason> {
        let work = self.resource_work();
        let mut copied = crate::value::resource::Forest::default();
        let result = (|| {
            let tables = program.tables.borrow();
            for ty in types {
                let ty = super::resource::substitute_bindings(
                    ty, bindings, &tables, &work,
                )?;
                copied.0.push(ty);
            }
            Ok::<_, Reason>(())
        })();
        self.pay(work.spent())?;
        result?;
        Ok(core::mem::take(&mut copied.0))
    }

    /// Peel only at full invocation; retain every temporary on failure.
    fn project_protocol_result(
        &mut self,
        meanings: &mut crate::value::resource::Forest,
        result: &mir::ReceiverResult,
    ) -> Result<(), Reason> {
        let mut tail =
            crate::value::resource::Forest(alloc::vec![core::mem::replace(
                &mut meanings.0[result.meaning],
                ir::Type::Unit
            )]);
        let mut discarded = crate::value::resource::Forest::default();
        let work = self.resource_work();
        let projected = (|| {
            for _ in 0..result.arity {
                work.pay(2)?;
                match core::mem::replace(&mut tail.0[0], ir::Type::Unit) {
                    ir::Type::Function { parameter, result } => {
                        discarded.0.push(*parameter);
                        tail.0[0] = *result;
                    }
                    other => {
                        tail.0[0] = other;
                        return Err(Reason::Host(
                            "the method callable is not known".into(),
                        ));
                    }
                }
            }
            Ok(())
        })();
        self.pay(work.spent())?;
        projected?;
        meanings.0[result.meaning] = tail.0.pop().expect("protocol result");
        Ok(())
    }

    fn adapter_meanings(
        &mut self,
        program: &mir::Program,
        adapter: &mir::ReceiverAdapter,
        row: &ir::DispatchRow,
        selected: &Selected,
        callable: &mut crate::value::resource::Forest,
        node: usize,
    ) -> Result<crate::value::resource::Forest, Fault> {
        let mut types = crate::value::resource::Forest(
            self.bound_types(
                program,
                core::slice::from_ref(&row.receiver),
                &selected.bindings,
            )
            .map_err(|reason| Self::fault(program, node, reason))?,
        );
        let operation = if let ir::Implementation::Definition {
            meanings, ..
        } = &row.target
        {
            self.bound_types(program, meanings, &selected.bindings)
        } else {
            let context = program.receiver_contexts[adapter.body]
                .as_ref()
                .expect("adapter context");
            let end =
                context.protocol_result.as_ref().map_or(
                    program.bodies[adapter.body].hidden.len(),
                    |result| result.meaning,
                );
            let hidden = &program.bodies[adapter.body].hidden[1..end];
            for id in &context.input_parameters {
                self.pay(
                    u64::try_from(selected.bindings.len()).unwrap_or(u64::MAX),
                )
                .map_err(|reason| Self::fault(program, node, reason))?;
                if !selected.bindings.iter().any(|(bound, _)| bound == id) {
                    return Err(Self::fault(
                        program,
                        node,
                        Reason::Host("missing predicate witness".into()),
                    ));
                }
            }
            self.pay(u64::try_from(hidden.len()).unwrap_or(u64::MAX))
                .map_err(|reason| Self::fault(program, node, reason))?;
            let parameters: Vec<_> =
                hidden.iter().copied().map(ir::Type::Parameter).collect();
            self.bound_types(program, &parameters, &selected.bindings)
        }
        .map_err(|reason| Self::fault(program, node, reason))?;
        let mut operation = crate::value::resource::Forest(operation);
        self.pay(u64::try_from(operation.0.len()).unwrap_or(u64::MAX))
            .map_err(|reason| Self::fault(program, node, reason))?;
        types.0.append(&mut operation.0);
        if program.receiver_contexts[adapter.body]
            .as_ref()
            .expect("adapter context")
            .protocol_result
            .is_some()
        {
            self.pay(1)
                .map_err(|reason| Self::fault(program, node, reason))?;
            types.0.push(
                callable.0.pop().expect(
                    "a dispatched Declared call has its checked callable",
                ),
            );
        }
        Ok(types)
    }

    fn call_meanings(
        &mut self,
        program: &mir::Program,
        closure: &Closure,
        node: usize,
        guarded_partial: bool,
    ) -> Result<crate::value::resource::Forest, Fault> {
        let callable = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.protocol_result.as_ref())
            .map(|result| result.meaning);
        let mut meanings = crate::value::resource::Forest::default();
        for (position, ty) in closure.meanings.iter().enumerate() {
            if callable == Some(position) || guarded_partial {
                let work = self.resource_work();
                let copied = {
                    let tables = program.tables.borrow();
                    super::resource::materialize(
                        ty,
                        &[],
                        &[],
                        &tables,
                        &work,
                        &mut Ok,
                    )
                };
                self.pay(work.spent())
                    .map_err(|reason| Self::fault(program, node, reason))?;
                let mut copied = copied
                    .map_err(|reason| Self::fault(program, node, reason))?;
                meanings.0.push(copied.0.pop().expect("callable witness"));
            } else {
                meanings.0.extend(
                    self.bound_types(program, core::slice::from_ref(ty), &[])
                        .map_err(|reason| Self::fault(program, node, reason))?,
                );
            }
        }
        Ok(meanings)
    }

    pub(super) fn admit_receiver_only(
        &mut self,
        program: &mir::Program,
        context: &mir::ReceiverContext,
        meanings: &[ir::Type],
        spots: &mut [Value],
        node: usize,
    ) -> Result<bool, Fault> {
        let mut expected = crate::value::resource::Forest(
            self.bound_types(
                program,
                core::slice::from_ref(&meanings[context.receiver_meaning]),
                &[],
            )
            .map_err(|reason| Self::fault(program, node, reason))?,
        );
        let mut row = ir::DispatchRow {
            input_contract: None,
            ty: ir::Type::Unit,
            declared_arity: None,
            receiver: expected.0.pop().expect("receiver witness"),
            receiver_spot: Some(0),
            argument: None,
            target: ir::Implementation::Derived {
                kind: ir::Derived::Default,
                ty: ir::Type::Unit,
            },
        };
        let work = self.resource_work();
        let admitted = super::dispatch::unpack(
            spots,
            0,
            &row,
            &[],
            (
                context.comparison == mir::ReceiverComparison::Eq,
                context.comparison == mir::ReceiverComparison::Cmp,
            ),
            &program.tables.borrow(),
            &work,
        );
        expected
            .0
            .push(core::mem::replace(&mut row.receiver, ir::Type::Unit));
        self.pay(work.spent())
            .map_err(|reason| Self::fault(program, node, reason))?;
        admitted.map_err(|reason| Self::fault(program, node, reason))
    }

    fn admit_call_receiver(
        &mut self,
        program: &mir::Program,
        context: &mir::ReceiverContext,
        meanings: &mut crate::value::resource::Forest,
        spots: &mut [Value],
        back: Return,
        node: usize,
    ) -> Result<Option<Flow>, Fault> {
        if !self.admit_receiver_only(
            program,
            context,
            &meanings.0,
            spots,
            node,
        )? {
            return self
                .deliver(program, self.prelude().bool(false), back)
                .map(Some);
        }
        if let Some(result) = &context.protocol_result {
            self.project_protocol_result(meanings, result)
                .map_err(|reason| Self::fault(program, node, reason))?;
        }
        Ok(None)
    }

    fn prepay_partial_slots(
        &mut self,
        closure: &Closure,
        extra: usize,
    ) -> Result<(), Reason> {
        if closure.guards.is_empty() {
            return Ok(());
        }
        let work = self.resource_work();
        let paid = (|| {
            work.pay(1)?;
            work.pay(
                u64::try_from(closure.captures.len()).unwrap_or(u64::MAX),
            )?;
            work.pay(
                u64::try_from(closure.filled.len().saturating_add(extra))
                    .unwrap_or(u64::MAX),
            )
        })();
        self.pay(work.spent())?;
        paid
    }

    fn partial_call(
        &mut self,
        program: &mir::Program,
        closure: &Rc<Closure>,
        mut args: Vec<Value>,
        mut meanings: crate::value::resource::Forest,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        self.prepay_partial_slots(closure, args.len())
            .map_err(|reason| Self::fault(program, node, reason))?;
        let mut filled = closure.filled.clone();
        filled.append(&mut args);
        if let Some(context) = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
        {
            if context.input_contract.is_some_and(|entry| {
                closure.guards.iter().any(|guard| {
                    guard.entry == entry
                        && guard.spot >= closure.filled.len()
                        && guard.spot < filled.len()
                })
            }) && !self.admit_receiver_only(
                program,
                context,
                &meanings.0,
                &mut filled,
                node,
            )? {
                return self.deliver(program, self.prelude().bool(false), back);
            }
        }
        // Keep copied meanings owned until every fallible guard copy succeeds.
        let work = self.resource_work();
        let copied = super::resource::copy_guards(
            &closure.guards,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())
            .map_err(|reason| Self::fault(program, node, reason))?;
        let guards =
            copied.map_err(|reason| Self::fault(program, node, reason))?;
        let input = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.input_contract);
        let guarded = self
            .guard_jobs(closure, |guard| {
                (input == Some(guard.entry)
                    && guard.spot >= closure.filled.len()
                    && guard.spot < filled.len())
                .then_some(guard.spot)
            })
            .map_err(|reason| Self::fault(program, node, reason))?;
        let partial = Value::Function(Rc::new(Closure {
            body: closure.body,
            captures: closure.captures.clone(),
            meanings: core::mem::take(&mut meanings.0),
            filled,
            guards,
        }));
        if !guarded.is_empty() {
            let Value::Function(partial) = partial else {
                unreachable!()
            };
            let pending = super::pending::Arguments::partial(partial, guarded)
                .map_err(|reason| Self::fault(program, node, reason))?;
            self.frames.push(Frame::Native(super::frames::NativeFrame {
                node,
                pending: Pending::Arguments(pending),
                back,
                result: None,
            }));
            return Ok(Flow::Continue);
        }
        self.deliver(program, partial, back)
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
        let mut meanings = self.call_meanings(
            program,
            closure,
            node,
            args.len() < remaining && !closure.guards.is_empty(),
        )?;
        if args.len() < remaining {
            return self
                .partial_call(program, closure, args, meanings, back, node);
        }
        let rest = args.split_off(remaining);
        let mut spots = closure.filled.clone();
        spots.append(&mut args);
        if let Some(context) = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
        {
            if let Some(flow) = self.admit_call_receiver(
                program,
                context,
                &mut meanings,
                &mut spots,
                back,
                node,
            )? {
                return Ok(flow);
            }
        }
        // A guard past the body's parameters tests the function it
        // returns, at the spot that function takes.
        let beyond = self
            .guard_jobs(closure, |guard| {
                (guard.spot >= arity).then(|| guard.spot - arity)
            })
            .map_err(|reason| Self::fault(program, node, reason))?;
        let back = if rest.is_empty() && beyond.is_empty() {
            back
        } else {
            self.frames.push(Frame::Native(super::frames::NativeFrame {
                node,
                pending: Pending::Apply {
                    rest: (!rest.is_empty()).then_some(rest),
                    guards: beyond,
                },
                back,
                result: None,
            }));
            Return::Resume
        };
        let input = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.input_contract);
        let guarded = self
            .guard_jobs(closure, |guard| {
                ((input != Some(guard.entry)
                    || guard.spot >= closure.filled.len())
                    && guard.spot < spots.len())
                .then_some(guard.spot)
            })
            .map_err(|reason| Self::fault(program, node, reason))?;
        let pending = (!guarded.is_empty())
            .then(|| super::pending::Arguments::new(spots.clone(), guarded));
        let frame = BodyFrame::new(
            program,
            closure.body,
            spots,
            closure.captures.clone(),
            core::mem::take(&mut meanings.0),
            back,
        );
        self.frames.push(Frame::Body(frame));
        if let Some(pending) = pending {
            self.frames.push(Frame::Native(super::frames::NativeFrame {
                node,
                pending: Pending::Arguments(pending),
                back: Return::Arguments {
                    body: closure.body,
                    arity,
                    node,
                },
                result: None,
            }));
        }
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
    let mut meanings = crate::value::resource::Forest(meanings);
    if let Some(body) = program.definitions[index] {
        if args.len() != program.bodies[body].arity {
            return Invoke::Function(
                Value::Function(Rc::new(Closure {
                    body,
                    captures: Vec::new(),
                    meanings: core::mem::take(&mut meanings.0),
                    filled: Vec::new(),
                    guards: Vec::new(),
                })),
                args,
            );
        }
        return Invoke::Body {
            body,
            args,
            meanings: core::mem::take(&mut meanings.0),
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
                meanings: core::mem::take(&mut meanings.0),
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
