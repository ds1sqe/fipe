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
    ) -> Flow {
        let (predicate, components) = {
            let tables = program.tables.borrow();
            let entry = &tables.entries[entry];
            (entry.predicate.clone(), components(value, &entry.path))
        };
        if components.is_empty() {
            return Flow::Continue;
        }
        let run = Run::over(predicate, components, After::Enter(entry));
        self.frames.push(Frame::Native(NativeFrame {
            node,
            pending: Pending::Predicate(run),
            back: Return::Continue,
            result: None,
        }));
        Flow::Continue
    }
}

impl Machine {
    /// Test one value by a predicate, a Fault naming `entry` on failure:
    /// a closure's guard on an argument.
    pub(super) fn enter_by(
        &mut self,
        predicate: ir::Predicate,
        entry: usize,
        value: Value,
        node: usize,
    ) {
        let run = Run::new(predicate, value, After::Enter(entry));
        self.frames.push(Frame::Native(NativeFrame {
            node,
            pending: Pending::Predicate(run),
            back: Return::Continue,
            result: None,
        }));
    }

    /// An open function entry: the closure keeps the entry's parameter
    /// limits, tested on each of its calls (07-checks §What is known 8).
    pub(super) fn guard(
        closure: &crate::value::Closure,
        entry: usize,
        mut predicate: &ir::Predicate,
    ) -> Value {
        let mut closure = closure.clone();
        let mut spot = closure.filled.len();
        while let ir::Predicate::Function { parameter, result } = predicate {
            if **parameter != ir::Predicate::All(Vec::new()) {
                closure.guards.push(crate::value::Guard {
                    spot,
                    entry,
                    predicate: (**parameter).clone(),
                });
            }
            spot += 1;
            predicate = result;
        }
        Value::Function(alloc::rc::Rc::new(closure))
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
