//! Natives in progress: the states of native frames, moved one call
//! at a time, and the lookup of a written row by a value's tag.
use super::{
    calls::Invoke,
    immediate::{is_float, seq},
    natives::of_type,
    natives::{Ctx, Of, Step},
    predicates::Run,
    walk::Walk,
};
use crate::{
    ir,
    value::{arith, float, Reason, Value},
};
use alloc::{rc::Rc, string::String, vec::Vec};

/// A native in progress, waiting on a call.
#[derive(Debug)]
pub(super) enum Pending {
    /// Eval/apply: the arguments left after the body returned a function,
    /// until they are applied; then the result passes through.
    Apply {
        rest: Option<Vec<Value>>,
    },
    Map(Map),
    Fold(Fold),
    Filter(Filter),
    /// `then`: the function and the payload until called; then the
    /// function's result is the answer.
    Then(Option<(Value, Value)>),
    Sum(Sum),
    Walk(Walk),
    Predicate(Run),
    Sort(super::order::Sort),
    Count(super::order::Count),
}

/// `map` over a sequence, a `Maybe` or a `Result`.
#[derive(Debug)]
pub(super) struct Map {
    function: Value,
    items: Rc<Vec<Value>>,
    index: usize,
    out: Vec<Value>,
    /// Rebuild the outcome around the result, by meaning and constructor.
    wrap: Option<(usize, usize)>,
}

#[derive(Debug)]
pub(super) struct Fold {
    function: Value,
    items: Rc<Vec<Value>>,
    index: usize,
    acc: Value,
}

#[derive(Debug)]
pub(super) struct Filter {
    function: Value,
    items: Rc<Vec<Value>>,
    index: usize,
    current: Option<Value>,
    out: Vec<Value>,
}

#[derive(Debug)]
pub(super) struct Sum {
    items: Rc<Vec<Value>>,
    index: usize,
    acc: Option<Value>,
}

impl Pending {
    pub(super) fn map(function: Value, over: Value) -> Step {
        match &over {
            Value::Seq(parts) => Step::Start(Self::Map(Map {
                function,
                items: Rc::clone(parts),
                index: 0,
                out: Vec::with_capacity(parts.len()),
                wrap: None,
            })),
            Value::Record(record) if record.constructor == 0 => {
                Step::Start(Self::Map(Map {
                    function,
                    items: Rc::new(alloc::vec![record.fields[0].clone()]),
                    index: 0,
                    out: Vec::with_capacity(1),
                    wrap: Some((record.meaning, 0)),
                }))
            }
            _ => Step::Done(over),
        }
    }

    pub(super) fn fold(init: Value, function: Value, over: &Value) -> Step {
        Step::Start(Self::Fold(Fold {
            function,
            items: Rc::clone(seq(over)),
            index: 0,
            acc: init,
        }))
    }

    pub(super) fn filter(function: Value, over: &Value) -> Step {
        let items = Rc::clone(seq(over));
        Step::Start(Self::Filter(Filter {
            function,
            out: Vec::with_capacity(items.len()),
            items,
            index: 0,
            current: None,
        }))
    }

    pub(super) fn sum(over: &Value) -> Step {
        let items = Rc::clone(seq(over));
        Step::Start(Self::Sum(Sum {
            items,
            index: 0,
            acc: None,
        }))
    }

    /// Units the native's own moves cost beyond its start, owed since
    /// the machine last paid.
    pub(super) const fn take_owed(&mut self) -> u64 {
        match self {
            Self::Predicate(run) => run.take_owed(),
            _ => 0,
        }
    }

    /// One move: with the value a call returned, or none to begin.
    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        match self {
            Self::Apply { rest } => match (rest.take(), returned) {
                (Some(rest), Some(function)) => {
                    Step::Call(Invoke::Function(function, rest))
                }
                (None, Some(value)) => Step::Done(value),
                (_, None) => unreachable!("an apply waits on the body"),
            },
            Self::Then(call) => match (call.take(), returned) {
                (Some((function, payload)), None) => {
                    Step::Call(Invoke::Function(function, alloc::vec![payload]))
                }
                (None, Some(value)) => Step::Done(value),
                _ => unreachable!("a then calls once"),
            },
            Self::Map(map) => {
                if let Some(value) = returned {
                    map.out.push(value);
                }
                if map.index < map.items.len() {
                    let item = map.items[map.index].clone();
                    map.index += 1;
                    return Step::Call(Invoke::Function(
                        map.function.clone(),
                        alloc::vec![item],
                    ));
                }
                let out = core::mem::take(&mut map.out);
                Step::Done(match map.wrap {
                    None => Value::seq(out),
                    Some((meaning, constructor)) => {
                        Value::Record(Rc::new(crate::value::Record {
                            meaning,
                            constructor,
                            fields: out,
                        }))
                    }
                })
            }
            Self::Fold(fold) => {
                if let Some(value) = returned {
                    fold.acc = value;
                }
                if fold.index < fold.items.len() {
                    let item = fold.items[fold.index].clone();
                    fold.index += 1;
                    return Step::Call(Invoke::Function(
                        fold.function.clone(),
                        alloc::vec![fold.acc.clone(), item],
                    ));
                }
                Step::Done(core::mem::replace(&mut fold.acc, Value::Unit))
            }
            Self::Filter(filter) => {
                if let Some(value) = returned {
                    let kept = filter.current.take().expect("the tested item");
                    if ctx.prelude.is_true(&value) {
                        filter.out.push(kept);
                    }
                }
                if filter.index < filter.items.len() {
                    let item = filter.items[filter.index].clone();
                    filter.index += 1;
                    filter.current = Some(item.clone());
                    return Step::Call(Invoke::Function(
                        filter.function.clone(),
                        alloc::vec![item],
                    ));
                }
                Step::Done(Value::seq(core::mem::take(&mut filter.out)))
            }
            Self::Sum(sum) => sum.step(ctx, returned),
            Self::Walk(walk) => walk.step(ctx, returned),
            Self::Predicate(run) => run.step(ctx, returned),
            Self::Sort(sort) => sort.step(ctx, returned),
            Self::Count(count) => count.step(ctx, returned),
        }
    }
}

impl Sum {
    /// Add the next item to the accumulator: at once for numbers, by
    /// the written `add` row for a record; the empty sum is the zero of
    /// the call's result meaning.
    fn step(&mut self, ctx: &Ctx<'_>, returned: Option<Value>) -> Step {
        if let Some(value) = returned {
            self.acc = Some(value);
        }
        if self.acc.is_none() && self.items.is_empty() {
            return match ctx.result.as_ref().map(|ty| of_type(ty, ctx.tables)) {
                Some(Of::F32) => Step::Done(Value::F32(0.0)),
                Some(Of::F64) => Step::Done(Value::F64(0.0)),
                Some(Of::Integer) | None => Step::Done(arith::zero()),
                Some(_) => {
                    let Some(ir::Type::Meaning { index, .. }) = &ctx.result
                    else {
                        unreachable!()
                    };
                    zero_of(*index, ctx)
                }
            };
        }
        while self.index < self.items.len() {
            let item = self.items[self.index].clone();
            self.index += 1;
            let Some(acc) = self.acc.take() else {
                self.acc = Some(item);
                continue;
            };
            if is_float(&acc) {
                match float::add(&acc, &item) {
                    Ok(value) => self.acc = Some(value),
                    Err(reason) => return Step::Fault(reason),
                }
            } else if super::step::leaf(&acc) {
                self.acc = Some(arith::add(&acc, &item));
            } else {
                return member_call(
                    ctx,
                    ctx.natives.members.add,
                    &acc,
                    alloc::vec![item],
                );
            }
        }
        Step::Done(self.acc.take().expect("a sum"))
    }
}

/// The written `zero` of a meaning, for an empty sum of its values.
fn zero_of(meaning: usize, ctx: &Ctx<'_>) -> Step {
    let Some(member) = ctx.natives.members.zero else {
        return Step::Fault(Reason::Message(String::from("no zero")));
    };
    let Some(selected) =
        crate::compile::dispatch::select_by_head(ctx.tables, member, meaning)
    else {
        return Step::Fault(Reason::Message(String::from("no zero")));
    };
    match &ctx.tables.dispatch[member].rows[selected.row].target {
        ir::Implementation::Definition { index, meanings } => {
            Step::Call(super::calls::definition_invoke(
                ctx.program,
                ctx.globals,
                *index,
                Vec::new(),
                meanings.clone(),
            ))
        }
        _ => Step::Fault(Reason::Message(String::from("no zero"))),
    }
}

/// Call a member's written row on a receiver by its tag, the other
/// arguments before or after it as the row's receiver spot says; the
/// prelude's own rows and a derived row give `None`.
pub(super) fn member_call(
    ctx: &Ctx<'_>,
    member: Option<usize>,
    receiver: &Value,
    others: Vec<Value>,
) -> Step {
    match written_row(ctx, member, receiver) {
        Some((index, meanings, spot)) => {
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
                if let Err(equal) =
                    super::dispatch::unpack(&mut args, spot, &tag, eq)
                {
                    return Step::Done(ctx.prelude.bool(equal));
                }
            }
            Step::Call(super::calls::definition_invoke(
                ctx.program,
                ctx.globals,
                index,
                args,
                meanings,
            ))
        }
        None => Step::Fault(Reason::Message(String::from(
            "no written member for the value",
        ))),
    }
}

/// The written row of a member for a value's tag: the definition, its
/// meanings and the receiver's spot.
pub(super) fn written_row(
    ctx: &Ctx<'_>,
    member: Option<usize>,
    receiver: &Value,
) -> Option<(usize, Vec<ir::Type>, usize)> {
    let member = member?;
    let selected =
        super::dispatch::by_tag(ctx.tables, ctx.prelude, member, receiver)?;
    let row = &ctx.tables.dispatch[member].rows[selected.row];
    match &row.target {
        ir::Implementation::Definition { index, meanings } => {
            Some((*index, meanings.clone(), row.receiver_spot.unwrap_or(0)))
        }
        _ => None,
    }
}
