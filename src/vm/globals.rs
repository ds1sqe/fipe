//! The globals a step reads, so an unset one no line stores is computed
//! before the step runs.
use crate::mir::{Callee, Operand, Rvalue, Statement, Terminator, Witness};
use alloc::vec::Vec;

fn push(found: &mut Vec<usize>, operand: &Operand) {
    if let Operand::Global(definition) = operand {
        found.push(*definition);
    }
}

fn rvalue(found: &mut Vec<usize>, rvalue: &Rvalue) {
    match rvalue {
        Rvalue::Admit { values, .. } => {
            for value in values {
                push(found, value);
            }
        }
        Rvalue::Use(value)
        | Rvalue::Field(value, _)
        | Rvalue::Repeat { value, .. }
        | Rvalue::Wrap { value, .. }
        | Rvalue::Cast { value, .. }
        | Rvalue::Convert { value, .. }
        | Rvalue::Pack { value, .. }
        | Rvalue::Length(value) => push(found, value),
        Rvalue::Construct { fields: parts, .. }
        | Rvalue::Tuple(parts)
        | Rvalue::Array(parts)
        | Rvalue::Closure {
            captures: parts, ..
        }
        | Rvalue::Native { args: parts, .. } => {
            for part in parts {
                push(found, part);
            }
        }
    }
}

/// The globals a statement reads.
pub(super) fn in_statement(statement: &Statement) -> Vec<usize> {
    let mut found = Vec::new();
    match statement {
        Statement::Assign(_, value) => rvalue(&mut found, value),
        Statement::Store { value, .. }
        | Statement::Enter { value, .. }
        | Statement::Push { value, .. }
        | Statement::Line(value) => push(&mut found, value),
        Statement::Charge(_) | Statement::Pop => (),
    }
    found
}

/// The globals a terminator reads.
pub(super) fn in_terminator(terminator: &Terminator) -> Vec<usize> {
    let mut found = Vec::new();
    match terminator {
        Terminator::Switch { value, .. } | Terminator::Test { value, .. } => {
            push(&mut found, value);
        }
        Terminator::Call { callee, args, .. } => {
            match callee {
                Callee::Value(value)
                | Callee::Dispatch {
                    witness: Witness::Value(value),
                    ..
                } => push(&mut found, value),
                _ => (),
            }
            for arg in args {
                push(&mut found, arg);
            }
        }
        Terminator::Goto(_) | Terminator::Return | Terminator::Unreachable => {}
    }
    found
}
