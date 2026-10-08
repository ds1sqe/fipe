//! A declaration or constructor as a value: its own value when it has
//! no spots, else a thunk body of its arity.
use super::{builder::Builder, calls::Target};
use crate::{
    ir,
    mir::{Operand, Rvalue, Terminator},
};
use alloc::vec::Vec;

impl Builder<'_, '_> {
    /// A declaration or constructor of no spots is its value: the
    /// native's result or the record, never a thunk.
    pub(super) fn nullary(
        &mut self,
        node: usize,
        dest: Option<usize>,
    ) -> Option<Operand> {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let (arity, target) = match &tables.nodes[node].kind {
            ir::Kind::Declared(index) => (
                super::Lowering::arity(ty),
                Target::Static(self.declared(*index), Vec::new()),
            ),
            ir::Kind::Constructor {
                meaning,
                constructor,
                fields,
            } => (
                fields.len(),
                Target::Construct {
                    meaning: *meaning,
                    constructor: *constructor,
                    fields: fields.clone(),
                },
            ),
            other => unreachable!("a value of {other:?}"),
        };
        if arity != 0 {
            return None;
        }
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        Some(self.apply(target, Vec::new(), None, dest, node))
    }

    /// A declaration or constructor as a value: a thunk of its arity.
    pub(super) fn thunk_of(&mut self, node: usize) -> Rvalue {
        let tables = self.tables();
        let (arity, target) = match &tables.nodes[node].kind {
            ir::Kind::Declared(index) => (
                super::Lowering::arity(&tables.nodes[node].ty),
                Target::Static(self.declared(*index), Vec::new()),
            ),
            ir::Kind::Constructor {
                meaning,
                constructor,
                fields,
            } => (
                fields.len(),
                Target::Construct {
                    meaning: *meaning,
                    constructor: *constructor,
                    fields: fields.clone(),
                },
            ),
            other => unreachable!("a thunk of {other:?}"),
        };
        let mut ty = &tables.nodes[node].ty;
        let mut parameters = Vec::with_capacity(arity);
        for _ in 0..arity {
            let ir::Type::Function { parameter, result } = ty else {
                unreachable!("a callable of {arity} spots")
            };
            parameters.push((**parameter).clone());
            ty = result;
        }
        let hidden = self.hidden.clone();
        let mut builder =
            Builder::new(self.lowering, node, ty.clone(), hidden, Vec::new());
        builder.count();
        for ty in parameters {
            builder.spot(None, ty, node);
        }
        let args = (1..=arity).map(Operand::Local).collect();
        builder.apply(target, args, None, 0, node);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        Rvalue::Closure {
            body,
            captures: Vec::new(),
            meanings: self.inherited(),
        }
    }
}
