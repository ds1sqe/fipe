//! Entries at run time: an `Eval` or `Init` entry tests each component
//! its path reaches from the value, in order, by the entry's predicate;
//! the first failure Faults naming the entry and the component.
use super::{
    frames::{Frame, NativeFrame},
    pending::Pending,
    predicates::{After, Run},
    Flow, Machine, Return,
};
use crate::{
    ir::{self, Projection},
    mir,
    value::Value,
};
use alloc::vec::Vec;

impl Machine {
    /// Start the entry's predicate over the components; the body
    /// continues where it stands once each passes.
    pub(super) fn enter(
        &mut self,
        program: &mir::Program,
        entry: usize,
        value: Value,
        node: usize,
    ) -> Result<Flow, crate::value::Reason> {
        let (snapshot, path) = {
            let tables = program.tables.borrow();
            let entry = &tables.entries[entry];
            (
                self.snapshot_predicate(program, &entry.predicate)?,
                entry.path.clone(),
            )
        };
        let components = components(value, &path);
        if components.is_empty() {
            return Ok(Flow::Continue);
        }
        let witnesses =
            self.predicate_witnesses(program, &snapshot.0[0], false)?;
        let run = self.owned_predicate_run(
            program,
            snapshot,
            components,
            After::Enter(entry),
            witnesses,
        )?;
        self.frames.push(Frame::Native(NativeFrame {
            node,
            pending: Pending::Predicate(run),
            back: Return::Continue,
            result: None,
        }));
        Ok(Flow::Continue)
    }
}

impl Machine {
    pub(super) fn predicate_witnesses(
        &mut self,
        program: &mir::Program,
        predicate: &ir::Predicate,
        entry: bool,
    ) -> Result<super::resource::Witnesses, crate::value::Reason> {
        let work = self.resource_work();
        let frame = self.top();
        let captured = super::resource::capture_predicate(
            predicate,
            entry,
            &program.bodies[frame.body].hidden,
            &frame.meanings,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())?;
        captured
    }

    /// Attach a selected declaration's input guards before supplying its receiver.
    pub(super) fn receiver_closure(
        &mut self,
        program: &mir::Program,
        mut closure: crate::value::Closure,
    ) -> Result<Value, crate::value::Reason> {
        let entry = program
            .receiver_contexts
            .get(closure.body)
            .and_then(Option::as_ref)
            .and_then(|context| context.input_contract);
        if let Some(entry) = entry {
            debug_assert!(closure.filled.is_empty());
            let tables = program.tables.borrow();
            let predicate = &tables.entries[entry].predicate;
            let work = self.resource_work();
            let captured = super::resource::capture_predicate(
                predicate,
                true,
                &program.bodies[closure.body].hidden,
                &closure.meanings,
                &tables,
                &work,
            );
            self.pay(work.spent())?;
            let witnesses = captured?;
            self.append_guard_parameters(
                program,
                &mut closure,
                entry,
                predicate,
                &witnesses,
            )?;
        }
        Ok(Value::Function(alloc::rc::Rc::new(closure)))
    }

    /// Move one ordered guard without changing its original argument alias.
    pub(super) fn admit_argument(
        &mut self,
        program: &mir::Program,
        job: super::pending::GuardJob,
        value: Value,
        node: usize,
    ) -> Result<Flow, crate::value::Reason> {
        let guard = job.guard();
        if let (Value::Function(closure), ir::Predicate::Function { .. }) =
            (&value, &guard.predicate)
        {
            let guarded = self.guard_with_witnesses(
                program,
                closure,
                guard.entry,
                &guard.predicate,
                &guard.witnesses,
            )?;
            let Some(Frame::Native(frame)) = self.frames.last_mut() else {
                return Err(crate::value::Reason::Host(
                    "missing argument sequence".into(),
                ));
            };
            frame.pending.argument_returned(guarded)?;
            return Ok(Flow::Continue);
        }
        let work = self.resource_work();
        let run = Run::guarded(job, value, &program.tables.borrow(), &work);
        self.pay(work.spent())?;
        self.frames.push(Frame::Native(NativeFrame {
            node,
            pending: Pending::Predicate(run?),
            back: Return::Resume,
            result: None,
        }));
        Ok(Flow::Continue)
    }

    /// An open function entry: the closure keeps the entry's parameter
    /// limits, tested on each of its calls (07-checks §What is known 8).
    pub(super) fn guard(
        &mut self,
        program: &crate::mir::Program,
        closure: &crate::value::Closure,
        entry: usize,
        predicate: &ir::Predicate,
    ) -> Result<Value, crate::value::Reason> {
        let witnesses = self.predicate_witnesses(program, predicate, true)?;
        self.guard_with_witnesses(
            program, closure, entry, predicate, &witnesses,
        )
    }

    /// Copy a function and append guards using its supplied witness context.
    pub(super) fn guard_with_witnesses(
        &mut self,
        program: &crate::mir::Program,
        closure: &crate::value::Closure,
        entry: usize,
        predicate: &ir::Predicate,
        witnesses: &super::resource::Witnesses,
    ) -> Result<Value, crate::value::Reason> {
        let work = self.resource_work();
        let copied = super::resource::copy_guarded_closure(
            closure,
            program,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())?;
        let mut closure = copied?;
        self.append_guard_parameters(
            program,
            &mut closure,
            entry,
            predicate,
            witnesses,
        )?;
        Ok(Value::Function(alloc::rc::Rc::new(closure)))
    }
}

/// The values a path reaches from a value, in order: every element, a
/// field or part, a payload or a case's field only when the value has
/// that constructor.
fn components(value: Value, path: &[Projection]) -> Vec<Value> {
    let mut reached = alloc::vec![value];
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
            alloc::vec![record.fields[*index].clone()]
        }
        (Projection::Part(index), Value::Tuple(parts)) => {
            alloc::vec![parts[*index].clone()]
        }
        (
            Projection::CaseField { constructor, field },
            Value::Record(record),
        ) if record.constructor == *constructor => {
            alloc::vec![record.fields[*field].clone()]
        }
        // The prelude's sides in declaration order: a payload is visited
        // on the constructor of its index.
        (Projection::Payload(index), Value::Record(record))
            if record.constructor == *index =>
        {
            alloc::vec![record.fields[0].clone()]
        }
        (Projection::CaseField { .. } | Projection::Payload(_), _) => {
            Vec::new()
        }
        (projection, value) => {
            unreachable!("{projection:?} of {value:?}")
        }
    }
}

impl Machine {
    /// Prepare a private callable path for one supplied formal spot.
    /// The tuple remains private until the returned guards have succeeded.
    pub(super) fn admitted_binding(
        &mut self,
        program: &mir::Program,
        callee: Value,
        spot: usize,
        argument: Value,
        node: usize,
    ) -> Result<(Value, super::pending::GuardJobs), crate::value::Reason> {
        use crate::value::{Closure, Reason};
        use alloc::rc::Rc;
        let mut current = callee.clone();
        let mut relative = spot;
        let mut path: Vec<(Rc<Closure>, usize)> = Vec::new();
        let (closure, position, entry) = loop {
            self.pay(1)?;
            let Value::Function(closure) = current else {
                return Ok((
                    Value::Tuple(Rc::new(alloc::vec![callee, argument])),
                    super::pending::GuardJobs::default(),
                ));
            };
            let body = program
                .bodies
                .get(closure.body)
                .ok_or_else(|| Reason::Host("missing binding body".into()))?;
            let position = relative
                .checked_add(closure.filled.len())
                .ok_or_else(|| Reason::Host("binding spot overflow".into()))?;
            // Beyond this raw body, application belongs to its result.
            // Its result callable is not available at this binding boundary.
            if position >= body.arity {
                return Ok((
                    Value::Tuple(Rc::new(alloc::vec![callee, argument])),
                    super::pending::GuardJobs::default(),
                ));
            }
            if let Some(binding) = &body.binding {
                let raw = *binding.open.get(position).ok_or_else(|| {
                    Reason::Host("missing open binding spot".into())
                })?;
                let child = closure
                    .captures
                    .get(binding.callee_capture)
                    .ok_or_else(|| {
                        Reason::Host("missing open binding callee".into())
                    })?
                    .clone();
                self.pay(1)?;
                path.push((closure, binding.callee_capture));
                current = child;
                relative = raw;
                continue;
            }
            let entry = program
                .receiver_contexts
                .get(closure.body)
                .and_then(Option::as_ref)
                .and_then(|context| context.input_contract);
            break (closure, position, entry);
        };
        let Some(entry) = entry else {
            return Ok((
                Value::Tuple(Rc::new(alloc::vec![callee, argument])),
                super::pending::GuardJobs::default(),
            ));
        };
        let guards = self.guard_jobs(&closure, |guard| {
            (guard.entry == entry && guard.spot == position).then_some(1)
        })?;
        if guards.is_empty() {
            return Ok((
                Value::Tuple(Rc::new(alloc::vec![callee, argument])),
                guards,
            ));
        }
        let work = self.resource_work();
        let copied = super::resource::copy_guarded_closure(
            &closure,
            program,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())?;
        let mut copied = copied?;
        let context = program.receiver_contexts[closure.body]
            .as_ref()
            .expect("row input context");
        if !self
            .admit_receiver_only(
                program,
                context,
                &copied.meanings,
                &mut copied.filled,
                node,
            )
            .map_err(|fault| fault.reason)?
        {
            return Ok((
                Value::Tuple(Rc::new(alloc::vec![callee, argument])),
                super::pending::GuardJobs::default(),
            ));
        }
        copied
            .guards
            .retain(|guard| guard.entry != entry || guard.spot != position);
        let child = Value::Function(Rc::new(copied));
        let child = self.rebuild_binding_path(program, child, path)?;
        Ok((Value::Tuple(Rc::new(alloc::vec![child, argument])), guards))
    }

    fn rebuild_binding_path(
        &mut self,
        program: &mir::Program,
        mut child: Value,
        mut path: Vec<(alloc::rc::Rc<crate::value::Closure>, usize)>,
    ) -> Result<Value, crate::value::Reason> {
        use alloc::rc::Rc;
        while let Some((parent, capture)) = path.pop() {
            let work = self.resource_work();
            let copied = super::resource::copy_guarded_closure(
                &parent,
                program,
                &program.tables.borrow(),
                &work,
            );
            self.pay(work.spent())?;
            let mut copied = copied?;
            copied.captures[capture] = child;
            child = Value::Function(Rc::new(copied));
        }
        Ok(child)
    }
}

impl Machine {
    pub(super) fn guard_jobs(
        &mut self,
        owner: &alloc::rc::Rc<crate::value::Closure>,
        select: impl FnMut(&crate::value::Guard) -> Option<usize>,
    ) -> Result<super::pending::GuardJobs, crate::value::Reason> {
        let work = self.resource_work();
        let jobs = super::pending::GuardJobs::select(owner, &work, select);
        self.pay(work.spent())?;
        jobs
    }
}

impl Machine {
    fn append_guard_parameters(
        &mut self,
        program: &mir::Program,
        closure: &mut crate::value::Closure,
        entry: usize,
        mut predicate: &ir::Predicate,
        witnesses: &super::resource::Witnesses,
    ) -> Result<(), crate::value::Reason> {
        let mut spot = closure.filled.len();
        while let ir::Predicate::Function { parameter, result } = predicate {
            let work = self.resource_work();
            let copied = (|| {
                work.pay(1)?;
                if **parameter == ir::Predicate::All(Vec::new()) {
                    return Ok(None);
                }
                work.pay(2)?;
                super::resource::copy_predicate(
                    parameter,
                    &program.tables.borrow(),
                    &work,
                )
                .map(Some)
            })();
            self.pay(work.spent())?;
            if let Some(mut copied) = copied? {
                closure.guards.push(crate::value::Guard {
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
}

impl Machine {
    pub(super) fn snapshot_predicate(
        &mut self,
        program: &mir::Program,
        predicate: &ir::Predicate,
    ) -> Result<crate::value::resource::Predicates, crate::value::Reason> {
        let work = self.resource_work();
        let copied = super::resource::copy_predicate(
            predicate,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())?;
        copied
    }
    pub(super) fn owned_predicate_run(
        &mut self,
        program: &mir::Program,
        predicate: crate::value::resource::Predicates,
        values: Vec<Value>,
        after: After,
        witnesses: super::resource::Witnesses,
    ) -> Result<Run, crate::value::Reason> {
        let work = self.resource_work();
        let run = Run::owned(
            predicate,
            values,
            after,
            witnesses,
            &program.tables.borrow(),
            &work,
        );
        self.pay(work.spent())?;
        run
    }
}
