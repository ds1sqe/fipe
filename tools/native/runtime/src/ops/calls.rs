//! The callees, copied from `src/vm/calls.rs` (the callee kinds, a
//! selected row, eval/apply of a function value with its guards, a
//! definition's invocation), `src/vm/step.rs` (a call terminator) and
//! `src/vm.rs` (a host call and its reply). Where the VM pushes a frame
//! and waits for its value, the runtime calls machine code, which
//! returns it.
use super::{
    dispatch, fail, fits, immediate, operands, pay, predicates, walk, Ctx,
    Faulted, Frame, Site,
};
use crate::start;
use fipe::{
    embed::Request,
    ir,
    mir::{Callee, Operand, Terminator, Witness},
    value::{Closure, Guard, Reason, Value},
};
use std::rc::Rc;

/// What an operation asks the program to run (`calls::Invoke`).
pub enum Invoke {
    /// A function value applied to arguments, by eval/apply.
    Function(Value, Vec<Value>),
    /// A body entered with its spots and witnesses.
    Body {
        body: usize,
        args: Vec<Value>,
        meanings: dispatch::Types,
    },
    /// A supplied Host declaration with its actual caller result.
    Declared {
        declaration: usize,
        args: Vec<Value>,
        result: dispatch::Types,
    },
    /// A value already in hand.
    Value(Value),
}

/// Run what an operation at `node` asked for: its value.
pub fn invoke(
    ctx: &Ctx,
    invoke: Invoke,
    node: usize,
) -> Result<Value, Faulted> {
    match invoke {
        Invoke::Function(function, args) => {
            call_value(ctx, &function, args, node)
        }
        Invoke::Body {
            body,
            args,
            meanings,
        } => enter(body, &args, &[], &meanings.0),
        Invoke::Declared {
            declaration,
            args,
            mut result,
        } => native(ctx, declaration, args, result.0.pop(), node),
        Invoke::Value(value) => Ok(value),
    }
}

/// Enter a body's machine code with its spots, captures and witnesses;
/// its result.
///
/// # Errors
///
/// When the body Faults; the Fault is kept.
pub fn enter(
    body: usize,
    args: &[Value],
    captures: &[Value],
    meanings: &[ir::Type],
) -> Result<Value, Faulted> {
    let hidden: Vec<Value> = meanings
        .iter()
        .map(|ty| {
            Value::Meaning(Rc::new(fipe::value::Meaning::new(dispatch::copy(
                ty,
            ))))
        })
        .collect();
    crate::abi::invoke(body, args, captures, &hidden)
}

/// A module's init body, run before the main; its value is dropped.
///
/// # Errors
///
/// When the body Faults; the Fault is kept.
pub fn init(body: usize) -> Result<(), Faulted> {
    enter(body, &[], &[], &[]).map(drop)
}

/// The result of the site's `Call`, performed by the runtime: its
/// arguments, then its callee's operand, read by the generated code;
/// `hidden` the calling frame's witnesses.
///
/// # Errors
///
/// When the callee Faults; the Fault is kept.
///
/// # Panics
///
/// At another terminator, a defect of the lowering.
pub fn call(
    site: Site,
    mut values: Vec<Value>,
    hidden: &[Value],
) -> Result<Value, Faulted> {
    let ctx = Ctx::get();
    let frame = Frame::new(site.body, hidden);
    let node = ctx.node(site);
    let program = Rc::clone(&ctx.program);
    let block = &program.bodies[site.body].blocks[site.block];
    let Terminator::Call {
        callee,
        meanings,
        dest,
        ..
    } = &block.terminator
    else {
        unreachable!("a call site")
    };
    let operand = operands::callee(callee)
        .map(|_| values.pop().expect("the callee's operand"));
    let mut result = dispatch::Types(vec![frame.local_type(&ctx, *dest)]);
    let args = values;
    match callee {
        Callee::Body(body) => {
            let mut copied = dispatch::Types::default();
            for ty in meanings {
                copied.0.push(frame.substitute(&ctx, ty));
            }
            enter(*body, &args, &[], &copied.0)
        }
        Callee::Native(op) => native(&ctx, *op, args, result.0.pop(), node),
        Callee::Host(declaration) => {
            host(&ctx, *declaration, args, &result.0[0], node)
        }
        Callee::Derived { kind, ty } => {
            let ty = frame.substitute(&ctx, ty);
            walk::derived(&ctx, *kind, args, ty, node)
        }
        Callee::Dispatch { .. } => dispatched(
            &ctx,
            &frame,
            &block.terminator,
            operand.as_ref(),
            args,
            result,
            node,
        ),
        Callee::Value(_) => {
            let function = operand.expect("the function value");
            call_value(&ctx, &function, args, node)
        }
    }
}

/// Dispatch the checked call without changing its caller witness or receiver order.
fn dispatched(
    ctx: &Ctx,
    frame: &Frame,
    terminator: &Terminator,
    operand: Option<&Value>,
    mut args: Vec<Value>,
    mut result: dispatch::Types,
    node: usize,
) -> Result<Value, Faulted> {
    let program = &ctx.program;
    let Terminator::Call {
        callee: Callee::Dispatch { member, witness },
        args: source_args,
        meanings,
        ..
    } = terminator
    else {
        unreachable!("a dispatch call site")
    };
    // Call.meanings carries the checked caller C for every value
    // dispatch, even when the selected row will not retain it.
    let mut transported = dispatch::Types::default();
    if matches!(witness, Witness::Value(_)) {
        transported.0.push(copied_callable(
            ctx,
            Some(frame),
            meanings.first().expect("checked dispatch callable"),
            true,
            node,
        )?);
    }
    let ty = dispatch::Types(match witness {
        Witness::Value(Operand::Local(local)) => {
            vec![frame.local_type(ctx, *local)]
        }
        Witness::Value(_) => Vec::new(),
        Witness::Meaning(ty) => vec![frame.substitute(ctx, ty)],
    });
    let selected = dispatch::select_for(
        &ctx.tables(),
        &ctx.prelude,
        *member,
        ty.0.first(),
        operand,
    );
    let Some(selected) = selected else {
        let label = ctx.tables().dispatch[*member].label.clone();
        return Err(fail(
            node,
            Reason::Message(format!(
                "no implementation of {label} for the value"
            )),
        ));
    };
    if let Witness::Value(receiver) = witness {
        let mut positions =
            source_args
                .iter()
                .enumerate()
                .filter_map(|(spot, operand)| {
                    (operand == receiver).then_some(spot)
                });
        let from = positions.next().expect("a receiver is an argument");
        assert!(positions.next().is_none(), "a receiver operand is unique");
        if let Some(adapter) =
            program.receiver_adapters.iter().find(|adapter| {
                adapter.member == *member && adapter.row == selected.row
            })
        {
            return adapter_call(
                ctx,
                adapter,
                &selected,
                &mut transported,
                args,
                from,
                node,
            );
        }
        let to = ctx.tables().dispatch[*member].rows[selected.row]
            .receiver_spot
            .expect("a value row has a receiver");
        if from < to {
            args[from..=to].rotate_left(1);
        } else {
            args[to..=from].rotate_right(1);
        }
    }
    row(
        ctx,
        *member,
        &selected,
        args,
        result.0.pop().expect("result"),
        node,
    )
}

/// Keep the selected table borrow and temporary guards through adapter invocation.
fn adapter_call(
    ctx: &Ctx,
    adapter: &fipe::mir::ReceiverAdapter,
    selected: &dispatch::Selected,
    transported: &mut dispatch::Types,
    mut args: Vec<Value>,
    from: usize,
    node: usize,
) -> Result<Value, Faulted> {
    let program = &ctx.program;
    let context = program.receiver_contexts[adapter.body]
        .as_ref()
        .expect("adapter context");
    let mut callable = dispatch::Types::default();
    if context.protocol_result.is_some() {
        callable
            .0
            .push(transported.0.pop().expect("transported callable"));
    }
    let tables = ctx.tables();
    let row = &tables.dispatch[adapter.member].rows[selected.row];
    let mut meanings = dispatch::Types(vec![dispatch::substitute(
        &row.receiver,
        &selected.bindings,
    )]);
    let mut operation = dispatch::Types::default();
    if let ir::Implementation::Definition { meanings: raw, .. } = &row.target {
        for ty in raw {
            operation
                .0
                .push(dispatch::substitute(ty, &selected.bindings));
        }
    } else {
        let end = context
            .protocol_result
            .as_ref()
            .map_or(program.bodies[adapter.body].hidden.len(), |result| {
                result.meaning
            });
        let hidden = &program.bodies[adapter.body].hidden[1..end];
        for id in &context.input_parameters {
            pay(
                u64::try_from(selected.bindings.len()).unwrap_or(u64::MAX),
                node,
            )?;
            if !selected.bindings.iter().any(|(bound, _)| bound == id) {
                return Err(fail(
                    node,
                    Reason::Host("missing predicate witness".into()),
                ));
            }
        }
        pay(u64::try_from(hidden.len()).unwrap_or(u64::MAX), node)?;
        for id in hidden.iter().copied() {
            operation.0.push(dispatch::substitute(
                &ir::Type::Parameter(id),
                &selected.bindings,
            ));
        }
    }
    pay(u64::try_from(operation.0.len()).unwrap_or(u64::MAX), node)?;
    meanings.0.append(&mut operation.0);
    if context.protocol_result.is_some() {
        pay(1, node)?;
        meanings.0.push(callable.0.pop().expect("checked callable"));
    }
    args[..=from].rotate_right(1);
    let function = super::entries::receiver_closure(
        ctx,
        Closure {
            body: adapter.body,
            captures: Vec::new(),
            meanings: std::mem::take(&mut meanings.0),
            filled: Vec::new(),
            guards: Vec::new(),
        },
        node,
    )?;
    call_value(ctx, &function, args, node)
}

/// A native: its value, a host's declaration among them a host call.
pub fn native(
    ctx: &Ctx,
    op: usize,
    args: Vec<Value>,
    result: Option<ir::Type>,
    node: usize,
) -> Result<Value, Faulted> {
    let result = dispatch::Types(result.into_iter().collect());
    // A declared row of a host's namespace is a host call.
    let Some(native) = ctx.natives.get(op) else {
        return host(
            ctx,
            op,
            args,
            result.0.first().expect("a host call has a result type"),
            node,
        );
    };
    start::with(|state| state.pay_native(native, &args))
        .map_err(|reason| fail(node, reason))?;
    immediate::immediate(ctx, native, args, result.0.first(), node)
}

/// A selected row's target with the arguments in hand.
fn row(
    ctx: &Ctx,
    member: usize,
    selected: &dispatch::Selected,
    mut args: Vec<Value>,
    result: ir::Type,
    node: usize,
) -> Result<Value, Faulted> {
    let mut result = dispatch::Types(vec![result]);
    let eq = ctx.natives.members.eq == Some(member);
    let cmp = ctx.natives.members.cmp == Some(member);
    let (target, receiver_spot, unpacked) = {
        let tables = ctx.tables();
        let row = &tables.dispatch[member].rows[selected.row];
        let unpacked = dispatch::unpack(
            &mut args,
            row.receiver_spot.unwrap_or(0),
            row,
            &selected.bindings,
            eq,
            cmp,
            ctx,
        );
        (row.target.clone(), row.receiver_spot, unpacked)
    };
    if !unpacked.map_err(|reason| fail(node, reason))? {
        return Ok(ctx.prelude.bool(false));
    }
    match target {
        ir::Implementation::Field(index) => {
            let receiver = args.remove(receiver_spot.unwrap_or(0));
            let value = super::field(&receiver, index);
            if args.is_empty() {
                Ok(value)
            } else {
                call_value(ctx, &value, args, node)
            }
        }
        ir::Implementation::Declared(declaration) => {
            native(ctx, declaration, args, result.0.pop(), node)
        }
        ir::Implementation::Derived { kind, ty } => {
            let ty = dispatch::substitute(&ty, &selected.bindings);
            walk::derived(ctx, kind, args, ty, node)
        }
        ir::Implementation::Definition { index, meanings } => {
            let mut copied = dispatch::Types::default();
            for ty in &meanings {
                copied.0.push(dispatch::substitute(ty, &selected.bindings));
            }
            let call = definition_invoke(
                ctx,
                index,
                args,
                std::mem::take(&mut copied.0),
            );
            invoke(ctx, call, node)
        }
    }
}

/// Supplied rows exclude intrinsic declarations and derived fallbacks.
pub(super) fn supplied(ctx: &Ctx, target: &ir::Implementation) -> bool {
    matches!(target, ir::Implementation::Definition { .. })
        || matches!(target, ir::Implementation::Declared(op) if ctx.natives.get(*op).is_none())
}

/// A supplied row uses the caller's actual result, never a cached signature.
pub(super) fn supplied_target(
    ctx: &Ctx,
    target: &ir::Implementation,
    bindings: &[(usize, ir::Type)],
    args: Vec<Value>,
    result: Option<&ir::Type>,
    node: usize,
) -> Result<Option<Invoke>, Faulted> {
    match target {
        ir::Implementation::Definition { index, meanings } => {
            let mut copied = dispatch::Types::default();
            for ty in meanings {
                copied.0.push(dispatch::substitute(ty, bindings));
            }
            Ok(Some(definition_invoke(
                ctx,
                *index,
                args,
                std::mem::take(&mut copied.0),
            )))
        }
        ir::Implementation::Declared(declaration)
            if ctx.natives.get(*declaration).is_none() =>
        {
            let result = result.ok_or_else(|| {
                fail(
                    node,
                    Reason::Message(
                        "missing actual supplied member result".into(),
                    ),
                )
            })?;
            let result = dispatch::Types(vec![copied_callable(
                ctx, None, result, true, node,
            )?]);
            Ok(Some(Invoke::Declared {
                declaration: *declaration,
                args,
                result,
            }))
        }
        _ => Ok(None),
    }
}

pub(super) struct GuardJobs {
    pub(super) owner: Option<Rc<Closure>>,
    pub(super) indices: Vec<(usize, usize)>,
}
impl GuardJobs {
    pub(super) fn select(
        closure: &Rc<Closure>,
        node: usize,
        mut select: impl FnMut(&Guard) -> Option<usize>,
    ) -> Result<Self, Faulted> {
        pay(
            u64::try_from(closure.guards.len()).unwrap_or(u64::MAX),
            node,
        )?;
        let mut indices = Vec::new();
        for (index, guard) in closure.guards.iter().enumerate() {
            if let Some(spot) = select(guard) {
                pay(1, node)?;
                indices.push((index, spot));
            }
        }
        let owner = if indices.is_empty() {
            None
        } else {
            pay(1, node)?;
            Some(closure.clone())
        };
        Ok(Self { owner, indices })
    }
    pub(super) const fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Preserve prefix filling and the original closure's guarded witness copies.
fn partial_call(
    ctx: &Ctx,
    closure: &Rc<Closure>,
    mut args: Vec<Value>,
    node: usize,
) -> Result<Value, Faulted> {
    let mut meanings = dispatch::Types::default();
    for (position, ty) in closure.meanings.iter().enumerate() {
        if ctx
            .program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.protocol_result.as_ref())
            .is_some_and(|result| result.meaning == position)
        {
            meanings.0.push(copied_callable(ctx, None, ty, true, node)?);
        } else if !closure.guards.is_empty() {
            let mut copied = copied_predicate_type(ctx, ty, &[], true, node)?;
            meanings.0.push(copied.0.pop().expect("partial meaning"));
        } else {
            meanings.0.push(dispatch::copy(ty));
        }
    }
    if !closure.guards.is_empty() {
        pay(1, node)?;
        pay(
            u64::try_from(closure.captures.len()).unwrap_or(u64::MAX),
            node,
        )?;
        pay(
            u64::try_from(closure.filled.len().saturating_add(args.len()))
                .unwrap_or(u64::MAX),
            node,
        )?;
    }
    let mut filled = closure.filled.clone();
    filled.append(&mut args);
    if ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.input_contract)
        .is_some_and(|entry| {
            closure.guards.iter().any(|guard| {
                guard.entry == entry
                    && guard.spot >= closure.filled.len()
                    && guard.spot < filled.len()
            })
        })
        && !admit_receiver(ctx, closure, &mut filled, node)?
    {
        return Ok(ctx.prelude.bool(false));
    }
    let guards = predicates::copy_guards(ctx, &closure.guards, node)?;
    let input = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.input_contract);
    let jobs = GuardJobs::select(closure, node, |guard| {
        (input == Some(guard.entry)
            && guard.spot >= closure.filled.len()
            && guard.spot < filled.len())
        .then_some(guard.spot)
    })?;
    let mut partial = Closure {
        body: closure.body,
        captures: closure.captures.clone(),
        meanings: std::mem::take(&mut meanings.0),
        filled,
        guards,
    };
    if let Some(owner) = jobs.owner {
        for (index, spot) in jobs.indices {
            let value = &mut partial.filled[spot];
            *value = super::entries::admit_argument(
                ctx,
                &owner.guards[index],
                value.clone(),
                node,
            )?;
        }
    }
    Ok(Value::Function(Rc::new(partial)))
}

/// Admit the assembled receiver before result projection or effectful guards.
pub(super) fn admit_receiver(
    ctx: &Ctx,
    closure: &Closure,
    spots: &mut [Value],
    node: usize,
) -> Result<bool, Faulted> {
    if let Some(context) = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
    {
        let mut expected = dispatch::Types(vec![dispatch::copy(
            &closure.meanings[context.receiver_meaning],
        )]);
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
        let admitted = dispatch::unpack(
            spots,
            0,
            &row,
            &[],
            context.comparison == fipe::mir::ReceiverComparison::Eq,
            context.comparison == fipe::mir::ReceiverComparison::Cmp,
            ctx,
        );
        expected
            .0
            .push(std::mem::replace(&mut row.receiver, ir::Type::Unit));
        return admitted.map_err(|reason| fail(node, reason));
    }
    Ok(true)
}

/// Peel only after receiver admission, retaining every discarded type guard.
fn peel_protocol_result(
    projected: &mut dispatch::Types,
    arity: usize,
    node: usize,
) -> Result<(), Faulted> {
    let mut discarded = dispatch::Types::default();
    for _ in 0..arity {
        pay(2, node)?;
        match std::mem::replace(&mut projected.0[0], ir::Type::Unit) {
            ir::Type::Function { parameter, result } => {
                discarded.0.push(*parameter);
                projected.0[0] = *result;
            }
            other => {
                projected.0[0] = other;
                return Err(fail(
                    node,
                    Reason::Host("the method callable is not known".into()),
                ));
            }
        }
    }
    Ok(())
}

/// Eval/apply: fewer arguments than the closure still needs fill it
/// into a new closure; equal enter its body; more enter with the first
/// and apply the rest to the result. A full call tests its guarded
/// arguments first, the first guard first; a guard past the body's
/// parameters moves to the function the body returns.
///
/// # Errors
///
/// When a guard refutes its argument or the body Faults.
pub fn call_value(
    ctx: &Ctx,
    function: &Value,
    mut args: Vec<Value>,
    node: usize,
) -> Result<Value, Faulted> {
    let Value::Function(closure) = function else {
        unreachable!("a function value, not {function:?}")
    };
    let arity = ctx.program.bodies[closure.body].arity;
    let remaining = arity - closure.filled.len();
    if args.len() < remaining {
        return partial_call(ctx, closure, args, node);
    }
    let mut projected = dispatch::Types::default();
    let protocol = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.protocol_result.as_ref());
    if let Some(result) = protocol {
        projected.0.push(copied_callable(
            ctx,
            None,
            &closure.meanings[result.meaning],
            true,
            node,
        )?);
    }
    let rest = args.split_off(remaining);
    let mut spots = closure.filled.clone();
    spots.append(&mut args);
    if !admit_receiver(ctx, closure, &mut spots, node)? {
        return Ok(ctx.prelude.bool(false));
    }
    if let Some(result) = protocol {
        peel_protocol_result(&mut projected, result.arity, node)?;
    }
    let beyond = GuardJobs::select(closure, node, |guard| {
        (guard.spot >= arity).then(|| guard.spot - arity)
    })?;
    let input = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.input_contract);
    let jobs = GuardJobs::select(closure, node, |guard| {
        ((input != Some(guard.entry) || guard.spot >= closure.filled.len())
            && guard.spot < spots.len())
        .then_some(guard.spot)
    })?;
    if let Some(owner) = jobs.owner {
        for (index, spot) in jobs.indices {
            let value = &mut spots[spot];
            *value = super::entries::admit_argument(
                ctx,
                &owner.guards[index],
                value.clone(),
                node,
            )?;
        }
    }
    let mut meanings = dispatch::Types::default();
    let hidden = protocol.map_or_else(
        || &closure.meanings,
        |result| {
            for (position, ty) in closure.meanings.iter().enumerate() {
                if position == result.meaning {
                    meanings
                        .0
                        .push(projected.0.pop().expect("protocol result"));
                } else {
                    meanings.0.push(dispatch::copy(ty));
                }
            }
            &meanings.0
        },
    );
    let returned = enter(closure.body, &spots, &closure.captures, hidden)?;
    let returned = match returned {
        Value::Function(closure) if !beyond.is_empty() => {
            let mut closure = copied_guarded_closure(ctx, &closure, node)?;
            append_deferred_guards(ctx, &mut closure, beyond, node)?;
            Value::Function(Rc::new(closure))
        }
        value => value,
    };
    if rest.is_empty() {
        Ok(returned)
    } else {
        call_value(ctx, &returned, rest, node)
    }
}

fn append_deferred_guards(
    ctx: &Ctx,
    closure: &mut Closure,
    beyond: GuardJobs,
    node: usize,
) -> Result<(), Faulted> {
    let filled = closure.filled.len();
    if let Some(owner) = beyond.owner {
        for (index, spot) in beyond.indices {
            let mut guard =
                predicates::copy_guard(ctx, &owner.guards[index], node)?;
            guard.spot = spot.checked_add(filled).ok_or_else(|| {
                fail(node, Reason::Host("guard spot overflow".into()))
            })?;
            closure.guards.push(guard);
        }
    }
    Ok(())
}

/// How a definition row runs: its body with the arguments, or its
/// stored value applied to them (a member written as a value).
pub fn definition_invoke(
    ctx: &Ctx,
    index: usize,
    args: Vec<Value>,
    meanings: Vec<ir::Type>,
) -> Invoke {
    let mut meanings = dispatch::Types(meanings);
    if let Some(body) = ctx.program.definitions[index] {
        if args.len() != ctx.program.bodies[body].arity {
            return Invoke::Function(
                Value::Function(Rc::new(Closure {
                    body,
                    captures: Vec::new(),
                    meanings: std::mem::take(&mut meanings.0),
                    filled: Vec::new(),
                    guards: Vec::new(),
                })),
                args,
            );
        }
        return Invoke::Body {
            body,
            args,
            meanings,
        };
    }
    let stored = start::with(|state| state.globals.get(index).cloned());
    // A value member no line stores, unset yet: its body computes it.
    if let (Some(None) | None, Some(body)) =
        (&stored, ctx.program.globals[index])
    {
        if args.is_empty() {
            return Invoke::Body {
                body,
                args,
                meanings,
            };
        }
    }
    let value = stored
        .flatten()
        .unwrap_or_else(|| panic!("definition {index} has a body or a value"));
    if args.is_empty() {
        Invoke::Value(value)
    } else {
        Invoke::Function(value, args)
    }
}

/// Hand a host call to the host and wait for its reply, whose value
/// must have the shape of `ty`, the call's result type.
fn host(
    ctx: &Ctx,
    declaration: usize,
    arguments: Vec<Value>,
    ty: &ir::Type,
    node: usize,
) -> Result<Value, Faulted> {
    let reply = start::with(|state| {
        let request = Request::Call {
            declaration,
            arguments,
            environment: state.environment.clone(),
        };
        let program = Rc::clone(&state.program);
        let tables = program.tables.borrow();
        state.host.call(&request, &state.lines, &tables)
    });
    match reply {
        Ok(Ok(value)) => {
            let misfit = {
                let tables = ctx.tables();
                let matched = fits::fits(
                    &value,
                    ty,
                    &tables,
                    &ctx.prelude,
                    &mut |units| start::with(|state| state.pay(units)),
                )
                .map_err(|reason| fail(node, reason))?;
                (!matched).then(|| fits::named(ty, &tables))
            };
            if let Some(named) = misfit {
                let message = format!("the reply is not {named}");
                return Err(fail(node, Reason::Host(message)));
            }
            Ok(value)
        }
        Ok(Err(message)) => Err(fail(node, Reason::Host(message))),
        Err(error) => Err(fail(node, Reason::Host(error.to_string()))),
    }
}

pub(super) fn copied_guarded_closure(
    ctx: &Ctx,
    closure: &Closure,
    node: usize,
) -> Result<Closure, Faulted> {
    copied_closure_mode(ctx, closure, node, true)
}
fn copied_closure_mode(
    ctx: &Ctx,
    closure: &Closure,
    node: usize,
    guarded: bool,
) -> Result<Closure, Faulted> {
    if guarded {
        pay(1, node)?;
        pay(
            u64::try_from(closure.captures.len()).unwrap_or(u64::MAX),
            node,
        )?;
        pay(
            u64::try_from(closure.filled.len()).unwrap_or(u64::MAX),
            node,
        )?;
    }
    let callable = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.protocol_result.as_ref())
        .map(|result| result.meaning);
    let mut meanings = dispatch::Types::default();
    for (position, ty) in closure.meanings.iter().enumerate() {
        meanings.0.push(if callable == Some(position) {
            copied_callable(ctx, None, ty, true, node)?
        } else if guarded {
            let mut copied = copied_predicate_type(ctx, ty, &[], true, node)?;
            copied.0.pop().expect("guarded closure meaning")
        } else {
            dispatch::copy(ty)
        });
    }
    let guards = predicates::copy_guards(ctx, &closure.guards, node)?;
    Ok(Closure {
        body: closure.body,
        captures: closure.captures.clone(),
        meanings: std::mem::take(&mut meanings.0),
        filled: closure.filled.clone(),
        guards,
    })
}

#[derive(Clone, Copy)]
enum CallableShape {
    Meaning(usize),
    Tuple,
    Packed,
    All,
    Sum,
    Array,
    Function,
    Not,
}
enum CallableCopy<'a> {
    Read(&'a ir::Type, bool),
    Finish(CallableShape, usize),
}

/// Copy only the new C witness with the Core materializer's work schedule.
pub(super) fn copied_callable(
    ctx: &Ctx,
    frame: Option<&Frame>,
    ty: &ir::Type,
    sessions: bool,
    node: usize,
) -> Result<ir::Type, Faulted> {
    let hidden = frame.map_or(&[][..], |frame| {
        ctx.program.bodies[frame.body].hidden.as_slice()
    });
    pay(u64::try_from(hidden.len()).unwrap_or(u64::MAX), node)?;
    let bindings: Vec<_> = hidden
        .iter()
        .enumerate()
        .map(|(position, id)| {
            (
                *id,
                frame.expect("hidden frame").meanings[position].as_type(),
            )
        })
        .collect();
    let mut done = copied_predicate_type(ctx, ty, &bindings, sessions, node)?;
    Ok(done.0.pop().expect("copied callable"))
}

/// The caller pays the binding count before allocating its borrowed bindings.
/// Completed roots stay guarded through the caller's next fallible payment.
pub(super) fn copied_predicate_type<'a>(
    ctx: &Ctx,
    ty: &'a ir::Type,
    bindings: &[(usize, &'a ir::Type)],
    sessions: bool,
    node: usize,
) -> Result<dispatch::Types, Faulted> {
    predicate_type_mode(ctx, ty, bindings, sessions, node, true)
}

pub(super) fn prepare_predicate_type(
    ctx: &Ctx,
    ty: &ir::Type,
    node: usize,
) -> Result<(), Faulted> {
    predicate_type_mode(ctx, ty, &[], false, node, false).map(|_| ())
}

fn predicate_type_mode<'a>(
    ctx: &Ctx,
    ty: &'a ir::Type,
    bindings: &[(usize, &'a ir::Type)],
    sessions: bool,
    node: usize,
    output: bool,
) -> Result<dispatch::Types, Faulted> {
    let tables = ctx.tables();
    let mut jobs = vec![CallableCopy::Read(ty, true)];
    let mut done = dispatch::Types::default();
    while let Some(job) = jobs.pop() {
        pay(1, node)?;
        match job {
            CallableCopy::Read(ty, substitute) => read_callable(
                ty,
                substitute,
                (bindings, &tables),
                sessions,
                output,
                node,
                (&mut jobs, &mut done),
            )?,
            CallableCopy::Finish(shape, count) => {
                if output {
                    finish_callable(shape, count, &mut done);
                }
            }
        }
    }
    Ok(done)
}

fn fixed_callable_session<'a>(
    ty: &'a ir::Type,
    tables: &'a ir::Tables,
    node: usize,
) -> Result<&'a ir::Type, Faulted> {
    use ir::Type;
    let mut next = ty;
    let mut seen = Vec::new();
    while let Type::Session(index) = next {
        pay(
            u64::try_from(seen.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
            node,
        )?;
        if seen.contains(index) {
            return Err(fail(
                node,
                Reason::Host("nonconcrete resource identity".into()),
            ));
        }
        seen.push(*index);
        if let Some(Some(fixed)) = tables.session_meanings.get(*index) {
            next = fixed;
        } else {
            break;
        }
    }
    Ok(next)
}

fn read_callable<'a>(
    ty: &'a ir::Type,
    substitute: bool,
    environment: (&[(usize, &'a ir::Type)], &'a ir::Tables),
    sessions: bool,
    output: bool,
    node: usize,
    builders: (&mut Vec<CallableCopy<'a>>, &mut dispatch::Types),
) -> Result<(), Faulted> {
    use ir::Type;
    let (bindings, tables) = environment;
    let (jobs, done) = builders;
    if substitute {
        if let Type::Parameter(parameter) = ty {
            pay(u64::try_from(bindings.len()).unwrap_or(u64::MAX), node)?;
            if let Some((_, actual)) =
                bindings.iter().find(|(id, _)| id == parameter)
            {
                jobs.push(CallableCopy::Read(actual, false));
                return Ok(());
            }
        }
    }
    if let Type::Session(slot) = ty {
        if sessions {
            let next = fixed_callable_session(ty, tables, node)?;
            if matches!(next, Type::Session(_)) {
                if output {
                    done.0.push(Type::Session(*slot));
                }
            } else {
                jobs.push(CallableCopy::Read(next, substitute));
            }
        } else if output {
            done.0.push(Type::Session(*slot));
        }
        return Ok(());
    }
    let arity = match ty {
        Type::Meaning { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Packed(arguments)
        | Type::All(arguments)
        | Type::Sum(arguments) => arguments.len(),
        Type::Array { .. } | Type::Function { .. } => 2,
        Type::Not(_) => 1,
        _ => 0,
    };
    pay(u64::try_from(arity).unwrap_or(u64::MAX), node)?;
    let (shape, parts): (_, Vec<&Type>) = match ty {
        Type::Unit => {
            if output {
                done.0.push(Type::Unit);
            }
            return Ok(());
        }
        Type::Parameter(parameter) => {
            if output {
                done.0.push(Type::Parameter(*parameter));
            }
            return Ok(());
        }
        Type::Number(number) => {
            pay(
                u64::try_from(number.bit_length().div_ceil(64))
                    .unwrap_or(u64::MAX),
                node,
            )?;
            if output {
                done.0.push(Type::Number(number.clone()));
            }
            return Ok(());
        }
        Type::Meaning { index, arguments } => {
            (CallableShape::Meaning(*index), arguments.iter().collect())
        }
        Type::Tuple(parts) => (CallableShape::Tuple, parts.iter().collect()),
        Type::Packed(parts) => (CallableShape::Packed, parts.iter().collect()),
        Type::All(parts) => (CallableShape::All, parts.iter().collect()),
        Type::Sum(parts) => (CallableShape::Sum, parts.iter().collect()),
        Type::Array { element, length } => {
            (CallableShape::Array, vec![element, length])
        }
        Type::Function { parameter, result } => {
            (CallableShape::Function, vec![parameter, result])
        }
        Type::Not(inner) => (CallableShape::Not, vec![inner]),
        Type::Session(_) => unreachable!(),
    };
    jobs.push(CallableCopy::Finish(shape, parts.len()));
    jobs.extend(
        parts
            .into_iter()
            .rev()
            .map(|part| CallableCopy::Read(part, substitute)),
    );
    Ok(())
}

fn finish_callable(
    shape: CallableShape,
    count: usize,
    done: &mut dispatch::Types,
) {
    use ir::Type;
    let mut parts = dispatch::Types(done.0.split_off(done.0.len() - count));
    let copied = match shape {
        CallableShape::Meaning(index) => Type::Meaning {
            index,
            arguments: std::mem::take(&mut parts.0),
        },
        CallableShape::Tuple => Type::Tuple(std::mem::take(&mut parts.0)),
        CallableShape::Packed => Type::Packed(std::mem::take(&mut parts.0)),
        CallableShape::All => Type::All(std::mem::take(&mut parts.0)),
        CallableShape::Sum => Type::Sum(std::mem::take(&mut parts.0)),
        CallableShape::Array => {
            let length = Box::new(parts.0.pop().unwrap());
            let element = Box::new(parts.0.pop().unwrap());
            Type::Array { element, length }
        }
        CallableShape::Function => {
            let result = Box::new(parts.0.pop().unwrap());
            let parameter = Box::new(parts.0.pop().unwrap());
            Type::Function { parameter, result }
        }
        CallableShape::Not => Type::Not(Box::new(parts.0.pop().unwrap())),
    };
    done.0.push(copied);
}

#[cfg(test)]
mod copy_work_tests {
    use super::*;
    use crate::{host::Host, ops::State};

    fn source(body: usize, with_guards: bool) -> Closure {
        let mut meaning = ir::Type::Unit;
        let mut predicate = ir::Predicate::Nonzero { when_integer: true };
        for _ in 0..20_000 {
            meaning = ir::Type::Not(Box::new(meaning));
            predicate = ir::Predicate::Not(Box::new(predicate));
        }
        let guards = if with_guards {
            vec![
                Guard {
                    spot: 0,
                    entry: 0,
                    predicate: ir::Predicate::Nonzero { when_integer: true },
                    witnesses: Rc::from([]),
                },
                Guard {
                    spot: 1,
                    entry: 0,
                    predicate: ir::Predicate::All(vec![
                        ir::Predicate::Nonzero { when_integer: true },
                        predicate,
                    ]),
                    witnesses: Rc::from([]),
                },
            ]
        } else {
            drop(predicates::OwnedPredicates(vec![predicate]));
            Vec::new()
        };
        Closure {
            body,
            captures: vec![Value::Int(3)],
            meanings: vec![meaning],
            filled: vec![Value::Int(7)],
            guards,
        }
    }

    fn copied(
        program: &fipe::mir::Program,
        source: &Closure,
        budget: Option<u64>,
    ) -> (Result<Closure, Faulted>, State) {
        let state = State::new(program.clone(), budget, Host::open().unwrap());
        start::test_with_state(state, || {
            copied_guarded_closure(&Ctx::get(), source, 0)
        })
    }

    fn guard_cost(program: &fipe::mir::Program, guard: &Guard) -> u64 {
        let state = State::new(program.clone(), None, Host::open().unwrap());
        let (copy, state) = start::test_with_state(state, || {
            predicates::copy_guard(&Ctx::get(), guard, 0)
        });
        drop(copy.unwrap());
        state.charged
    }

    #[test]
    fn guarded_copy_late_budget_failure_drains_deep_owned_forests() {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                // This metadata only locates Budget; no executable body is needed.
                let mut tables = ir::Tables::default();
                tables.nodes.push(ir::Node {
                    namespace: 0,
                    span: fipe::token::Span::default(),
                    ty: ir::Type::Unit,
                    kind: ir::Kind::Tuple(Vec::new()),
                });
                let program = fipe::mir::Program {
                    tables: Rc::new(std::cell::RefCell::new(tables)),
                    bodies: Vec::new(),
                    receiver_adapters: Vec::new(),
                    receiver_contexts: Vec::new(),
                    constants: Vec::new(),
                    definitions: Vec::new(),
                    tests: Vec::new(),
                    inits: Vec::new(),
                    globals: Vec::new(),
                    main: 0,
                };
                let prefix_source = source(program.main, false);
                let (prefix_copy, prefix_state) =
                    copied(&program, &prefix_source, None);
                drop(prefix_copy.unwrap());
                drop(prefix_source);
                let source = source(program.main, true);
                let first = guard_cost(&program, &source.guards[0]);
                let second = guard_cost(&program, &source.guards[1]);
                let prefix = prefix_state
                    .charged
                    .checked_add(u64::try_from(source.guards.len()).unwrap())
                    .unwrap();
                let later = prefix
                    .checked_add(first)
                    .unwrap()
                    .checked_add(second / 2)
                    .unwrap();
                assert!(later > prefix + first + 8,
                "later failure follows the second guard's first built child");
                assert!(later < prefix + first + second);
                let final_finish = prefix
                    .checked_add(first)
                    .unwrap()
                    .checked_add(second)
                    .unwrap()
                    .checked_sub(1)
                    .unwrap();
                for allowance in [prefix, later, final_finish] {
                    let (result, state) =
                        copied(&program, &source, Some(allowance));
                    assert!(result.is_err());
                    assert!(matches!(
                        state.fault.as_ref().map(|fault| &fault.reason),
                        Some(Reason::Budget)
                    ));
                    assert_eq!(state.charged, allowance);
                    assert_eq!(state.budget, Some(allowance));
                }
                let (copy, state) = copied(&program, &source, None);
                let copy = copy.unwrap();
                assert_eq!(copy.guards.len(), 2);
                assert_eq!(state.charged, prefix + first + second);
                drop(copy);
                drop(source);
                drop(prefix_state);
                drop(state);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn session_copy_program(
        fixed: bool,
        integer: &ir::Type,
    ) -> fipe::mir::Program {
        let mut tables = ir::Tables::default();
        tables.nodes.push(ir::Node {
            namespace: 0,
            span: fipe::token::Span::default(),
            ty: ir::Type::Unit,
            kind: ir::Kind::Tuple(Vec::new()),
        });
        tables.meanings = vec![
            ir::Meaning {
                resource: false,
                label: "Integer".into(),
                namespace: 0,
                parameters: Vec::new(),
                constructors: Vec::new(),
            },
            ir::Meaning {
                resource: false,
                label: "Box".into(),
                namespace: 0,
                parameters: Vec::new(),
                constructors: Vec::new(),
            },
        ];
        tables
            .session_meanings
            .push(fixed.then_some(integer.clone()));
        fipe::mir::Program {
            tables: Rc::new(std::cell::RefCell::new(tables)),
            bodies: Vec::new(),
            receiver_adapters: Vec::new(),
            receiver_contexts: Vec::new(),
            constants: Vec::new(),
            definitions: Vec::new(),
            tests: Vec::new(),
            inits: Vec::new(),
            globals: Vec::new(),
            main: 0,
        }
    }

    #[test]
    fn guarded_closure_session_meanings_are_semantic_but_test_syntax_is_raw() {
        for fixed in [false, true] {
            let integer = ir::Type::Meaning {
                index: 0,
                arguments: Vec::new(),
            };
            let raw = ir::Type::Meaning {
                index: 1,
                arguments: vec![ir::Type::Session(0)],
            };
            let program = session_copy_program(fixed, &integer);
            // Private copy-engine input; executable Unit startup does not create Session slots.
            let source = Closure {
                body: 0,
                captures: Vec::new(),
                meanings: vec![raw.clone()],
                filled: Vec::new(),
                guards: vec![Guard {
                    spot: 0,
                    entry: 0,
                    predicate: ir::Predicate::Test {
                        index: 0,
                        meanings: vec![ir::Type::Session(0)],
                    },
                    witnesses: Rc::from([]),
                }],
            };
            let (copy, state) = copied(&program, &source, None);
            let copy = copy.unwrap();
            let expected_meaning = if fixed {
                ir::Type::Meaning {
                    index: 1,
                    arguments: vec![integer],
                }
            } else {
                raw.clone()
            };
            assert_eq!(copy.meanings, [expected_meaning]);
            assert_eq!(source.meanings.as_slice(), core::slice::from_ref(&raw));
            assert_eq!(copy.guards[0].predicate, source.guards[0].predicate);
            assert!(Rc::ptr_eq(
                &copy.guards[0].witnesses,
                &source.guards[0].witnesses
            ));
            let expected = if fixed { 15 } else { 13 };
            assert_eq!(
                state.charged, expected,
                "fixed={fixed}, exact semantic-copy events"
            );
            let raw_state =
                State::new(program.clone(), None, Host::open().unwrap());
            let (raw_copy, raw_state) =
                start::test_with_state(raw_state, || {
                    predicates::copy_predicate(
                        &Ctx::get(),
                        &source.guards[0].predicate,
                        0,
                    )
                });
            assert_eq!(
                raw_copy.unwrap().0,
                [ir::Predicate::Test {
                    index: 0,
                    meanings: vec![ir::Type::Session(0)]
                }]
            );
            assert_eq!(
                raw_state.charged, 4,
                "raw syntax never follows the fixed Session"
            );
            let (short, short_state) =
                copied(&program, &source, Some(expected - 1));
            assert!(short.is_err());
            assert!(matches!(
                short_state.fault.as_ref().map(|fault| &fault.reason),
                Some(Reason::Budget)
            ));
            assert_eq!(short_state.charged, expected - 1);
            assert_eq!(source.meanings, [raw]);
        }
    }
}
