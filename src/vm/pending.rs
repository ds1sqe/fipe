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
use alloc::{rc::Rc, vec::Vec};

/// A lazy immutable reference to one creator-owned guard.
#[derive(Clone, Debug)]
pub(super) struct GuardJob {
    pub(super) owner: Rc<crate::value::Closure>,
    pub(super) index: usize,
    pub(super) spot: usize,
}
impl GuardJob {
    pub(super) fn guard(&self) -> &crate::value::Guard {
        &self.owner.guards[self.index]
    }
}
#[derive(Default, Debug)]
pub(super) struct GuardJobs {
    owner: Option<Rc<crate::value::Closure>>,
    indices: alloc::vec::IntoIter<(usize, usize)>,
}
impl GuardJobs {
    pub(super) fn select(
        owner: &Rc<crate::value::Closure>,
        work: &super::resource::Work,
        mut select: impl FnMut(&crate::value::Guard) -> Option<usize>,
    ) -> Result<Self, Reason> {
        work.pay(u64::try_from(owner.guards.len()).unwrap_or(u64::MAX))?;
        let mut indices = Vec::new();
        for (index, guard) in owner.guards.iter().enumerate() {
            if let Some(spot) = select(guard) {
                work.pay(1)?;
                indices.push((index, spot));
            }
        }
        let retained = if indices.is_empty() {
            None
        } else {
            work.pay(1)?;
            Some(owner.clone())
        };
        Ok(Self {
            owner: retained,
            indices: indices.into_iter(),
        })
    }
    pub(super) fn is_empty(&self) -> bool {
        self.indices.len() == 0
    }
    pub(super) fn next(&mut self) -> Option<GuardJob> {
        self.indices.next().map(|(index, spot)| GuardJob {
            owner: self.owner.as_ref().expect("guard owner").clone(),
            index,
            spot,
        })
    }
}

/// A native in progress, waiting on a call.
#[derive(Debug)]
pub(super) enum Pending {
    /// Eval/apply: the arguments left after the body returned a function,
    /// until they are applied; then the result passes through. The guards
    /// past the body's own parameters move to the function it returned.
    Apply {
        rest: Option<Vec<Value>>,
        guards: GuardJobs,
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
    Arguments(Arguments),
    Sort(super::order::Sort),
    Count(super::order::Count),
    /// A native that hands its arguments to one written member: the
    /// call until made, then its answer passes through.
    Once(Option<Invoke>),
}

/// Ordered guards retain the current value of every spot between tests.
#[derive(Debug)]
pub(super) struct Arguments {
    values: Vec<Value>,
    guards: GuardJobs,
    waiting: Option<(usize, bool)>,
    completed: Option<Value>,
    finish: Option<ArgumentsFinish>,
}

#[derive(Debug)]
enum ArgumentsFinish {
    Tuple,
    Partial(Rc<crate::value::Closure>),
}

impl Arguments {
    pub(super) const fn new(values: Vec<Value>, guards: GuardJobs) -> Self {
        Self {
            values,
            guards,
            waiting: None,
            completed: None,
            finish: Some(ArgumentsFinish::Tuple),
        }
    }

    pub(super) fn partial(
        mut closure: Rc<crate::value::Closure>,
        guards: GuardJobs,
    ) -> Result<Self, Reason> {
        let private = Rc::get_mut(&mut closure)
            .ok_or_else(|| Reason::Host("shared private partial".into()))?;
        let values = core::mem::take(&mut private.filled);
        Ok(Self {
            values,
            guards,
            waiting: None,
            completed: None,
            finish: Some(ArgumentsFinish::Partial(closure)),
        })
    }

    fn step(&mut self, returned: Option<Value>) -> Step {
        let returned = returned.or_else(|| self.completed.take());
        if let Some((spot, function)) = self.waiting.take() {
            let Some(value) = returned else {
                return Step::Fault(Reason::Host(
                    "missing argument completion".into(),
                ));
            };
            if function {
                let Some(current) = self.values.get_mut(spot) else {
                    return Step::Fault(Reason::Host(
                        "missing guarded argument".into(),
                    ));
                };
                *current = value;
            } else if !matches!(value, Value::Unit) {
                return Step::Fault(Reason::Host(
                    "invalid predicate completion".into(),
                ));
            }
        } else if returned.is_some() {
            return Step::Fault(Reason::Host(
                "unexpected argument completion".into(),
            ));
        }
        if let Some(job) = self.guards.next() {
            let spot = job.spot;
            let guard = job.guard();
            let Some(value) = self.values.get(spot) else {
                return Step::Fault(Reason::Host(
                    "missing guarded argument".into(),
                ));
            };
            let function =
                matches!(&guard.predicate, ir::Predicate::Function { .. })
                    && matches!(value, Value::Function(_));
            self.waiting = Some((spot, function));
            return Step::AdmitArgument {
                value: value.clone(),
                guard: job,
            };
        }
        match self.finish.take() {
            Some(ArgumentsFinish::Tuple) => Step::Done(Value::Tuple(Rc::new(
                core::mem::take(&mut self.values),
            ))),
            Some(ArgumentsFinish::Partial(mut closure)) => {
                let Some(private) = Rc::get_mut(&mut closure) else {
                    return Step::Fault(Reason::Host(
                        "shared private partial".into(),
                    ));
                };
                private.filled = core::mem::take(&mut self.values);
                Step::Done(Value::Function(closure))
            }
            None => {
                Step::Fault(Reason::Host("finished argument sequence".into()))
            }
        }
    }
}

impl Pending {
    pub(super) fn argument_returned(
        &mut self,
        value: Value,
    ) -> Result<(), Reason> {
        let Self::Arguments(arguments) = self else {
            return Err(Reason::Host("missing argument sequence".into()));
        };
        if !matches!(arguments.waiting, Some((_, true)))
            || arguments.completed.is_some()
        {
            return Err(Reason::Host(
                "unexpected function argument completion".into(),
            ));
        }
        arguments.completed = Some(value);
        Ok(())
    }
}

impl Pending {
    /// A native's step as the frame it becomes: a call starts one.
    pub(super) fn started(step: Step) -> Step {
        match step {
            Step::Call(invoke) => Step::Start(Self::Once(Some(invoke))),
            Step::AdmitArgument { .. } => unreachable!(
                "argument guards start only in their private queue"
            ),
            other => other,
        }
    }
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

    /// One move: with the value a call returned, or none to begin.
    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        match self {
            Self::Apply { rest, guards } => {
                apply_step(rest, guards, ctx, returned)
            }
            Self::Once(call) => match (call.take(), returned) {
                (Some(invoke), None) => Step::Call(invoke),
                (None, Some(value)) => Step::Done(value),
                _ => unreachable!("a once calls once"),
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
            Self::Arguments(arguments) => arguments.step(returned),
            Self::Sort(sort) => sort.step(ctx, returned),
            Self::Count(count) => count.step(ctx, returned),
        }
    }
}

fn apply_step(
    rest: &mut Option<Vec<Value>>,
    guards: &mut GuardJobs,
    ctx: &Ctx<'_>,
    returned: Option<Value>,
) -> Step {
    let returned = match returned {
        Some(Value::Function(closure)) if !guards.is_empty() => {
            let mut closure = match super::resource::copy_guarded_closure(
                &closure,
                ctx.program,
                ctx.tables,
                ctx.work,
            ) {
                Ok(closure) => closure,
                Err(reason) => return Step::Fault(reason),
            };
            let filled = closure.filled.len();
            let mut copied = Vec::new();
            while let Some(job) = guards.next() {
                let mut guard = match super::resource::copy_guard(
                    job.guard(),
                    ctx.tables,
                    ctx.work,
                ) {
                    Ok(guard) => guard,
                    Err(reason) => return Step::Fault(reason),
                };
                guard.spot = match job.spot.checked_add(filled) {
                    Some(spot) => spot,
                    None => {
                        return Step::Fault(Reason::Host(
                            "guard spot overflow".into(),
                        ))
                    }
                };
                copied.push(guard);
            }
            closure.guards.extend(copied);
            Some(Value::Function(Rc::new(closure)))
        }
        other => other,
    };
    match (rest.take(), returned) {
        (Some(rest), Some(function)) => {
            Step::Call(Invoke::Function(function, rest))
        }
        (None, Some(value)) => Step::Done(value),
        (_, None) => unreachable!("an apply waits on the body"),
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
        return Step::Fault(Reason::Message("no zero".into()));
    };
    let fallback = ir::Type::Meaning {
        index: meaning,
        arguments: Vec::new(),
    };
    let ty = ctx.result.unwrap_or(&fallback);
    let selected = match super::dispatch::select_for(
        ctx.tables,
        ctx.prelude,
        member,
        Some(ty),
        None,
        ctx.work,
    ) {
        Ok(Some(selected)) => selected,
        Ok(None) => return Step::Fault(Reason::Message("no zero".into())),
        Err(reason) => return Step::Fault(reason),
    };
    match supplied_target(
        ctx,
        &ctx.tables.dispatch[member].rows[selected.row].target,
        &selected.bindings,
        Vec::new(),
        ctx.result,
    ) {
        Ok(Some(invoke)) => Step::Call(invoke),
        Ok(None) => Step::Fault(Reason::Message("no zero".into())),
        Err(reason) => Step::Fault(reason),
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
    match member_invoke(ctx, member, receiver, others) {
        Ok(Some(invoke)) => Step::Call(invoke),
        Ok(None) => Step::Fault(Reason::Message(
            "no written member for the value".into(),
        )),
        Err(reason) => Step::Fault(reason),
    }
}
fn member_invoke(
    ctx: &Ctx<'_>,
    member: Option<usize>,
    receiver: &Value,
    mut args: Vec<Value>,
) -> Result<Option<Invoke>, Reason> {
    let Some(member) = member else {
        return Ok(None);
    };
    let Some(selected) = super::dispatch::by_tag(
        ctx.tables,
        ctx.prelude,
        member,
        receiver,
        ctx.work,
    )?
    else {
        return Ok(None);
    };
    let row = &ctx.tables.dispatch[member].rows[selected.row];
    let spot = row.receiver_spot.unwrap_or(0).min(args.len());
    args.insert(spot, receiver.clone());
    let eq = Some(member) == ctx.natives.members.eq;
    let cmp = Some(member) == ctx.natives.members.cmp;
    if !super::dispatch::unpack(
        &mut args,
        spot,
        row,
        &selected.bindings,
        (eq, cmp),
        ctx.tables,
        ctx.work,
    )? {
        return Ok(Some(Invoke::Value(ctx.prelude.bool(false))));
    }
    // These are the complete callers: comparison/show walks, Len, and Sum Add.
    // A sort/count walk has no enclosing result, so its fixed scalar is explicit.
    let scalar = if eq {
        Some(ctx.prelude.bool)
    } else if cmp {
        Some(ctx.prelude.ordering)
    } else if Some(member) == ctx.natives.members.show {
        Some(ctx.prelude.string)
    } else if Some(member) == ctx.natives.members.len && ctx.result.is_none() {
        Some(ctx.prelude.integer)
    } else {
        None
    };
    let scalar = scalar
        .map(|identity| {
            identity.ok_or_else(|| {
                Reason::Message("missing supplied member result meaning".into())
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
        ctx.result
    };
    supplied_target(ctx, &row.target, &selected.bindings, args, result)
}

/// Builtin declarations retain intrinsic fallbacks, never recursive interception.
fn supplied(target: &ir::Implementation, ctx: &Ctx<'_>) -> bool {
    matches!(target, ir::Implementation::Definition { .. })
        || matches!(target,ir::Implementation::Declared(op) if ctx.natives.get(*op).is_none())
}
pub(super) fn written_presence(
    ctx: &Ctx<'_>,
    member: Option<usize>,
    receiver: &Value,
) -> Result<Option<()>, Reason> {
    let Some(member) = member else {
        return Ok(None);
    };
    let Some(selected) = super::dispatch::by_tag(
        ctx.tables,
        ctx.prelude,
        member,
        receiver,
        ctx.work,
    )?
    else {
        return Ok(None);
    };
    Ok(
        supplied(&ctx.tables.dispatch[member].rows[selected.row].target, ctx)
            .then_some(()),
    )
}

/// Invoke a checked row using its caller's actual result, not cached Declaration.ty.
pub(super) fn supplied_target(
    ctx: &Ctx<'_>,
    target: &ir::Implementation,
    bindings: &[(usize, ir::Type)],
    args: Vec<Value>,
    result: Option<&ir::Type>,
) -> Result<Option<Invoke>, Reason> {
    match target {
        ir::Implementation::Definition { index, meanings } => {
            let mut copied = crate::value::resource::Forest::default();
            for ty in meanings {
                copied.0.push(super::resource::substitute_bindings(
                    ty, bindings, ctx.tables, ctx.work,
                )?);
            }
            Ok(Some(super::calls::definition_invoke(
                ctx.program,
                ctx.globals,
                *index,
                args,
                core::mem::take(&mut copied.0),
            )))
        }
        ir::Implementation::Declared(op) if ctx.natives.get(*op).is_none() => {
            let result = result.ok_or_else(|| {
                Reason::Message("missing actual supplied member result".into())
            })?;
            let result = super::resource::materialize(
                result,
                &[],
                &[],
                ctx.tables,
                ctx.work,
                &mut Ok,
            )?;
            Ok(Some(Invoke::Declared {
                declaration: *op,
                args,
                result,
            }))
        }
        _ => Ok(None),
    }
}
