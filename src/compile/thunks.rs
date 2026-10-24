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
    /// A declaration, derived `default` or constructor of no spots is
    /// its value: the native's result, the default or the record,
    /// never a thunk.
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
            ir::Kind::Derived { kind, ty: on } => {
                let on = self.lowering.fixed(on);
                (
                    super::Lowering::arity(ty),
                    Target::Static(self.derived(*kind, on), Vec::new()),
                )
            }
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
        builder.binding =
            callee_capture.map(|callee_capture| crate::mir::OpenBinding {
                callee_capture,
                open: open.to_vec(),
            });
        builder.count(node);
        for (spot, ty) in open.iter().zip(parameters) {
            builder.spot(Some(slots[*spot]), ty);
        }
        let inner: Vec<usize> =
            slots.iter().map(|slot| builder.slot_local(*slot)).collect();
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
        self.count(node);
        let tables = self.tables();
        let ty = &tables.nodes[node].ty;
        // Binding supplies only the receiver; the adapter's actual body
        // arity decides whether this is a value or a partial function.
        let target = || Target::Dispatch {
            member,
            receiver: ir::Receiver::Value(value),
            signature: crate::value::resource::Forest(alloc::vec![
                ir::Type::Function {
                    parameter: alloc::boxed::Box::new(
                        tables.nodes[value].ty.clone()
                    ),
                    result: alloc::boxed::Box::new(ty.clone()),
                }
            ]),
        };
        let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
        let arguments = Arguments {
            args: alloc::vec![receiver],
            receiver: Some(receiver),
            argument: None,
        };
        self.apply(target(), arguments, dest, node)
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
                    signature: crate::value::resource::Forest(alloc::vec![
                        ty.clone()
                    ]),
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
                ..
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
        builder.count(node);
        for ty in parameters {
            builder.spot(None, ty);
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

impl super::Lowering<'_> {
    /// Reserve a receiver-first body without exposing an unfinished raw body.
    pub(super) fn receiver_adapter(
        &mut self,
        member: usize,
        row: usize,
        node: usize,
    ) -> Option<usize> {
        if let Some(adapter) = self
            .receiver_adapters
            .iter()
            .find(|a| a.member == member && a.row == row)
        {
            return Some(adapter.body);
        }
        let selected = &self.tables.dispatch[member].rows[row];
        let receiver_spot = selected.receiver_spot?;
        let target = match selected.target {
            ir::Implementation::Definition { index, .. } => {
                self.function_body(index)?;
                crate::mir::ReceiverTarget::Definition(index)
            }
            ir::Implementation::Declared(index) => {
                crate::mir::ReceiverTarget::Declared(index)
            }
            ir::Implementation::Derived { kind, .. } => {
                crate::mir::ReceiverTarget::Derived(kind)
            }
            ir::Implementation::Field(_) => return None,
        };
        match &target {
            crate::mir::ReceiverTarget::Declared(index)
                if self.native(*index) =>
            {
                let walked = Self::walked_members(
                    &self.tables.declarations[*index].label,
                );
                self.reach_members(walked);
            }
            crate::mir::ReceiverTarget::Derived(kind) => {
                self.reach_members(&[match kind {
                    ir::Derived::Eq => "eq",
                    ir::Derived::Cmp => "cmp",
                    ir::Derived::Show => "show",
                    ir::Derived::Default => "default",
                }]);
            }
            _ => (),
        }
        let body = self.reserve(node);
        self.receiver_adapters.push(crate::mir::ReceiverAdapter {
            member,
            row,
            target,
            receiver_spot,
            body,
        });
        Some(body)
    }

    fn receiver_adapter_prototype(
        &self,
        target: ir::Implementation,
        signature: &ir::Type,
        declared_arity: Option<usize>,
    ) -> (Vec<ir::Type>, ir::Type, Vec<usize>, Target) {
        match target {
            ir::Implementation::Definition { index, .. } => {
                let body = self.definitions[index].expect("reached raw body");
                let raw = &self.bodies[body];
                (
                    raw.locals[1..=raw.arity]
                        .iter()
                        .map(|local| local.ty.clone())
                        .collect::<Vec<_>>(),
                    raw.locals[0].ty.clone(),
                    raw.hidden.clone(),
                    super::calls::Target::Static(
                        crate::mir::Callee::Body(body),
                        raw.hidden
                            .iter()
                            .copied()
                            .map(ir::Type::Parameter)
                            .collect(),
                    ),
                )
            }
            ir::Implementation::Declared(index) => {
                let (parameters, result) = parameters(
                    signature,
                    declared_arity.expect("declared formal count"),
                );
                let hidden = type_parameters(signature);
                let callee = if self.native(index) {
                    crate::mir::Callee::Native(index)
                } else {
                    crate::mir::Callee::Host(index)
                };
                (
                    parameters,
                    result,
                    hidden,
                    super::calls::Target::Static(callee, Vec::new()),
                )
            }
            ir::Implementation::Derived { kind, ty } => {
                let arity = match kind {
                    ir::Derived::Eq | ir::Derived::Cmp => 2,
                    ir::Derived::Show => 1,
                    ir::Derived::Default => 0,
                };
                let (parameters, result) = parameters(signature, arity);
                let hidden = type_parameters(signature);
                (
                    parameters,
                    result,
                    hidden,
                    super::calls::Target::Static(
                        crate::mir::Callee::Derived { kind, ty },
                        Vec::new(),
                    ),
                )
            }
            ir::Implementation::Field(_) => {
                unreachable!("fields have no method adapter")
            }
        }
    }

    fn receiver_comparison(
        &self,
        member: usize,
    ) -> crate::mir::ReceiverComparison {
        if self
            .tables
            .dispatch
            .iter()
            .position(|member| member.label == "eq")
            == Some(member)
        {
            crate::mir::ReceiverComparison::Eq
        } else if self
            .tables
            .dispatch
            .iter()
            .position(|member| member.label == "cmp")
            == Some(member)
        {
            crate::mir::ReceiverComparison::Cmp
        } else {
            crate::mir::ReceiverComparison::Other
        }
    }

    pub(super) fn complete_receiver_adapters(&mut self) {
        let adapters: Vec<_> = self
            .receiver_adapters
            .iter()
            .filter(|adapter| self.bodies[adapter.body].blocks.is_empty())
            .cloned()
            .collect();
        for adapter in adapters {
            let row =
                self.tables.dispatch[adapter.member].rows[adapter.row].clone();
            let node = self.bodies[adapter.body].node;
            let declared =
                matches!(&row.target, ir::Implementation::Declared(_));
            let (parameters, mut result, mut hidden, target) = self
                .receiver_adapter_prototype(
                    row.target,
                    &row.ty,
                    row.declared_arity,
                );
            let input_parameters =
                row.input_contract.map_or_else(Vec::new, |entry| {
                    let predicate =
                        self.tables.entries[entry].predicate.clone();
                    self.reach_predicate(&predicate);
                    input_parameters(&predicate)
                });
            for parameter in &input_parameters {
                if !hidden.contains(parameter) {
                    hidden.push(*parameter);
                }
            }
            let mut used = type_parameters(&row.ty);
            used.extend(hidden.iter().copied());
            for parameter in &parameters {
                used.extend(type_parameters(parameter));
            }
            used.extend(type_parameters(&result));
            let fresh = (0..=used.len())
                .find(|id| !used.contains(id))
                .expect("a fresh parameter");
            let mut witnesses = alloc::vec![fresh];
            witnesses.extend(hidden);
            let protocol_result = if declared {
                used.push(fresh);
                let result_id = (0..=used.len())
                    .find(|id| !used.contains(id))
                    .expect("a fresh result parameter");
                result = ir::Type::Parameter(result_id);
                let position = witnesses.len();
                witnesses.push(result_id);
                Some(crate::mir::ReceiverResult {
                    meaning: position,
                    arity: parameters.len(),
                })
            } else {
                None
            };
            let mut builder =
                Builder::new(self, node, result, witnesses, Vec::new());
            builder.spot(None, ir::Type::Parameter(fresh));
            for (spot, ty) in parameters.iter().enumerate() {
                if spot != adapter.receiver_spot {
                    builder.spot(None, ty.clone());
                }
            }
            let mut args: Vec<_> =
                (2..=parameters.len()).map(Operand::Local).collect();
            args.insert(adapter.receiver_spot, Operand::Local(1));
            builder.apply(
                target,
                Arguments {
                    args,
                    receiver: None,
                    argument: None,
                },
                0,
                node,
            );
            builder.terminate(Terminator::Return, node);
            builder.finish(Some(adapter.body));
            self.receiver_contexts.resize(self.bodies.len(), None);
            self.receiver_contexts[adapter.body] =
                Some(crate::mir::ReceiverContext {
                    input_contract: row.input_contract,
                    input_parameters,
                    receiver_meaning: 0,
                    protocol_result,
                    comparison: self.receiver_comparison(adapter.member),
                });
        }
        self.receiver_contexts.resize(self.bodies.len(), None);
    }
}

/// Deterministic parameter order, without expanding generic templates.
pub(super) fn type_parameters(ty: &ir::Type) -> Vec<usize> {
    let mut found = Vec::new();
    let mut pending = alloc::vec![ty];
    while let Some(ty) = pending.pop() {
        match ty {
            ir::Type::Parameter(id) => {
                if !found.contains(id) {
                    found.push(*id);
                }
            }
            ir::Type::Meaning { arguments, .. }
            | ir::Type::Tuple(arguments)
            | ir::Type::Packed(arguments)
            | ir::Type::All(arguments)
            | ir::Type::Sum(arguments) => {
                pending.extend(arguments.iter().rev());
            }
            ir::Type::Array { element, length } => {
                pending.push(length);
                pending.push(element);
            }
            ir::Type::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            ir::Type::Not(inner) => pending.push(inner),
            ir::Type::Unit | ir::Type::Number(_) | ir::Type::Session(_) => (),
        }
    }
    found
}

/// Exactly the meaning variables an input Function entry will execute.
pub(super) fn input_parameters(predicate: &ir::Predicate) -> Vec<usize> {
    let mut found = Vec::new();
    let mut pending = alloc::vec![(predicate, true)];
    while let Some((part, entry)) = pending.pop() {
        if entry {
            if let ir::Predicate::Function { parameter, result } = part {
                pending.extend([
                    (result.as_ref(), true),
                    (
                        parameter.as_ref(),
                        matches!(
                            parameter.as_ref(),
                            ir::Predicate::Function { .. }
                        ),
                    ),
                ]);
            }
            continue;
        }
        match part {
            ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                pending.extend(parts.iter().rev().map(|part| (part, false)));
            }
            ir::Predicate::Not(inner) => pending.push((inner, false)),
            ir::Predicate::Test { meanings, .. } => {
                for ty in meanings {
                    for id in type_parameters(ty) {
                        if !found.contains(&id) {
                            found.push(id);
                        }
                    }
                }
            }
            ir::Predicate::Function { .. } | ir::Predicate::Nonzero { .. } => {}
        }
    }
    found
}
