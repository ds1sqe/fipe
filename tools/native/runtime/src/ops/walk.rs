//! Structural walks that may meet a written member, copied from
//! `src/vm/walk.rs` (equality, order and display of a value by its tag)
//! and `src/vm/defaults.rs` (the default of a type), with the lookup
//! of a written row by a value's tag from `src/vm/pending.rs`. Each
//! walks with the VM's explicit work stack; where the VM calls a
//! written member and resumes with its answer, the walk calls machine
//! code and continues with the value it returns.
use super::{calls, dispatch, fail, Ctx, Faulted};
use fipe::{
    ir,
    value::{arith, float, structural, text, Reason, Record, Value},
};
use std::{cmp::Ordering, rc::Rc};

/// A derived member with the call's arguments: `other` then `self`
/// for `eq` and `cmp`, `self` for `show`, none for `default`.
pub fn derived(
    ctx: &Ctx,
    kind: ir::Derived,
    mut args: Vec<Value>,
    ty: ir::Type,
    node: usize,
) -> Result<Value, Faulted> {
    let mut ty = dispatch::Types(vec![ty]);
    let mut arg =
        |index: usize| std::mem::replace(&mut args[index], Value::Unit);
    match kind {
        ir::Derived::Eq => {
            let (other, this) = (arg(0), arg(1));
            eq(ctx, this, other, node)
        }
        ir::Derived::Cmp => {
            let (other, this) = (arg(0), arg(1));
            cmp(ctx, this, other, node)
        }
        ir::Derived::Show => show(ctx, arg(0), node),
        ir::Derived::Default => {
            default(ctx, ty.0.pop().expect("default type"), node)
        }
    }
}

/// Call a member's written row on a receiver by its tag, the other
/// arguments before or after it as the row's receiver spot says; the
/// prelude's own rows and a derived row give a `Message` reason.
pub fn member_call(
    ctx: &Ctx,
    member: Option<usize>,
    receiver: &Value,
    others: Vec<Value>,
    result: Option<&ir::Type>,
    node: usize,
) -> Result<calls::Invoke, Faulted> {
    let missing = || {
        fail(
            node,
            Reason::Message("no written member for the value".into()),
        )
    };
    let selected = written_row(ctx, member, receiver).ok_or_else(missing)?;
    let tables = ctx.tables();
    let member = member.expect("selected member");
    let row = &tables.dispatch[member].rows[selected.row];
    let mut args = others;
    let spot = row.receiver_spot.unwrap_or(0).min(args.len());
    args.insert(spot, receiver.clone());
    let eq = ctx.natives.members.eq == Some(member);
    let cmp = ctx.natives.members.cmp == Some(member);
    if !dispatch::unpack(&mut args, spot, row, &selected.bindings, eq, cmp, ctx)
        .map_err(|reason| fail(node, reason))?
    {
        return Ok(calls::Invoke::Value(ctx.prelude.bool(false)));
    }
    let scalar = if eq {
        Some(ctx.prelude.bool)
    } else if cmp {
        Some(ctx.prelude.ordering)
    } else if ctx.natives.members.show == Some(member) {
        Some(ctx.prelude.string)
    } else if ctx.natives.members.len == Some(member) && result.is_none() {
        Some(ctx.prelude.integer)
    } else {
        None
    };
    let scalar = scalar
        .map(|identity| {
            identity.ok_or_else(|| {
                fail(
                    node,
                    Reason::Message(
                        "missing supplied member result meaning".into(),
                    ),
                )
            })
        })
        .transpose()?;
    let fixed = ir::Type::Meaning {
        index: scalar.unwrap_or(0),
        arguments: Vec::new(),
    };
    let result = if scalar.is_some() {
        Some(&fixed)
    } else {
        result
    };
    calls::supplied_target(
        ctx,
        &row.target,
        &selected.bindings,
        args,
        result,
        node,
    )?
    .ok_or_else(missing)
}

pub fn written_row(
    ctx: &Ctx,
    member: Option<usize>,
    receiver: &Value,
) -> Option<dispatch::Selected> {
    let member = member?;
    let tables = ctx.tables();
    let selected = dispatch::by_tag(&tables, &ctx.prelude, member, receiver)?;
    calls::supplied(ctx, &tables.dispatch[member].rows[selected.row].target)
        .then_some(selected)
}

/// A written member's answer for a receiver at `node`.
fn written(
    ctx: &Ctx,
    member: Option<usize>,
    receiver: &Value,
    others: Vec<Value>,
    node: usize,
) -> Result<Value, Faulted> {
    let call = member_call(ctx, member, receiver, others, None, node)?;
    calls::invoke(ctx, call, node)
}

/// The children of two values of one shape, `None` when they differ
/// here, `Some(empty)` for equal leaves.
fn children(a: &Value, b: &Value) -> Option<Vec<(Value, Value)>> {
    let zip = |a: &[Value], b: &[Value]| {
        (a.len() == b.len())
            .then(|| a.iter().cloned().zip(b.iter().cloned()).collect())
    };
    match (a, b) {
        (Value::Seq(a), Value::Seq(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
            zip(a, b)
        }
        (Value::Record(a), Value::Record(b)) => (a.meaning == b.meaning
            && a.constructor == b.constructor)
            .then(|| zip(&a.fields, &b.fields))
            .flatten(),
        (Value::Packed(a), Value::Packed(b)) => {
            dispatch::equal(a.source_type(), b.source_type())
                .then(|| vec![(a.value().clone(), b.value().clone())])
        }
        _ => structural::eq(a, b).then(Vec::new),
    }
}

/// Equality by tag, a written `eq` where a record's meaning has one.
pub fn eq(
    ctx: &Ctx,
    a: Value,
    b: Value,
    node: usize,
) -> Result<Value, Faulted> {
    let member = ctx.natives.members.eq;
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        if matches!(
            (&a, &b),
            (Value::Record(_), Value::Record(_))
                | (Value::Packed(_), Value::Packed(_))
        ) && written_row(ctx, member, &a).is_some()
        {
            let answer = written(ctx, member, &a, vec![b], node)?;
            if !ctx.prelude.is_true(&answer) {
                return Ok(ctx.prelude.bool(false));
            }
            continue;
        }
        match children(&a, &b) {
            None => return Ok(ctx.prelude.bool(false)),
            Some(pairs) => work.extend(pairs.into_iter().rev()),
        }
    }
    Ok(ctx.prelude.bool(true))
}

enum Order {
    Pair(Value, Value),
    Decided(Ordering),
}

/// An `Ordering` value's order.
pub fn ordering_of(value: &Value) -> Ordering {
    match value {
        Value::Record(record) => match record.constructor {
            0 => Ordering::Less,
            1 => Ordering::Equal,
            _ => Ordering::Greater,
        },
        other => unreachable!("an Ordering, not {other:?}"),
    }
}

/// Order by tag, a written `cmp` where a record's meaning has one.
pub fn cmp(
    ctx: &Ctx,
    a: Value,
    b: Value,
    node: usize,
) -> Result<Value, Faulted> {
    let member = ctx.natives.members.cmp;
    let decided = |ordering| Ok(ctx.prelude.ordering(ordering));
    let mut work = vec![Order::Pair(a, b)];
    while let Some(item) = work.pop() {
        let (a, b) = match item {
            Order::Decided(ordering) => {
                if ordering != Ordering::Equal {
                    return decided(ordering);
                }
                continue;
            }
            Order::Pair(a, b) => (a, b),
        };
        let children = |a: &[Value],
                        b: &[Value],
                        after: Ordering,
                        work: &mut Vec<Order>| {
            work.push(Order::Decided(after));
            work.extend(
                a.iter()
                    .cloned()
                    .zip(b.iter().cloned())
                    .rev()
                    .map(|(a, b)| Order::Pair(a, b)),
            );
        };
        match (&a, &b) {
            (Value::Record(x), Value::Record(y)) => {
                if written_row(ctx, member, &a).is_some() {
                    let answer = written(ctx, member, &a, vec![b], node)?;
                    let ordering = ordering_of(&answer);
                    if ordering != Ordering::Equal {
                        return decided(ordering);
                    }
                    continue;
                }
                match x.constructor.cmp(&y.constructor) {
                    Ordering::Equal => {
                        children(
                            &x.fields,
                            &y.fields,
                            Ordering::Equal,
                            &mut work,
                        );
                    }
                    ordering => return decided(ordering),
                }
            }
            (Value::Seq(x), Value::Seq(y))
            | (Value::Tuple(x), Value::Tuple(y)) => {
                children(x, y, x.len().cmp(&y.len()), &mut work);
            }
            (Value::Packed(x), Value::Packed(y)) => {
                if written_row(ctx, member, &a).is_some() {
                    let answer = written(ctx, member, &a, vec![b], node)?;
                    let ordering = ordering_of(&answer);
                    if ordering != Ordering::Equal {
                        return decided(ordering);
                    }
                    continue;
                }
                let ordering = match (x.source_type(), y.source_type()) {
                    (
                        ir::Type::Meaning { index: a, .. },
                        ir::Type::Meaning { index: b, .. },
                    ) => a.cmp(b),
                    (a, b) if dispatch::equal(a, b) => Ordering::Equal,
                    _ => {
                        return Err(fail(
                            node,
                            Reason::Message(
                                "incompatible packed ordering types".into(),
                            ),
                        ))
                    }
                };
                match ordering {
                    Ordering::Equal => work.push(Order::Pair(
                        x.value().clone(),
                        y.value().clone(),
                    )),
                    ordering => return decided(ordering),
                }
            }

            _ => {
                let ordering = structural::cmp(&a, &b);
                if ordering != Ordering::Equal {
                    return decided(ordering);
                }
            }
        }
    }
    decided(Ordering::Equal)
}

enum Piece {
    Value(Value, bool),
    Text(String),
}

/// Display by tag, a written `show` where a record's meaning has one.
pub fn show(ctx: &Ctx, value: Value, node: usize) -> Result<Value, Faulted> {
    let member = ctx.natives.members.show;
    let mut work = vec![Piece::Value(value, false)];
    let mut out = String::new();
    while let Some(piece) = work.pop() {
        let (value, is_argument) = match piece {
            Piece::Text(text) => {
                out.push_str(&text);
                continue;
            }
            Piece::Value(value, is_argument) => (value, is_argument),
        };
        if matches!(value, Value::Record(_) | Value::Packed(_))
            && written_row(ctx, member, &value).is_some()
        {
            let answer = written(ctx, member, &value, vec![], node)?;
            let Value::Text(shown) = answer else {
                unreachable!("show gives text")
            };
            if is_argument && shown.contains(' ') {
                out.push('(');
                out.push_str(&shown);
                out.push(')');
            } else {
                out.push_str(&shown);
            }
            continue;
        }
        match &value {
            Value::Resource(_) => {
                return Err(fail(
                    node,
                    Reason::Message(
                        "native execution does not support resource values"
                            .into(),
                    ),
                ))
            }
            Value::Record(record) => {
                let parenthesize = is_argument && !record.fields.is_empty();
                if parenthesize {
                    work.push(Piece::Text(String::from(")")));
                }
                for field in record.fields.iter().rev() {
                    work.push(Piece::Value(field.clone(), true));
                    work.push(Piece::Text(String::from(" ")));
                }
                let tables = ctx.tables();
                let meaning = &tables.meanings[record.meaning];
                let label = meaning
                    .constructors
                    .get(record.constructor)
                    .map_or(&meaning.label, |constructor| &constructor.label);
                work.push(Piece::Text(label.clone()));
                if parenthesize {
                    work.push(Piece::Text(String::from("(")));
                }
            }
            Value::Seq(parts) | Value::Tuple(parts) => {
                let (open, close) = if matches!(value, Value::Seq(_)) {
                    ("[", "]")
                } else {
                    ("(", ")")
                };
                work.push(Piece::Text(String::from(close)));
                for (index, part) in parts.iter().enumerate().rev() {
                    work.push(Piece::Value(part.clone(), false));
                    if index > 0 {
                        work.push(Piece::Text(String::from(", ")));
                    }
                }
                work.push(Piece::Text(String::from(open)));
            }
            Value::Packed(packed) => {
                work.push(Piece::Value(packed.value().clone(), is_argument));
            }
            Value::Int(_) | Value::Big(_) => out.push_str(&arith::show(&value)),
            Value::F32(_) | Value::F64(_) => out.push_str(&float::show(&value)),
            Value::Text(_) => out.push_str(&text::show(&value)),
            Value::Unit | Value::Function(_) | Value::Meaning(_) => {
                out.push_str(&structural::show(&value, &ctx.tables()));
            }
        }
    }
    Ok(Value::text(&out))
}

/// A value under construction from its parts' defaults.
struct Build {
    shape: Shape,
    parts: Vec<ir::Type>,
    built: Vec<Value>,
}

impl Drop for Build {
    fn drop(&mut self) {
        drop(dispatch::Types(std::mem::take(&mut self.parts)));
    }
}

enum Shape {
    Tuple,
    Array(usize),
    Record {
        meaning: usize,
        constructor: usize,
    },
    /// The one type a default is asked of.
    Top,
}

/// A finished build's value from its built parts.
fn assemble(mut build: Build) -> Value {
    let shape = std::mem::replace(&mut build.shape, Shape::Top);
    let values = std::mem::take(&mut build.built);
    match shape {
        Shape::Top => values.into_iter().next().expect("one"),
        Shape::Tuple => Value::Tuple(Rc::new(values)),
        Shape::Array(length) => {
            let element = values.into_iter().next().expect("the element");
            Value::seq(vec![element; length])
        }
        Shape::Record {
            meaning,
            constructor,
        } => Value::Record(Rc::new(Record {
            meaning,
            constructor,
            fields: values,
        })),
    }
}

/// The written `default` row of a meaning: its definition and meanings.
fn written_default(
    ctx: &Ctx,
    ty: &ir::Type,
    node: usize,
) -> Result<Option<calls::Invoke>, Faulted> {
    let Some(member) = ctx.natives.members.default else {
        return Ok(None);
    };
    let tables = ctx.tables();
    let Some(selected) =
        dispatch::select_for(&tables, &ctx.prelude, member, Some(ty), None)
    else {
        return Ok(None);
    };
    calls::supplied_target(
        ctx,
        &tables.dispatch[member].rows[selected.row].target,
        &selected.bindings,
        Vec::new(),
        Some(ty),
        node,
    )
}

/// Build a tuple or array before continuing the existing default walk.
fn compound_default(
    ty: &ir::Type,
    node: usize,
) -> Result<Option<Build>, Faulted> {
    let build = match ty {
        ir::Type::Tuple(parts) => Build {
            shape: Shape::Tuple,
            parts: parts.iter().map(dispatch::copy).collect(),
            built: Vec::new(),
        },
        ir::Type::Array { element, length } => {
            let ir::Type::Number(length) = &**length else {
                return Err(fail(
                    node,
                    Reason::Message(String::from(
                        "an array length is a number",
                    )),
                ));
            };
            let length =
                usize::try_from(length.to_u64().unwrap_or(0)).unwrap_or(0);
            Build {
                shape: Shape::Array(length),
                parts: vec![dispatch::copy(element)],
                built: Vec::new(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(build))
}

/// The default of a type, built part by part; a meaning with a written
/// `default` row is called for its value.
fn default(ctx: &Ctx, ty: ir::Type, node: usize) -> Result<Value, Faulted> {
    let at = |reason| fail(node, reason);
    let mut work = vec![Build {
        shape: Shape::Top,
        parts: vec![ty],
        built: Vec::new(),
    }];
    loop {
        let Some(build) = work.last_mut() else {
            unreachable!("the top build finishes the walk")
        };
        if build.built.len() == build.parts.len() {
            let finished = work.pop().expect("the build");
            let top = matches!(finished.shape, Shape::Top);
            let value = assemble(finished);
            match work.last_mut() {
                Some(parent) if !top => parent.built.push(value),
                _ => return Ok(value),
            }
            continue;
        }
        let ty = dispatch::Types(vec![dispatch::copy(
            &build.parts[build.built.len()],
        )]);
        if let Some(compound) = compound_default(&ty.0[0], node)? {
            work.push(compound);
            continue;
        }
        if let ir::Type::Meaning { index, arguments } = &ty.0[0] {
            if let Some(call) = written_default(ctx, &ty.0[0], node)? {
                let value = calls::invoke(ctx, call, node)?;
                build.built.push(value);
                continue;
            }
            let prelude = &*ctx.prelude;
            let known = [
                prelude.integer,
                prelude.f32,
                prelude.f64,
                prelude.string,
                prelude.vec,
                prelude.deque,
                prelude.maybe,
                prelude.bool,
            ]
            .contains(&Some(*index));
            let tables = ctx.tables();
            let meaning = tables.meanings.get(*index).ok_or_else(|| {
                at(Reason::Message("invalid witness meaning".into()))
            })?;
            if meaning.resource {
                return Err(at(Reason::Message(
                    "native execution does not support resource values".into(),
                )));
            }
            if known || meaning.constructors.len() != 1 {
                let value = structural::default(&ty.0[0], &tables, prelude)
                    .map_err(at)?;
                build.built.push(value);
                continue;
            }
            let parts = meaning.constructors[0]
                .fields
                .iter()
                .map(|field| {
                    dispatch::arguments(
                        &field.ty,
                        &meaning.parameters,
                        arguments,
                    )
                })
                .collect();
            drop(tables);
            work.push(Build {
                shape: Shape::Record {
                    meaning: *index,
                    constructor: 0,
                },
                parts,
                built: Vec::new(),
            });
        } else {
            let value =
                structural::default(&ty.0[0], &ctx.tables(), &ctx.prelude)
                    .map_err(at)?;
            build.built.push(value);
        }
    }
}
