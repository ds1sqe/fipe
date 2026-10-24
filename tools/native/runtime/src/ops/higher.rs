//! The natives that call back into the program, rewritten as plain
//! recursion from the VM's native frames (`src/vm/pending.rs`: `map`,
//! `fold`, `filter`, `sum`; `src/vm/order.rs`: `sort`, `count`): each
//! calls machine code for a value where the VM's frame waits on one, in
//! the VM's order, so the calls, their charges and their Faults are the
//! VM's. The drift fixtures run each against the VM.
use super::{
    calls::{self, call_value},
    dispatch, fail,
    immediate::{is_float, leaf, seq},
    natives::{of_type, Of},
    walk, Ctx, Faulted,
};
use fipe::{
    ir,
    value::{arith, float, structural, Reason, Record, Value},
};
use std::{cmp::Ordering, rc::Rc};

/// `map` over a sequence, a `Maybe` or a `Result`: the function of each
/// item, the outcome rebuilt around the one it holds.
pub fn map(
    ctx: &Ctx,
    function: &Value,
    over: Value,
    node: usize,
) -> Result<Value, Faulted> {
    let apply =
        |item: &Value| call_value(ctx, function, vec![item.clone()], node);
    match &over {
        Value::Seq(items) => {
            let out = items.iter().map(apply).collect::<Result<_, _>>()?;
            Ok(Value::seq(out))
        }
        Value::Record(record) if record.constructor == 0 => {
            let value = apply(&record.fields[0])?;
            Ok(Value::Record(Rc::new(Record {
                meaning: record.meaning,
                constructor: 0,
                fields: vec![value],
            })))
        }
        _ => Ok(over),
    }
}

/// `fold`: the function of the accumulator and each item, in order.
pub fn fold(
    ctx: &Ctx,
    init: Value,
    function: &Value,
    over: &Value,
    node: usize,
) -> Result<Value, Faulted> {
    seq(over).iter().try_fold(init, |acc, item| {
        call_value(ctx, function, vec![acc, item.clone()], node)
    })
}

/// `filter`: the items the function keeps, in order.
pub fn filter(
    ctx: &Ctx,
    function: &Value,
    over: &Value,
    node: usize,
) -> Result<Value, Faulted> {
    let mut out = Vec::new();
    for item in seq(over).iter() {
        let kept = call_value(ctx, function, vec![item.clone()], node)?;
        if ctx.prelude.is_true(&kept) {
            out.push(item.clone());
        }
    }
    Ok(Value::seq(out))
}

/// `sum`: the items added in order, at once for numbers, by the written
/// `add` row for a record; the empty sum is the zero of the call's
/// result meaning.
pub fn sum(
    ctx: &Ctx,
    over: &Value,
    result: Option<&ir::Type>,
    node: usize,
) -> Result<Value, Faulted> {
    let at = |reason| fail(node, reason);
    let items = seq(over);
    if items.is_empty() {
        let of = result.map(|ty| of_type(ty, &ctx.tables()));
        return match of {
            Some(Of::F32) => Ok(Value::F32(0.0)),
            Some(Of::F64) => Ok(Value::F64(0.0)),
            Some(Of::Integer) | None => Ok(arith::zero()),
            Some(_) => {
                let Some(ir::Type::Meaning { .. }) = result else {
                    unreachable!()
                };
                zero_of(ctx, result.expect("sum result"), node)
            }
        };
    }
    let mut acc = items[0].clone();
    for item in &items[1..] {
        acc = if is_float(&acc) {
            float::add(&acc, item).map_err(at)?
        } else if leaf(&acc) {
            arith::add(&acc, item)
        } else {
            let member = ctx.natives.members.add;
            let call = walk::member_call(
                ctx,
                member,
                &acc,
                vec![item.clone()],
                result,
                node,
            )?;
            calls::invoke(ctx, call, node)?
        };
    }
    Ok(acc)
}

/// The written `zero` of a meaning, for an empty sum of its values.
fn zero_of(ctx: &Ctx, ty: &ir::Type, node: usize) -> Result<Value, Faulted> {
    let missing = || fail(node, Reason::Message(String::from("no zero")));
    let member = ctx.natives.members.zero.ok_or_else(missing)?;
    let tables = ctx.tables();
    let selected =
        dispatch::select_for(&tables, &ctx.prelude, member, Some(ty), None)
            .ok_or_else(missing)?;
    let call = calls::supplied_target(
        ctx,
        &tables.dispatch[member].rows[selected.row].target,
        &selected.bindings,
        Vec::new(),
        Some(ty),
        node,
    )?
    .ok_or_else(missing)?;
    calls::invoke(ctx, call, node)
}

/// Whether two values compare without a call: both leaves, or no
/// written member of this name in the unit.
const fn plain(member: Option<usize>, a: &Value, b: &Value) -> bool {
    member.is_none() || (leaf(a) && leaf(b))
}

/// `sort`: the keys first, by the `by` function, then a stable
/// bottom-up merge of the positions, the right key compared against the
/// left; the right goes first only when it strictly precedes.
pub fn sort(
    ctx: &Ctx,
    by: &Value,
    desc: bool,
    over: &Value,
    node: usize,
) -> Result<Value, Faulted> {
    let items = seq(over);
    let count = items.len();
    let keys = items
        .iter()
        .map(|item| call_value(ctx, by, vec![item.clone()], node))
        .collect::<Result<Vec<_>, _>>()?;
    let wanted = if desc {
        Ordering::Greater
    } else {
        Ordering::Less
    };
    let member = ctx.natives.members.cmp;
    let mut order: Vec<usize> = (0..count).collect();
    let mut width = 1;
    while width < count {
        let mut out = Vec::with_capacity(count);
        for start in (0..count).step_by(2 * width) {
            let mid = (start + width).min(count);
            let end = (start + 2 * width).min(count);
            let (mut i, mut j) = (start, mid);
            while i < mid && j < end {
                let (right, left) = (&keys[order[j]], &keys[order[i]]);
                let ordering = if plain(member, right, left) {
                    structural::cmp(right, left)
                } else {
                    let answer =
                        walk::cmp(ctx, right.clone(), left.clone(), node)?;
                    walk::ordering_of(&answer)
                };
                if ordering == wanted {
                    out.push(order[j]);
                    j += 1;
                } else {
                    out.push(order[i]);
                    i += 1;
                }
            }
            out.extend(&order[i..mid]);
            out.extend(&order[j..end]);
        }
        order = out;
        width *= 2;
    }
    Ok(Value::seq(
        order.iter().map(|index| items[*index].clone()).collect(),
    ))
}

/// `count`: each distinct item in its first order with how often it
/// occurs, each item compared by `eq` with the distinct ones in order.
pub fn count(ctx: &Ctx, over: &Value, node: usize) -> Result<Value, Faulted> {
    let member = ctx.natives.members.eq;
    let mut distinct: Vec<(Value, usize)> = Vec::new();
    for item in seq(over).iter() {
        let mut probe = 0;
        loop {
            let Some((known, n)) = distinct.get_mut(probe) else {
                distinct.push((item.clone(), 1));
                break;
            };
            let equal = if plain(member, item, known) {
                structural::eq(item, known)
            } else {
                let answer = walk::eq(ctx, item.clone(), known.clone(), node)?;
                ctx.prelude.is_true(&answer)
            };
            if equal {
                *n += 1;
                break;
            }
            probe += 1;
        }
    }
    let counts = distinct
        .into_iter()
        .map(|(item, n)| ctx.prelude.count(item, n))
        .collect();
    Ok(Value::seq(counts))
}
