//! The compile phase: after lowering, the checker runs pure code on
//! exact values on the machine, each evaluation under the full compile
//! budget (`07-checks.md` §Phases).
//!
//! First a literal whose reader runs, a program's `literal` or a read
//! cast of a written literal, becomes a constant; a Fault there is a
//! check error. Then each value demanded at compile time is prepared; one
//! that Faults or cannot be ready within the budget is a check error.
//! Last a test on an exact value is settled: proved, it leaves the plan;
//! refuted, or Faulting, it is a check error; out of budget, it stays
//! open with that reason.
use super::{Error, Evaluated};
use crate::{
    compile::{self, Unavailable},
    embed::{Outcome, Request},
    ir,
    limits::known::Known,
    mir,
    token::Span,
    value::{self, Value},
    vm::{Fault, Machine},
};
use alloc::{boxed::Box, string::String, vec::Vec};

/// An entry the phase proved, by its plan entry, with its value in the
/// language's spelling.
#[derive(Clone, Debug)]
pub struct Proved {
    pub entry: ir::Entry,
    pub value: String,
}

/// What the phase did to a unit.
#[derive(Clone, Debug, Default)]
pub struct Phase {
    pub proved: Vec<Proved>,
    /// Entries left open because the budget ran out.
    pub exhausted: Vec<ir::Entry>,
    /// Values the phase could not evaluate alone, by the span of the
    /// node: what they read from outside themselves.
    pub unavailable: Vec<(Span, Unavailable)>,
    /// Demanded values prepared, by the span of the demand, with what is
    /// known of them now.
    pub prepared: Vec<(Span, Known)>,
    /// Values computed but kept as code: they hold a closure or a
    /// witness, so they run at their own phase.
    pub code: Vec<Span>,
}

/// One evaluation's end.
enum Evaluation {
    Value(Value),
    Fault(Fault),
    Unavailable(Unavailable),
}

/// Run the compile phase over the nodes a unit emitted from `first` on.
///
/// # Errors
/// A refuted entry, or a test that Faults, as [`Error::Compile`].
///
/// # Panics
/// When an evaluation reaches a host declaration: purity was checked,
/// so that is a crate defect.
pub fn run(
    unit: &mut ir::Unit,
    first: usize,
    budget: u64,
    demands: &[Span],
) -> Result<Phase, Vec<Error>> {
    let mut phase = Phase::default();
    let mut errors = literals(unit, first, budget, &mut phase);
    if errors.is_empty() {
        errors = sequences(unit, first, budget, &mut phase);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let errors = demanded(unit, first, budget, demands, &mut phase);
    if !errors.is_empty() {
        return Err(errors);
    }
    entries(unit, first, budget, &mut phase)?;
    Ok(phase)
}

/// The literals whose reader runs, and the read casts of written
/// literals, as constants.
fn literals(
    unit: &ir::Unit,
    first: usize,
    budget: u64,
    phase: &mut Phase,
) -> Vec<Error> {
    let nodes: Vec<usize> = {
        let tables = unit.tables();
        (first..tables.nodes.len())
            .filter(|node| {
                !tables.prepared.contains_key(node)
                    && ((compile::runs_reader(&tables, *node)
                        && !compile::host_reader(&tables, *node))
                        || literal_cast(&tables, *node))
            })
            .collect()
    };
    let mut errors = Vec::new();
    for node in nodes {
        let span = unit.tables().nodes[node].span;
        match evaluate(unit, node, budget) {
            Evaluation::Value(value) => prepare(unit, node, &value, phase),
            // A generic body's reader dispatches on its witness when it
            // runs; a reader out of budget runs then too.
            Evaluation::Unavailable(_)
            | Evaluation::Fault(Fault {
                reason: value::Reason::Budget,
                ..
            }) => phase.code.push(span),
            Evaluation::Fault(fault) => errors.push(failure(
                unit,
                fault,
                span,
                Evaluated::Literal {
                    meaning: meaning_label(unit, node),
                },
            )),
        }
    }
    errors
}

/// Each bracket of exact elements handed to a program's `sequence`, as
/// a constant (`05-containers.md`): a Fault there is a check error.
fn sequences(
    unit: &ir::Unit,
    first: usize,
    budget: u64,
    phase: &mut Phase,
) -> Vec<Error> {
    let nodes: Vec<usize> = {
        let tables = unit.tables();
        (first..tables.nodes.len())
            .filter(|node| {
                !tables.prepared.contains_key(node)
                    && written_sequence(&tables, *node)
                        .is_some_and(|array| exact(&tables, array))
            })
            .collect()
    };
    let mut errors = Vec::new();
    for node in nodes {
        let span = unit.tables().nodes[node].span;
        match evaluate(unit, node, budget) {
            Evaluation::Value(value) => prepare(unit, node, &value, phase),
            Evaluation::Unavailable(_)
            | Evaluation::Fault(Fault {
                reason: value::Reason::Budget,
                ..
            }) => phase.code.push(span),
            Evaluation::Fault(fault) => errors.push(failure(
                unit,
                fault,
                span,
                Evaluated::Sequence {
                    meaning: meaning_label(unit, node),
                },
            )),
        }
    }
    errors
}

/// The array a bracket hands to a program's `sequence`: a call of one
/// spot whose supply is an array written where the call stands.
fn written_sequence(tables: &ir::Tables, node: usize) -> Option<usize> {
    let ir::Kind::Call { supplies, .. } = &tables.nodes[node].kind else {
        return None;
    };
    let [supply] = supplies.as_slice() else {
        return None;
    };
    (matches!(tables.nodes[supply.value].kind, ir::Kind::Array(_))
        && tables.nodes[supply.value].span == tables.nodes[node].span)
        .then_some(supply.value)
}

/// Whether a node is an exact value by its form: a literal the
/// compiler prepares, a prepared node, or a construction of exact parts.
fn exact(tables: &ir::Tables, node: usize) -> bool {
    if tables.prepared.contains_key(&node) {
        return true;
    }
    match &tables.nodes[node].kind {
        ir::Kind::Literal { .. } => !compile::runs_reader(tables, node),
        ir::Kind::Array(parts) | ir::Kind::Tuple(parts) => {
            parts.iter().all(|part| exact(tables, *part))
        }
        ir::Kind::Construct { fields, .. } => {
            fields.iter().all(|field| exact(tables, *field))
        }
        ir::Kind::Enter { value, .. } => exact(tables, *value),
        _ => false,
    }
}

/// A cast that may Fault, of a literal, by the prelude's own reader: its
/// verdict is known now. A host's reader is an effect and runs when the
/// cast does.
fn literal_cast(tables: &ir::Tables, node: usize) -> bool {
    let ir::Kind::Cast {
        row: ir::CastRow::Read | ir::CastRow::Number,
        value,
        operation,
        ..
    } = tables.nodes[node].kind
    else {
        return false;
    };
    matches!(tables.nodes[value].kind, ir::Kind::Literal { .. })
        && operation.is_none_or(|operation| {
            compile::prelude_operation(tables, operation)
        })
}

/// The label of a node's meaning, or the type's words when it has none.
fn meaning_label(unit: &ir::Unit, node: usize) -> String {
    let tables = unit.tables();
    match &tables.nodes[node].ty {
        ir::Type::Meaning { index, .. } => {
            tables.meanings[*index].label.clone()
        }
        _ => String::from("its place"),
    }
}

/// Keep a computed value as the node's constant, when it is one.
fn prepare(unit: &ir::Unit, node: usize, value: &Value, phase: &mut Phase) {
    if let Some(constant) = constant(value) {
        unit.tables.borrow_mut().prepared.insert(node, constant);
    } else {
        phase.code.push(unit.tables().nodes[node].span);
    }
}

/// Each `Compile (value)` and each value a binding's place demands at
/// compile time, prepared.
fn demanded(
    unit: &ir::Unit,
    first: usize,
    budget: u64,
    demands: &[Span],
    phase: &mut Phase,
) -> Vec<Error> {
    let nodes: Vec<usize> = {
        let tables = unit.tables();
        let mut nodes: Vec<usize> = (first..tables.nodes.len())
            .filter(|node| {
                matches!(
                    tables.nodes[*node].kind,
                    ir::Kind::Timing {
                        phase: ir::Phase::Compile,
                        value,
                    } if !exact(&tables, value)
                )
            })
            .collect();
        for span in demands {
            let Some(mut node) = (first..tables.nodes.len())
                .find(|node| tables.nodes[*node].span == *span)
            else {
                continue;
            };
            while let ir::Kind::Enter { value, .. } = tables.nodes[node].kind {
                node = value;
            }
            if !exact(&tables, node) && !nodes.contains(&node) {
                nodes.push(node);
            }
        }
        nodes.retain(|node| !tables.prepared.contains_key(node));
        nodes
    };
    let mut errors = Vec::new();
    for node in nodes {
        let span = unit.tables().nodes[node].span;
        match evaluate(unit, node, budget) {
            Evaluation::Value(value) => {
                prepare(unit, node, &value, phase);
                phase.prepared.push((span, known(&value, &unit.tables())));
            }
            Evaluation::Unavailable(what) => {
                phase.unavailable.push((span, what));
            }
            // A demanded value that cannot be ready is a check error
            // (`07-checks.md` §Timing), the budget's end included.
            Evaluation::Fault(fault) => {
                errors.push(failure(unit, fault, span, Evaluated::Demand));
            }
        }
    }
    errors
}

/// The check error of a Fault met while settling `what` at `span`. A
/// refuted entry inside the evaluated code is reported against its own
/// limit, a refuted divisor as a division by zero, each at its own place
/// when that lies inside `span`.
fn failure(
    unit: &ir::Unit,
    fault: Fault,
    span: Span,
    what: Evaluated,
) -> Error {
    let at = if super::within(fault.span, span) {
        fault.span
    } else {
        span
    };
    let value::Reason::Failed { entry, value } = &fault.reason else {
        return Error::Compile {
            span,
            what,
            reason: fault.reason,
        };
    };
    let tables = unit.tables();
    let entry = &tables.entries[*entry];
    if matches!(entry.predicate, ir::Predicate::Nonzero { .. }) {
        return Error::Compile {
            span: at,
            what,
            reason: value::Reason::DivideByZero,
        };
    }
    Error::Compile {
        span: at,
        what: Evaluated::Entry {
            limit: entry.label.clone(),
            value: Some(value.clone()),
        },
        reason: fault.reason.clone(),
    }
}

/// What is known of a prepared value: an `Integer`'s exact range, a
/// `Bool`, else the value in the language's spelling.
fn known(value: &Value, tables: &ir::Tables) -> Known {
    use crate::limits::known::{Integer, Range};
    match value {
        Value::Int(n) => Known {
            range: Some(Range::exact(Integer::new(
                *n < 0,
                crate::token::Magnitude::from(n.unsigned_abs()),
            ))),
            ..Known::default()
        },
        Value::Big(big) => Known {
            range: Some(Range::exact(Integer::new(
                big.negative,
                big.magnitude.clone(),
            ))),
            ..Known::default()
        },
        Value::Record(record)
            if tables.meanings[record.meaning].label == "Bool"
                && record.fields.is_empty() =>
        {
            Known {
                boolean: Some(
                    tables.meanings[record.meaning].constructors
                        [record.constructor]
                        .label
                        == "True",
                ),
                ..Known::default()
            }
        }
        _ => Known {
            exact: Some(value::structural::show(value, tables)),
            ..Known::default()
        },
    }
}

/// Settle each plan entry of the unit on an exact value.
fn entries(
    unit: &mut ir::Unit,
    first: usize,
    budget: u64,
    phase: &mut Phase,
) -> Result<(), Vec<Error>> {
    let mut errors = Vec::new();
    let entry_start = unit.tables().entries.len() - unit.plan.len();
    let mut plan = Vec::new();
    for (position, entry) in
        core::mem::take(&mut unit.plan).into_iter().enumerate()
    {
        let index = entry_start + position;
        if entry.reason != ir::Reason::CompileTimeEvaluation {
            plan.push(entry);
            continue;
        }
        let Some((node, value)) = enter_node(unit, first, index) else {
            plan.push(entry);
            continue;
        };
        match settle(unit, (node, value), (index, &entry), budget) {
            Settled::Proved(shown) => {
                let kind = unit.tables().nodes[value].kind.clone();
                unit.tables.borrow_mut().nodes[node].kind = kind;
                phase.proved.push(Proved {
                    entry,
                    value: shown,
                });
            }
            Settled::Exhausted => {
                unit.tables.borrow_mut().entries[index].reason =
                    ir::Reason::CompileBudget;
                let entry = ir::Entry {
                    reason: ir::Reason::CompileBudget,
                    ..entry
                };
                phase.exhausted.push(entry.clone());
                plan.push(entry);
            }
            Settled::Unavailable(what) => {
                let span = unit.tables().nodes[value].span;
                phase.unavailable.push((span, what));
                plan.push(entry);
            }
            Settled::Failed(error) => {
                errors.push(error);
                plan.push(entry);
            }
        }
    }
    unit.plan = plan;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// How an entry ended in the phase.
enum Settled {
    Proved(String),
    Exhausted,
    Unavailable(Unavailable),
    Failed(Error),
}

/// Evaluate an entry's value, prepare it when it is a constant, then
/// run the entry's test on it.
fn settle(
    unit: &ir::Unit,
    (node, value): (usize, usize),
    (index, entry): (usize, &ir::Entry),
    budget: u64,
) -> Settled {
    let span = unit.tables().nodes[value].span;
    let limit = |value| Evaluated::Entry {
        limit: entry.label.clone(),
        value,
    };
    let computed = match evaluate(unit, value, budget) {
        Evaluation::Value(computed) => computed,
        Evaluation::Fault(Fault {
            reason: value::Reason::Budget,
            ..
        }) => return Settled::Exhausted,
        Evaluation::Fault(fault) => {
            return Settled::Failed(failure(unit, fault, span, limit(None)))
        }
        Evaluation::Unavailable(what) => return Settled::Unavailable(what),
    };
    let shown = value::structural::show(&computed, &unit.tables());
    if let Some(constant) = constant(&computed) {
        unit.tables.borrow_mut().prepared.insert(value, constant);
    }
    // A value holding a closure stays code; the test runs on it again.
    match evaluate(unit, node, budget) {
        Evaluation::Value(_) => Settled::Proved(shown),
        Evaluation::Fault(Fault {
            reason: value::Reason::Budget,
            ..
        }) => Settled::Exhausted,
        // A refuted entry inside the test is reported against its own
        // limit.
        Evaluation::Fault(
            fault @ Fault {
                reason: value::Reason::Failed { .. },
                ..
            },
        ) if !matches!(
            fault.reason,
            value::Reason::Failed { entry: refuted, .. } if refuted == index
        ) =>
        {
            Settled::Failed(failure(
                unit,
                fault,
                entry.span,
                limit(Some(shown)),
            ))
        }
        // A test that Faults has failed (`07-checks.md` §What is known
        // 1); the reason says why.
        Evaluation::Fault(fault) => Settled::Failed(Error::Compile {
            span: entry.span,
            what: limit(Some(shown)),
            reason: fault.reason,
        }),
        Evaluation::Unavailable(what) => Settled::Unavailable(what),
    }
}

/// The `Enter` node of a plan entry among the unit's own nodes, with the
/// node of the value it enters.
fn enter_node(
    unit: &ir::Unit,
    first: usize,
    entry: usize,
) -> Option<(usize, usize)> {
    let tables = unit.tables();
    tables.nodes[first..]
        .iter()
        .enumerate()
        .find_map(|(offset, node)| match node.kind {
            ir::Kind::Enter { entry: e, value } if e == entry => {
                Some((first + offset, value))
            }
            _ => None,
        })
}

/// Run one node of the unit alone, on a fresh machine with the full
/// budget.
fn evaluate(unit: &ir::Unit, node: usize, budget: u64) -> Evaluation {
    let program = match compile::evaluation(unit, node) {
        Ok(program) => program,
        Err(what) => return Evaluation::Unavailable(what),
    };
    let mut machine = Machine::new(budget);
    match machine.run(&program) {
        Outcome::Done(Ok(value)) => Evaluation::Value(value),
        Outcome::Done(Err(fault)) => Evaluation::Fault(fault),
        Outcome::Request(request) => host_reached(&program, &request),
    }
}

fn host_reached(program: &mir::Program, request: &Request) -> ! {
    let label = match request {
        Request::Call { declaration, .. } => {
            let tables = program.tables.borrow();
            let declaration = &tables.declarations[*declaration];
            alloc::format!(
                "{}.{}",
                tables.namespaces[declaration.namespace],
                declaration.label
            )
        }
        Request::File { stem } => stem.clone(),
    };
    panic!("the compile phase reached the host at {label}: purity was checked")
}

/// A value as a constant of the unit; `None` when it holds a closure or
/// a witness, which stay code.
pub fn constant(value: &Value) -> Option<ir::Constant> {
    let each = |parts: &[Value]| -> Option<Vec<ir::Constant>> {
        parts.iter().map(constant).collect()
    };
    Some(match value {
        Value::Unit => ir::Constant::Unit,
        Value::Int(n) => ir::Constant::Integer {
            negative: *n < 0,
            magnitude: crate::token::Magnitude::from(n.unsigned_abs()),
        },
        Value::Big(big) => ir::Constant::Integer {
            negative: big.negative,
            magnitude: big.magnitude.clone(),
        },
        Value::F32(n) => ir::Constant::F32(n.to_bits()),
        Value::F64(n) => ir::Constant::F64(n.to_bits()),
        Value::Text(text) => ir::Constant::Text(String::clone(text)),
        Value::Seq(parts) => ir::Constant::Sequence(each(parts)?),
        Value::Tuple(parts) => ir::Constant::Tuple(each(parts)?),
        Value::Record(record) => ir::Constant::Record {
            meaning: record.meaning,
            constructor: record.constructor,
            fields: each(&record.fields)?,
        },
        Value::Packed(packed) => ir::Constant::Packed {
            tag: packed.meaning,
            value: Box::new(constant(&packed.value)?),
        },
        Value::Function(_) | Value::Meaning(_) => return None,
    })
}
