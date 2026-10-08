//! Thunks: a call's open spots, a bare member on a value, and a
//! declaration, constructor, member or derived operation as a value.
use super::{
    builder::Builder,
    calls::{Arguments, Site, Target},
};
use crate::{
    ir,
    mir::{Operand, Rvalue, Terminator},
};
use alloc::vec::Vec;

/// The spot types of a callable type, `arity` of them.
fn parameters(ty: &ir::Type, arity: usize) -> (Vec<ir::Type>, ir::Type) {
    let mut ty = ty;
    let mut parameters = Vec::with_capacity(arity);
    for _ in 0..arity {
        let ir::Type::Function { parameter, result } = ty else {
            unreachable!("a callable of {arity} spots")
        };
        parameters.push((**parameter).clone());
        ty = result;
    }
    (parameters, ty.clone())
}

/// The first spot that is not the receiver: the overload argument.
pub(super) fn argument_spot(
    count: usize,
    receiver_spot: Option<usize>,
) -> Option<usize> {
    (0..count).find(|spot| Some(*spot) != receiver_spot)
}

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
            _ => return None,
        };
        if arity != 0 {
            return None;
        }
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let arguments = Arguments {
            args: Vec::new(),
            receiver: None,
            argument: None,
        };
        Some(self.apply(target, arguments, dest, node))
    }

    /// The function of a call's open spots: its captures are the
    /// supplied values, its spots the open ones in recorded order; the
    /// defaults run when it is called, reading both.
    pub(super) fn thunk(
        &mut self,
        site: &Site<'_>,
        locals: &[Option<usize>],
        target: Target,
        receiver_spot: Option<usize>,
        argument: Option<ir::Type>,
    ) -> Rvalue {
        let Site {
            node, slots, open, ..
        } = *site;
        let supplied = site.supplied();
        let mut captures: Vec<Operand> = supplied
            .iter()
            .map(|spot| Operand::Local(locals[*spot].expect("a supplied spot")))
            .collect();
        let (target, callee_capture) = match target {
            Target::Value(operand) => {
                captures.push(operand);
                (None, Some(captures.len() - 1))
            }
            target => (Some(target), None),
        };
        let (parameters, result) =
            site.open_signature(&self.tables().nodes[node].ty);
        let hidden = self.hidden.clone();
        let mut builder =
            Builder::new(self.lowering, node, result, hidden, Vec::new());
        builder.captures = captures.len();
        builder.count();
        for (spot, ty) in open.iter().zip(parameters) {
            builder.spot(Some(slots[*spot]), ty, node);
        }
        let inner: Vec<usize> = slots
            .iter()
            .map(|slot| builder.slot_local(*slot, node))
            .collect();
        for (position, spot) in supplied.iter().enumerate() {
            builder.assign(
                inner[*spot],
                Rvalue::Use(Operand::Capture(position)),
                node,
            );
        }
        for (spot, default) in site.defaults {
            builder.lower_to(*default, Some(inner[*spot]));
        }
        let target = target.unwrap_or_else(|| {
            Target::Value(Operand::Capture(
                callee_capture.expect("a value callee is captured"),
            ))
        });
        let arguments = Arguments {
            args: inner.iter().copied().map(Operand::Local).collect(),
            receiver: receiver_spot.map(|spot| Operand::Local(inner[spot])),
            argument,
        };
        builder.apply(target, arguments, 0, node);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        Rvalue::Closure {
            body,
            captures,
            meanings: self.inherited(),
        }
    }

    /// A member read on a value, `p.x`: the member applied to the
    /// receiver; with spots left, a thunk capturing the receiver.
    pub(super) fn member(
        &mut self,
        node: usize,
        member: usize,
        value: usize,
        dest: Option<usize>,
    ) -> Operand {
        let receiver = self.lower(value);
        self.count();
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let receiver_spot = tables.dispatch[member]
            .rows
            .first()
            .and_then(|row| row.receiver_spot)
            .unwrap_or(0);
        let target = || Target::Dispatch {
            member,
            receiver: ir::Receiver::Value(value),
        };
        let remaining = super::Lowering::arity(ty);
        if remaining == 0 {
            let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
            let arguments = Arguments {
                args: alloc::vec![receiver],
                receiver: Some(receiver),
                argument: None,
            };
            return self.apply(target(), arguments, dest, node);
        }
        let (parameters, result) = parameters(ty, remaining);
        let argument = parameters.first().cloned();
        let hidden = self.hidden.clone();
        let mut builder =
            Builder::new(self.lowering, node, result, hidden, Vec::new());
        builder.captures = 1;
        builder.count();
        for ty in parameters {
            builder.spot(None, ty, node);
        }
        let mut args: Vec<Operand> =
            (1..=remaining).map(Operand::Local).collect();
        args.insert(receiver_spot.min(remaining), Operand::Capture(0));
        let arguments = Arguments {
            args,
            receiver: Some(Operand::Capture(0)),
            argument,
        };
        builder.apply(target(), arguments, 0, node);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        let closure = Rvalue::Closure {
            body,
            captures: alloc::vec![receiver],
            meanings: self.inherited(),
        };
        self.materialize(closure, dest, ty, node)
    }

    /// A declaration, constructor, member or derived operation as a
    /// value: a thunk of its arity.
    pub(super) fn thunk_of(&mut self, node: usize) -> Rvalue {
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        let (arity, target) = match &tables.nodes[node].kind {
            ir::Kind::Declared(index) => (
                super::Lowering::arity(ty),
                Target::Static(self.declared(*index), Vec::new()),
            ),
            ir::Kind::Derived { kind, ty: on } => {
                let on = self.lowering.fixed(on);
                (
                    super::Lowering::arity(ty),
                    Target::Static(self.derived(*kind, on), Vec::new()),
                )
            }
            ir::Kind::Dispatch { receiver, member } => (
                super::Lowering::arity(ty),
                Target::Dispatch {
                    member: *member,
                    receiver: receiver.clone(),
                },
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
        let receiver_spot = match &target {
            Target::Dispatch {
                member,
                receiver: ir::Receiver::Value(_),
            } => tables.dispatch[*member]
                .rows
                .first()
                .and_then(|row| row.receiver_spot),
            _ => None,
        };
        let (parameters, result) = parameters(ty, arity);
        let argument = argument_spot(arity, receiver_spot)
            .map(|spot| self.lowering.fixed(&parameters[spot]));
        let hidden = self.hidden.clone();
        let mut builder =
            Builder::new(self.lowering, node, result, hidden, Vec::new());
        builder.count();
        for ty in parameters {
            builder.spot(None, ty, node);
        }
        let arguments = Arguments {
            args: (1..=arity).map(Operand::Local).collect(),
            receiver: receiver_spot.map(|spot| Operand::Local(spot + 1)),
            argument,
        };
        builder.apply(target, arguments, 0, node);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        Rvalue::Closure {
            body,
            captures: Vec::new(),
            meanings: self.inherited(),
        }
    }
}
