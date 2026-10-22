//! What the generated code does with locals beyond copying them, copied
//! from `src/vm/assign.rs` (an rvalue into a local) and `src/vm/step.rs`
//! (a field read, a switch's key, a global read before the step that
//! reads it).
use super::{calls, casts, fail, predicates, Ctx, Faulted, Frame, Site};
use crate::start;
use fipe::{
    ir,
    mir::{self, Rvalue, Statement},
    value::{Closure, Packed, Reason, Record, Value},
};
use std::rc::Rc;

/// The value of the site's `Assign` of an rvalue the runtime computes,
/// its operands read into `args` by the generated code; `hidden` the
/// frame's witnesses.
///
/// # Errors
///
/// When a native or a cast Faults; the Fault is kept.
///
/// # Panics
///
/// At another statement, a defect of the lowering.
pub fn assign(
    site: Site,
    mut args: Vec<Value>,
    hidden: &[Value],
) -> Result<Value, Faulted> {
    let ctx = Ctx::get();
    let frame = Frame::new(site.body, hidden);
    let node = ctx.node(site);
    let block = &ctx.program.bodies[site.body].blocks[site.block];
    let Statement::Assign(dest, rvalue) = &block.statements[site.index] else {
        unreachable!("an assign site")
    };
    Ok(match rvalue {
        // A rebuild is a record of its one value.
        Rvalue::Construct {
            meaning,
            constructor,
            ..
        }
        | Rvalue::Wrap {
            meaning,
            constructor,
            ..
        } => Value::Record(Rc::new(Record {
            meaning: *meaning,
            constructor: *constructor,
            fields: args,
        })),
        Rvalue::Tuple(_) => Value::Tuple(Rc::new(args)),
        Rvalue::Array(_) => Value::seq(args),
        // Captures are copied now, meanings substituted by this frame.
        Rvalue::Closure { body, meanings, .. } => {
            Value::Function(Rc::new(Closure {
                body: *body,
                captures: args,
                meanings: meanings
                    .iter()
                    .map(|ty| frame.substitute(&ctx, ty))
                    .collect(),
                filled: Vec::new(),
                guards: Vec::new(),
            }))
        }
        Rvalue::Cast { row, .. } => {
            // The target is the local's type, witnesses in place.
            let target = frame.local_type(&ctx, *dest);
            let value = args.pop().expect("the cast's value");
            casts::cast(*row, &target, value, &ctx.prelude)
                .map_err(|reason| fail(node, reason))?
        }
        // A native rvalue computes at once.
        Rvalue::Native { op, .. } => {
            let result = frame.local_type(&ctx, *dest);
            calls::native(&ctx, *op, args, Some(result), node)?
        }
        Rvalue::Repeat { length, .. } => {
            let value = args.pop().expect("the repeated value");
            repeat(&frame, value, *length)
        }
        Rvalue::Length(_) => match args.pop() {
            Some(Value::Seq(parts)) => {
                Value::Int(i64::try_from(parts.len()).expect("a length fits"))
            }
            other => unreachable!("a sequence, not {other:?}"),
        },
        Rvalue::Pack { tag, .. } => {
            let ir::Type::Meaning { index, .. } = frame.substitute(&ctx, tag)
            else {
                unreachable!("a tag is a meaning")
            };
            Value::Packed(Rc::new(Packed {
                meaning: index,
                value: args.pop().expect("the packed value"),
            }))
        }
        Rvalue::Convert {
            mode, predicate, ..
        } => {
            let value = args.pop().expect("the converted value");
            convert(&ctx, &frame, *dest, *mode, predicate, value, node)?
        }
        Rvalue::Use(_) | Rvalue::Field(..) => {
            unreachable!("the generated code assigns {rvalue:?}")
        }
    })
}

/// A repeated element, its length written or read from a witness.
fn repeat(frame: &Frame, value: Value, length: mir::Length) -> Value {
    let length = match length {
        mir::Length::Fixed(length) => length,
        mir::Length::Hidden(index) => match &frame.meanings[index] {
            ir::Type::Number(number) => number.to_u64().expect("a length fits"),
            other => unreachable!("a length, not {other:?}"),
        },
    };
    let length = usize::try_from(length).expect("a length fits");
    Value::seq(vec![value; length])
}

/// A conversion: direct, wrapped, unwrapped, or checked against its
/// predicate by the predicate's tests.
fn convert(
    ctx: &Ctx,
    frame: &Frame,
    dest: usize,
    mode: ir::ConversionMode,
    predicate: &ir::Predicate,
    value: Value,
    node: usize,
) -> Result<Value, Faulted> {
    let prelude = &ctx.prelude;
    Ok(match mode {
        ir::ConversionMode::Direct => value,
        ir::ConversionMode::Wrap => match frame.local_type(ctx, dest) {
            ir::Type::Meaning { index, .. } if Some(index) == prelude.maybe => {
                prelude.just(value)
            }
            _ => prelude.ok(value),
        },
        ir::ConversionMode::Unwrap => match &value {
            Value::Record(record) if record.constructor == 0 => {
                record.fields[0].clone()
            }
            _ => return Err(fail(node, Reason::Cast)),
        },
        ir::ConversionMode::Check | ir::ConversionMode::TryCheck => {
            let passed = predicates::holds(ctx, predicate, &value, node)?;
            match (mode, passed) {
                (ir::ConversionMode::TryCheck, true) => prelude.ok(value),
                (ir::ConversionMode::TryCheck, false) => prelude.limit_error(),
                (_, true) => value,
                (_, false) => return Err(fail(node, Reason::Cast)),
            }
        }
    })
}

/// A value definition's global: the stored value, or, unset, the value
/// its body computes, stored first.
///
/// # Errors
///
/// When the body Faults.
///
/// # Panics
///
/// On a global unset with no body, a defect of the program.
pub fn global(definition: usize) -> Result<Value, Faulted> {
    let (stored, body) = start::with(|state| {
        let stored = state.globals[definition].clone();
        (stored, state.program.globals[definition])
    });
    if let Some(value) = stored {
        return Ok(value);
    }
    let body = body
        .expect("a global with no value has a body, or its line was withdrawn");
    let value = calls::enter(body, &[], &[], &[])?;
    start::with(|state| state.globals[definition] = Some(value.clone()));
    Ok(value)
}

/// A record's field, a tuple's part, a sequence's element, a packed
/// value's payload, or the bytes of text (`String { bytes }`).
///
/// # Panics
///
/// On a value with no parts, a defect of the lowering.
#[must_use]
pub fn field(value: &Value, index: usize) -> Value {
    match value {
        Value::Record(record) => record.fields[index].clone(),
        Value::Tuple(parts) | Value::Seq(parts) => parts[index].clone(),
        Value::Packed(packed) => packed.value.clone(),
        Value::Text(_) => fipe::value::text::bytes(value),
        other => unreachable!("a field of {other:?}"),
    }
}

/// The key a switch compares with its cases: a record's constructor, a
/// packed value's meaning, an integer's number.
///
/// # Panics
///
/// On another value, a defect of the lowering.
#[must_use]
pub fn switch_key(value: &Value) -> u64 {
    let key = match value {
        Value::Record(record) => record.constructor,
        Value::Packed(packed) => packed.meaning,
        Value::Int(number) => usize::try_from(*number).unwrap_or(usize::MAX),
        other => unreachable!("a switch on {other:?}"),
    };
    key as u64
}
