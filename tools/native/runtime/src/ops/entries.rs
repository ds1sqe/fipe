//! Entries at run time, copied from `src/vm/entries.rs` and the `Enter`
//! statement of `src/vm/step.rs`: an entry tests each component its
//! path reaches from the value, in order, by the entry's predicate; a
//! function value meeting a function entry in a local is not tested but
//! rebuilt with guards, which each of its calls tests.
use super::{predicates, Ctx, Faulted, Frame, Site};
use fipe::{
    ir::{self, Projection},
    mir::{Operand, Statement},
    value::{Closure, Guard, Value},
};
use std::rc::Rc;

/// The site's `Enter` of `value`: the guarded function to put in its
/// local's place, or `None` once the entry's tests passed.
///
/// # Errors
///
/// When a component fails the entry, or a test Faults.
pub fn enter(
    site: Site,
    value: Value,
    hidden: &[Value],
) -> Result<Option<Value>, Faulted> {
    let ctx = Ctx::get();
    let node = ctx.node(site);
    let block = &ctx.program.bodies[site.body].blocks[site.block];
    let Statement::Enter {
        entry,
        value: operand,
    } = &block.statements[site.index]
    else {
        unreachable!("an enter site")
    };
    let entry = *entry;
    let frame = Frame::new(site.body, hidden);
    // Function attachment precedes the ordinary entry snapshot, as in Core.
    if let (Value::Function(closure), Operand::Local(_)) = (&value, operand) {
        let tables = ctx.tables();
        let predicate = &tables.entries[entry].predicate;
        if matches!(predicate, ir::Predicate::Function { .. }) {
            return Ok(Some(guard(
                &ctx, &frame, closure, entry, predicate, node,
            )?));
        }
    }
    let (snapshot, path) = {
        let tables = ctx.tables();
        let source = &tables.entries[entry];
        (
            predicates::copy_predicate(&ctx, &source.predicate, node)?,
            source.path.clone(),
        )
    };
    let predicate = &snapshot.0[0];
    let components = components(value, &path);
    if !components.is_empty() {
        let witnesses = predicates::capture_predicate(
            predicate,
            false,
            &ctx.program.bodies[site.body].hidden,
            &frame.meanings,
            &ctx,
            node,
        )?;
        predicates::enter_each(
            &ctx, predicate, entry, components, node, &witnesses,
        )?;
    }
    Ok(None)
}

/// An open function entry: the closure keeps the entry's parameter
/// limits, tested on each of its calls (07-checks §What is known 8).
fn guard(
    ctx: &Ctx,
    frame: &Frame,
    closure: &Closure,
    entry: usize,
    predicate: &ir::Predicate,
    node: usize,
) -> Result<Value, Faulted> {
    let witnesses = predicates::capture_predicate(
        predicate,
        true,
        &ctx.program.bodies[frame.body].hidden,
        &frame.meanings,
        ctx,
        node,
    )?;
    guard_with_witnesses(ctx, closure, entry, predicate, node, &witnesses)
}

/// Copy a function and append guards using its supplied witness context.
pub(super) fn guard_with_witnesses(
    ctx: &Ctx,
    closure: &Closure,
    entry: usize,
    predicate: &ir::Predicate,
    node: usize,
    witnesses: &predicates::Witnesses,
) -> Result<Value, Faulted> {
    let mut closure = super::calls::copied_guarded_closure(ctx, closure, node)?;
    append_guard_parameters(
        ctx,
        &mut closure,
        entry,
        predicate,
        node,
        witnesses,
    )?;
    Ok(Value::Function(Rc::new(closure)))
}

/// Admit one argument using only the guard creator's retained witnesses.
pub(super) fn admit_argument(
    ctx: &Ctx,
    guard: &Guard,
    value: Value,
    node: usize,
) -> Result<Value, Faulted> {
    if let (Value::Function(closure), ir::Predicate::Function { .. }) =
        (&value, &guard.predicate)
    {
        return guard_with_witnesses(
            ctx,
            closure,
            guard.entry,
            &guard.predicate,
            node,
            &guard.witnesses,
        );
    }
    predicates::enter_by(
        ctx,
        &guard.predicate,
        guard.entry,
        value.clone(),
        node,
        &guard.witnesses,
    )?;
    Ok(value)
}

/// Guard a fresh receiver adapter in its own resolved witness context.
pub(super) fn receiver_closure(
    ctx: &Ctx,
    mut closure: Closure,
    node: usize,
) -> Result<Value, Faulted> {
    let entry = ctx
        .program
        .receiver_contexts
        .get(closure.body)
        .and_then(Option::as_ref)
        .and_then(|context| context.input_contract);
    if let Some(entry) = entry {
        debug_assert!(closure.filled.is_empty());
        let tables = ctx.tables();
        let predicate = &tables.entries[entry].predicate;
        let witnesses = predicates::capture_predicate_types(
            predicate,
            true,
            &ctx.program.bodies[closure.body].hidden,
            &closure.meanings,
            ctx,
            node,
        )?;
        append_guard_parameters(
            ctx,
            &mut closure,
            entry,
            predicate,
            node,
            &witnesses,
        )?;
    }
    Ok(Value::Function(Rc::new(closure)))
}

/// The values a path reaches from a value, in order: every element, a
/// field or part, a payload or a case's field only when the value has
/// that constructor.
fn components(value: Value, path: &[Projection]) -> Vec<Value> {
    let mut reached = vec![value];
    for projection in path {
        reached = reached
            .iter()
            .flat_map(|value| step(value, projection))
            .collect();
    }
    reached
}

fn step(value: &Value, projection: &Projection) -> Vec<Value> {
    match (projection, value) {
        (Projection::Element, Value::Seq(items)) => items.to_vec(),
        (Projection::Field(index), Value::Record(record)) => {
            vec![record.fields[*index].clone()]
        }
        (Projection::Part(index), Value::Tuple(parts)) => {
            vec![parts[*index].clone()]
        }
        (
            Projection::CaseField { constructor, field },
            Value::Record(record),
        ) if record.constructor == *constructor => {
            vec![record.fields[*field].clone()]
        }
        // The prelude's sides in declaration order: a payload is visited
        // on the constructor of its index.
        (Projection::Payload(index), Value::Record(record))
            if record.constructor == *index =>
        {
            vec![record.fields[0].clone()]
        }
        (Projection::CaseField { .. } | Projection::Payload(_), _) => {
            Vec::new()
        }
        (projection, value) => {
            unreachable!("{projection:?} of {value:?}")
        }
    }
}

/// Admit one binding on a private copied wrapper path before its next source operand.
pub(super) fn admitted_binding(
    ctx: &Ctx,
    callee: Value,
    spot: usize,
    mut argument: Value,
    node: usize,
) -> Result<Value, Faulted> {
    use super::{fail, pay};
    use fipe::value::Reason;
    let mut current = callee.clone();
    let mut relative = spot;
    let mut path: Vec<(Rc<Closure>, usize)> = Vec::new();
    let (closure, position, entry) = loop {
        pay(1, node)?;
        let Value::Function(closure) = current else {
            return Ok(Value::Tuple(Rc::new(vec![callee, argument])));
        };
        let body = ctx.program.bodies.get(closure.body).ok_or_else(|| {
            fail(node, Reason::Host("missing binding body".into()))
        })?;
        let position =
            relative.checked_add(closure.filled.len()).ok_or_else(|| {
                fail(node, Reason::Host("binding spot overflow".into()))
            })?;
        // Beyond this raw body, application belongs to its result.
        // Its result callable is not available at this binding boundary.
        if position >= body.arity {
            return Ok(Value::Tuple(Rc::new(vec![callee, argument])));
        }
        if let Some(binding) = &body.binding {
            let raw = *binding.open.get(position).ok_or_else(|| {
                fail(node, Reason::Host("missing open binding spot".into()))
            })?;
            let child = closure
                .captures
                .get(binding.callee_capture)
                .ok_or_else(|| {
                    fail(
                        node,
                        Reason::Host("missing open binding callee".into()),
                    )
                })?
                .clone();
            pay(1, node)?;
            path.push((closure, binding.callee_capture));
            current = child;
            relative = raw;
            continue;
        }
        let entry = ctx
            .program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.input_contract);
        break (closure, position, entry);
    };
    let Some(entry) = entry else {
        return Ok(Value::Tuple(Rc::new(vec![callee, argument])));
    };
    let guards = super::calls::GuardJobs::select(&closure, node, |guard| {
        (guard.entry == entry && guard.spot == position).then_some(1)
    })?;
    if guards.is_empty() {
        return Ok(Value::Tuple(Rc::new(vec![callee, argument])));
    }
    let mut copied = super::calls::copied_guarded_closure(ctx, &closure, node)?;
    if !super::calls::admit_receiver(ctx, &closure, &mut copied.filled, node)? {
        return Ok(Value::Tuple(Rc::new(vec![callee, argument])));
    }
    copied
        .guards
        .retain(|guard| guard.entry != entry || guard.spot != position);
    let mut child = Value::Function(Rc::new(copied));
    while let Some((parent, capture)) = path.pop() {
        let mut copied =
            super::calls::copied_guarded_closure(ctx, &parent, node)?;
        copied.captures[capture] = child;
        child = Value::Function(Rc::new(copied));
    }
    if let Some(owner) = guards.owner {
        for (index, _) in guards.indices {
            argument =
                admit_argument(ctx, &owner.guards[index], argument, node)?;
        }
    }
    Ok(Value::Tuple(Rc::new(vec![child, argument])))
}

fn append_guard_parameters(
    ctx: &Ctx,
    closure: &mut Closure,
    entry: usize,
    mut predicate: &ir::Predicate,
    node: usize,
    witnesses: &predicates::Witnesses,
) -> Result<(), Faulted> {
    let mut spot = closure.filled.len();
    while let ir::Predicate::Function { parameter, result } = predicate {
        super::pay(1, node)?;
        if **parameter != ir::Predicate::All(Vec::new()) {
            super::pay(2, node)?;
            let mut copied = predicates::copy_predicate(ctx, parameter, node)?;
            closure.guards.push(Guard {
                spot,
                entry,
                predicate: copied.0.pop().expect("parameter predicate"),
                witnesses: witnesses.clone(),
            });
        }
        spot += 1;
        predicate = result;
    }
    Ok(())
}
