//! Casts, conversions, packs and entries: one value adapted in place.
use super::{
    builder::Builder,
    calls::{Arguments, Target},
};
use crate::{
    ir,
    mir::{Operand, Rvalue, Statement, Terminator},
};
use alloc::{vec, vec::Vec};

impl Builder<'_, '_> {
    /// A cast, a conversion as a value, a pack or an entry; counted here.
    pub(super) fn adapter(
        &mut self,
        node: usize,
        kind: &ir::Kind,
        dest: Option<usize>,
    ) -> Operand {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        match kind {
            ir::Kind::Cast {
                operation,
                row,
                from,
                to,
                value,
            } => {
                let mut operand = self.lower(*value);
                if let Some(operation) = operation {
                    operand = self.operate(*operation, operand, node);
                }
                self.count(node);
                let rvalue = Rvalue::Cast {
                    row: *row,
                    from: from.clone(),
                    to: to.clone(),
                    value: operand,
                };
                self.materialize(rvalue, dest, ty, node)
            }
            ir::Kind::Convert { .. } => {
                let closure = self.convert_thunk(node);
                self.count(node);
                self.materialize(closure, dest, ty, node)
            }
            ir::Kind::Pack { tag, value } => {
                let value = self.lower(*value);
                self.count(node);
                let rvalue = Rvalue::Pack {
                    tag: tag.clone(),
                    value,
                };
                self.materialize(rvalue, dest, ty, node)
            }
            ir::Kind::Enter { entry, value } => {
                let value = self.lower_to(*value, dest);
                self.lowering
                    .reach_predicate(&tables.entries[*entry].predicate);
                self.count(node);
                self.emit(
                    Statement::Enter {
                        entry: *entry,
                        value,
                    },
                    node,
                );
                value
            }
            other => unreachable!("an adapter of {other:?}"),
        }
    }

    /// Apply a selected operation (a cast's reader or shower, a
    /// conversion's member) to one value.
    fn operate(
        &mut self,
        operation: usize,
        value: Operand,
        node: usize,
    ) -> Operand {
        let tables = self.tables();
        let ir::Type::Function { result, .. } = &tables.nodes[operation].ty
        else {
            unreachable!("an operation takes the value")
        };
        let target: Target = self.callee(operation);
        let out = self.local((**result).clone(), None);
        let arguments = Arguments {
            args: vec![value],
            receiver: Some(value),
            argument: None,
        };
        self.apply(target, arguments, out, node)
    }

    /// A conversion of `value` into `dest`: with a selected callee, its
    /// parameter limit is entered and the callee runs; then the mode's
    /// adaptation with the target's predicate.
    pub(super) fn convert(
        &mut self,
        node: usize,
        value: Operand,
        dest: usize,
    ) -> Operand {
        let tables = self.tables();
        let ir::Kind::Convert {
            callee,
            mode,
            from,
            parameter,
            target,
            ..
        } = &tables.nodes[node].kind
        else {
            unreachable!("a conversion")
        };
        // The machine runs the limits' tests; their bodies are reached.
        self.lowering.reach_predicate(parameter);
        self.lowering.reach_predicate(target);
        let mut value = value;
        if let Some(callee) = callee {
            if !trivial(parameter) {
                let checked = self.local(from.clone(), None);
                let rvalue = Rvalue::Convert {
                    mode: ir::ConversionMode::Check,
                    predicate: parameter.clone(),
                    value,
                };
                self.assign(checked, rvalue, node);
                value = Operand::Local(checked);
            }
            value = self.operate(*callee, value, node);
        }
        let rvalue = Rvalue::Convert {
            mode: *mode,
            predicate: target.clone(),
            value,
        };
        self.assign(dest, rvalue, node);
        Operand::Local(dest)
    }

    /// A conversion as a value: a thunk of one spot.
    fn convert_thunk(&mut self, node: usize) -> Rvalue {
        let tables = self.tables();
        let ir::Type::Function { parameter, result } = &tables.nodes[node].ty
        else {
            unreachable!("a conversion takes the value")
        };
        let hidden = self.hidden.clone();
        let mut builder = Builder::new(
            self.lowering,
            node,
            (**result).clone(),
            hidden,
            Vec::new(),
        );
        builder.count(node);
        builder.spot(None, (**parameter).clone());
        builder.convert(node, Operand::Local(1), 0);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        Rvalue::Closure {
            body,
            captures: Vec::new(),
            meanings: self.inherited(),
        }
    }
}

/// A predicate every value passes.
fn trivial(predicate: &ir::Predicate) -> bool {
    match predicate {
        ir::Predicate::All(parts) => parts.iter().all(trivial),
        _ => false,
    }
}
