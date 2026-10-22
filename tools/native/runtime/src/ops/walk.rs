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
        ir::Derived::Default => default(ctx, ty, node),
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
) -> Result<calls::Invoke, Reason> {
    let Some((index, meanings, spot)) = written_row(ctx, member, receiver)
    else {
        return Err(Reason::Message(String::from(
            "no written member for the value",
        )));
    };
    let mut args = others;
    let spot = spot.min(args.len());
    args.insert(spot, receiver.clone());
    // The row is the receiver's tag's, by `written_row`.
    if let Value::Packed(packed) = receiver {
        let tag = ir::Type::Meaning {
            index: packed.meaning,
            arguments: Vec::new(),
        };
        let eq = member.is_some() && member == ctx.natives.members.eq;
        if let Err(equal) = dispatch::unpack(&mut args, spot, &tag, eq) {
            return Ok(calls::Invoke::Value(ctx.prelude.bool(equal)));
        }
    }
    Ok(calls::definition_invoke(ctx, index, args, meanings))
}

/// The written row of a member for a value's tag: the definition, its
/// meanings and the receiver's spot.
pub fn written_row(
    ctx: &Ctx,
    member: Option<usize>,
    receiver: &Value,
) -> Option<(usize, Vec<ir::Type>, usize)> {
    let member = member?;
    let tables = ctx.tables();
    let selected = dispatch::by_tag(&tables, &ctx.prelude, member, receiver)?;
    let row = &tables.dispatch[member].rows[selected.row];
    match &row.target {
        ir::Implementation::Definition { index, meanings } => {
            Some((*index, meanings.clone(), row.receiver_spot.unwrap_or(0)))
        }
        _ => None,
    }
}

/// A written member's answer for a receiver at `node`.
fn written(
    ctx: &Ctx,
    member: Option<usize>,
    receiver: &Value,
    others: Vec<Value>,
    node: usize,
) -> Result<Value, Faulted> {
    let call = member_call(ctx, member, receiver, others)
        .map_err(|reason| fail(node, reason))?;
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
        (Value::Packed(a), Value::Packed(b)) => (a.meaning == b.meaning)
            .then(|| vec![(a.value.clone(), b.value.clone())]),
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
        if let (Value::Record(_), Value::Record(_)) = (&a, &b) {
            if written_row(ctx, member, &a).is_some() {
                let answer = written(ctx, member, &a, vec![b], node)?;
                if !ctx.prelude.is_true(&answer) {
                    return Ok(ctx.prelude.bool(false));
                }
                continue;
            }
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
                match x.meaning.cmp(&y.meaning) {
                    Ordering::Equal => {
                        work.push(Order::Pair(
                            x.value.clone(),
                            y.value.clone(),
                        ));
                    }
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
        match &value {
            Value::Record(record) => {
                if written_row(ctx, member, &value).is_some() {
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
                work.push(Piece::Value(packed.value.clone(), is_argument));
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
fn assemble(build: Build) -> Value {
    let Build { shape, built, .. } = build;
    match shape {
        Shape::Top => built.into_iter().next().expect("one"),
        Shape::Tuple => Value::Tuple(Rc::new(built)),
        Shape::Array(length) => {
            let element = built.into_iter().next().expect("the element");
            Value::seq(vec![element; length])
        }
        Shape::Record {
            meaning,
            constructor,
        } => Value::Record(Rc::new(Record {
            meaning,
            constructor,
            fields: built,
        })),
    }
}

/// The written `default` row of a meaning: its definition and meanings.
fn written_default(
    ctx: &Ctx,
    meaning: usize,
) -> Option<(usize, Vec<ir::Type>)> {
    let member = ctx.natives.members.default?;
    let tables = ctx.tables();
    let selected = dispatch::select_by_head(&tables, member, meaning)?;
    match &tables.dispatch[member].rows[selected.row].target {
        ir::Implementation::Definition { index, meanings } => {
            Some((*index, meanings.clone()))
        }
        _ => None,
    }
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
        let ty = build.parts[build.built.len()].clone();
        match &ty {
            ir::Type::Tuple(parts) => work.push(Build {
                shape: Shape::Tuple,
                parts: parts.clone(),
                built: Vec::new(),
            }),
            ir::Type::Array { element, length } => {
                let ir::Type::Number(length) = &**length else {
                    return Err(at(Reason::Message(String::from(
                        "an array length is a number",
                    ))));
                };
                let length =
                    usize::try_from(length.to_u64().unwrap_or(0)).unwrap_or(0);
                work.push(Build {
                    shape: Shape::Array(length),
                    parts: vec![(**element).clone()],
                    built: Vec::new(),
                });
            }
            ir::Type::Meaning { index, arguments } => {
                if let Some((definition, meanings)) =
                    written_default(ctx, *index)
                {
                    let call = calls::definition_invoke(
                        ctx,
                        definition,
                        Vec::new(),
                        meanings,
                    );
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
                let meaning = &tables.meanings[*index];
                if known || meaning.constructors.len() != 1 {
                    let value = structural::default(&ty, &tables, prelude)
                        .map_err(at)?;
                    build.built.push(value);
                    continue;
                }
                let parts = meaning.constructors[0]
                    .fields
                    .iter()
                    .map(|field| {
                        field.ty.substituted(&meaning.parameters, arguments)
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
            }
            _ => {
                let value =
                    structural::default(&ty, &ctx.tables(), &ctx.prelude)
                        .map_err(at)?;
                build.built.push(value);
            }
        }
    }
}
