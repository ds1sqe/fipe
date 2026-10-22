//! Entries at run time, copied from `src/vm/entries.rs` and the `Enter`
//! statement of `src/vm/step.rs`: an entry tests each component its
//! path reaches from the value, in order, by the entry's predicate; a
//! function value meeting a function entry in a local is not tested but
//! rebuilt with guards, which each of its calls tests.
use super::{predicates, Ctx, Faulted, Site};
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
pub fn enter(site: Site, value: Value) -> Result<Option<Value>, Faulted> {
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
    let (predicate, path) = {
        let tables = ctx.tables();
        let entry = &tables.entries[entry];
        (entry.predicate.clone(), entry.path.clone())
    };
    // A function value is not tested; its calls are.
    if let (Value::Function(closure), Operand::Local(_)) = (&value, operand) {
        if matches!(predicate, ir::Predicate::Function { .. }) {
            return Ok(Some(guard(closure, entry, &predicate)));
        }
    }
    let components = components(value, &path);
    if !components.is_empty() {
        predicates::enter_each(&ctx, &predicate, entry, components, node)?;
    }
    Ok(None)
}

/// An open function entry: the closure keeps the entry's parameter
/// limits, tested on each of its calls (07-checks §What is known 8).
fn guard(
    closure: &Closure,
    entry: usize,
    mut predicate: &ir::Predicate,
) -> Value {
    let mut closure = closure.clone();
    let mut spot = closure.filled.len();
    while let ir::Predicate::Function { parameter, result } = predicate {
        if **parameter != ir::Predicate::All(Vec::new()) {
            closure.guards.push(Guard {
                spot,
                entry,
                predicate: (**parameter).clone(),
            });
        }
        spot += 1;
        predicate = result;
    }
    Value::Function(Rc::new(closure))
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
