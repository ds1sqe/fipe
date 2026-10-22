//! The callees, copied from `src/vm/calls.rs` (the callee kinds, a
//! selected row, eval/apply of a function value with its guards, a
//! definition's invocation), `src/vm/step.rs` (a call terminator) and
//! `src/vm.rs` (a host call and its reply). Where the VM pushes a frame
//! and waits for its value, the runtime calls machine code, which
//! returns it.
use super::{
    budget, dispatch, fail, fits, immediate, operands, pay, predicates, walk,
    Ctx, Faulted, Frame, Site,
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
        meanings: Vec<ir::Type>,
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
        } => enter(body, &args, &[], &meanings),
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
        .map(|ty| Value::Meaning(Rc::new(ty.clone())))
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
    let result = frame.local_type(&ctx, *dest);
    let args = values;
    match callee {
        Callee::Body(body) => {
            let meanings: Vec<_> = meanings
                .iter()
                .map(|ty| frame.substitute(&ctx, ty))
                .collect();
            enter(*body, &args, &[], &meanings)
        }
        Callee::Native(op) => native(&ctx, *op, args, Some(result), node),
        Callee::Host(declaration) => {
            host(&ctx, *declaration, args, &result, node)
        }
        Callee::Derived { kind, ty } => {
            let ty = frame.substitute(&ctx, ty);
            walk::derived(&ctx, *kind, args, ty, node)
        }
        Callee::Dispatch { member, witness } => {
            let ty = match witness {
                Witness::Value(Operand::Local(local)) => {
                    Some(frame.local_type(&ctx, *local))
                }
                Witness::Value(_) => None,
                Witness::Meaning(ty) => Some(frame.substitute(&ctx, ty)),
            };
            let selected = dispatch::select_for(
                &ctx.tables(),
                &ctx.prelude,
                *member,
                ty.as_ref(),
                operand.as_ref(),
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
            row(&ctx, *member, &selected, args, result, node)
        }
        Callee::Value(_) => {
            let function = operand.expect("the function value");
            call_value(&ctx, &function, args, node)
        }
    }
}

/// A native: its value, a host's declaration among them a host call.
pub fn native(
    ctx: &Ctx,
    op: usize,
    args: Vec<Value>,
    result: Option<ir::Type>,
    node: usize,
) -> Result<Value, Faulted> {
    // A declared row of a host's namespace is a host call.
    let Some(native) = ctx.natives.get(op) else {
        let ty = result.expect("a host call has a result type");
        return host(ctx, op, args, &ty, node);
    };
    pay(budget::size(native, &args), node)?;
    immediate::immediate(ctx, native, args, result.as_ref(), node)
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
    let eq = ctx.natives.members.eq == Some(member);
    let (target, receiver_spot, unpacked) = {
        let tables = ctx.tables();
        let row = &tables.dispatch[member].rows[selected.row];
        let unpacked = dispatch::unpack(
            &mut args,
            row.receiver_spot.unwrap_or(0),
            &row.receiver,
            eq,
        );
        (row.target.clone(), row.receiver_spot, unpacked)
    };
    if let Err(equal) = unpacked {
        return Ok(ctx.prelude.bool(equal));
    }
    match target {
        ir::Implementation::Field(index) => {
            let receiver = &args[receiver_spot.unwrap_or(0)];
            Ok(super::field(receiver, index))
        }
        ir::Implementation::Declared(declaration) => {
            native(ctx, declaration, args, Some(result), node)
        }
        ir::Implementation::Derived { kind, ty } => {
            let ty = dispatch::substitute(&ty, &selected.bindings);
            walk::derived(ctx, kind, args, ty, node)
        }
        ir::Implementation::Definition { index, meanings } => {
            let meanings = meanings
                .iter()
                .map(|ty| dispatch::substitute(ty, &selected.bindings))
                .collect();
            let call = definition_invoke(ctx, index, args, meanings);
            invoke(ctx, call, node)
        }
    }
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
        let mut filled = closure.filled.clone();
        filled.append(&mut args);
        return Ok(Value::Function(Rc::new(Closure {
            body: closure.body,
            captures: closure.captures.clone(),
            meanings: closure.meanings.clone(),
            filled,
            guards: closure.guards.clone(),
        })));
    }
    let rest = args.split_off(remaining);
    let mut spots = closure.filled.clone();
    spots.append(&mut args);
    let beyond: Vec<_> = closure
        .guards
        .iter()
        .filter(|guard| guard.spot >= arity)
        .map(|guard| Guard {
            spot: guard.spot - arity,
            ..guard.clone()
        })
        .collect();
    for guard in &closure.guards {
        if let Some(value) = spots.get(guard.spot) {
            predicates::enter_by(
                ctx,
                &guard.predicate,
                guard.entry,
                value.clone(),
                node,
            )?;
        }
    }
    let returned =
        enter(closure.body, &spots, &closure.captures, &closure.meanings)?;
    let returned = match returned {
        Value::Function(closure) if !beyond.is_empty() => {
            let mut closure = (*closure).clone();
            let filled = closure.filled.len();
            closure.guards.extend(beyond.into_iter().map(|guard| Guard {
                spot: guard.spot + filled,
                ..guard
            }));
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

/// How a definition row runs: its body with the arguments, or its
/// stored value applied to them (a member written as a value).
pub fn definition_invoke(
    ctx: &Ctx,
    index: usize,
    args: Vec<Value>,
    meanings: Vec<ir::Type>,
) -> Invoke {
    if let Some(body) = ctx.program.definitions[index] {
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
        state.host.call(&request, &state.lines)
    });
    match reply {
        Ok(Ok(value)) => {
            let misfit = {
                let tables = ctx.tables();
                (!fits::fits(&value, ty, &tables, &ctx.prelude))
                    .then(|| fits::named(ty, &tables))
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
