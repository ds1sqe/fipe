//! Calls: supplies, defaults, element supplies, and the callee a call
//! hands its arguments to; open spots and bare members are thunks in
//! `thunks`, a known receiver's row is selected in `dispatch`.
use super::{builder::Builder, thunks::argument_spot};
use crate::{
    ir,
    mir::{Callee, Operand, Rvalue, Terminator},
};
use alloc::vec::Vec;

/// What a callee node resolves to before its arguments are known.
pub(super) enum Target {
    /// A callee fixed at compile time, with its witnesses.
    Static(Callee, Vec<ir::Type>),
    /// A constructor: `fields[spot]` is the physical field each spot fills.
    Construct {
        meaning: usize,
        constructor: usize,
        fields: Vec<usize>,
    },
    /// A member resolved by its receiver: at compile time when the
    /// receiver is concrete, else at run time.
    Dispatch {
        member: usize,
        receiver: ir::Receiver,
        /// Full checked callable, in source argument order.
        signature: crate::value::resource::Forest,
    },
    /// A function value.
    Value(Operand),
    /// A conversion adapter, by its node.
    Convert(usize),
}

/// What a call hands its callee: the spots in order, the receiver's
/// operand for a value dispatch, and the overload argument's type.
pub(super) struct Arguments {
    pub(super) args: Vec<Operand>,
    pub(super) receiver: Option<Operand>,
    pub(super) argument: Option<ir::Type>,
}

/// A call node's recorded parts.
pub(super) struct Site<'a> {
    pub(super) node: usize,
    pub(super) callee: usize,
    pub(super) slots: &'a [usize],
    pub(super) supplies: &'a [ir::Supply],
    pub(super) defaults: &'a [(usize, usize)],
    pub(super) open: &'a [usize],
}

impl Site<'_> {
    /// The spots filled by a supply: not a default and not left open.
    pub(super) fn supplied(&self) -> Vec<usize> {
        (0..self.slots.len())
            .filter(|spot| {
                !self.open.contains(spot)
                    && self
                        .defaults
                        .iter()
                        .all(|(defaulted, _)| defaulted != spot)
            })
            .collect()
    }

    /// The spot a value receiver was supplied to.
    fn receiver_spot(&self, target: &Target) -> Option<usize> {
        match target {
            Target::Dispatch {
                receiver: ir::Receiver::Value(value),
                ..
            } => Some(
                self.supplies
                    .iter()
                    .find(|supply| supply.value == *value)
                    .map(|supply| supply.spot)
                    .expect("a receiver is a supply of its call"),
            ),
            _ => None,
        }
    }

    /// The types of the open spots in order, then the result.
    pub(super) fn open_signature(
        &self,
        ty: &ir::Type,
    ) -> (Vec<ir::Type>, ir::Type) {
        let mut ty = ty;
        let mut parameters = Vec::with_capacity(self.open.len());
        for _ in self.open {
            let ir::Type::Function { parameter, result } = ty else {
                unreachable!("an open spot is a parameter")
            };
            parameters.push((**parameter).clone());
            ty = result;
        }
        (parameters, ty.clone())
    }
}

impl Builder<'_, '_> {
    /// Resolve a callee node; a static one is counted here, a value is
    /// lowered.
    pub(super) fn callee(&mut self, node: usize) -> Target {
        let tables = self.tables();
        match &tables.nodes[node].kind {
            ir::Kind::Definition { index, meanings } => {
                self.count(node);
                if let Some(body) = self.lowering.function_body(*index) {
                    Target::Static(Callee::Body(body), meanings.clone())
                } else if self.lowering.generic(*index) {
                    let ty = tables.nodes[node].ty.clone();
                    let value =
                        self.generic_value(*index, meanings, &ty, node, None);
                    Target::Value(value)
                } else {
                    self.lowering.reach_global(*index);
                    Target::Value(Operand::Global(*index))
                }
            }
            ir::Kind::Declared(index) => {
                self.count(node);
                Target::Static(self.declared(*index), Vec::new())
            }
            ir::Kind::Derived { kind, ty } => {
                self.count(node);
                let ty = self.lowering.fixed(ty);
                Target::Static(self.derived(*kind, ty), Vec::new())
            }
            ir::Kind::Constructor {
                meaning,
                constructor,
                fields,
            } => {
                self.count(node);
                Target::Construct {
                    meaning: *meaning,
                    constructor: *constructor,
                    fields: fields.clone(),
                }
            }
            ir::Kind::Dispatch { receiver, member } => {
                self.count(node);
                Target::Dispatch {
                    member: *member,
                    receiver: receiver.clone(),
                    signature: crate::value::resource::Forest(alloc::vec![
                        tables.nodes[node].ty.clone()
                    ]),
                }
            }
            ir::Kind::Convert { .. } => {
                self.count(node);
                Target::Convert(node)
            }
            _ => Target::Value(self.lower(node)),
        }
    }

    /// The prelude's declarations are native; any other namespace's are
    /// the host's.
    pub(super) fn declared(&mut self, declaration: usize) -> Callee {
        if self.lowering.native(declaration) {
            let label = &self.tables().declarations[declaration].label;
            let walked = super::Lowering::walked_members(label);
            self.lowering.reach_members(walked);
            Callee::Native(declaration)
        } else {
            Callee::Host(declaration)
        }
    }

    /// A derived operation walks its value by tag, so the written rows
    /// of its member are reached.
    pub(super) fn derived(
        &mut self,
        kind: ir::Derived,
        ty: ir::Type,
    ) -> Callee {
        let member = match kind {
            ir::Derived::Eq => "eq",
            ir::Derived::Cmp => "cmp",
            ir::Derived::Show => "show",
            ir::Derived::Default => "default",
        };
        self.lowering.reach_members(&[member]);
        Callee::Derived { kind, ty }
    }

    /// A call with every spot in hand: the terminator, the construction
    /// or the conversion.
    pub(super) fn apply(
        &mut self,
        target: Target,
        mut arguments: Arguments,
        dest: usize,
        node: usize,
    ) -> Operand {
        let (callee, meanings) = match target {
            Target::Static(callee, meanings) => (callee, meanings),
            Target::Construct {
                meaning,
                constructor,
                fields,
            } => {
                let count = self.tables().meanings[meaning].constructors
                    [constructor]
                    .fields
                    .len();
                let mut placed: Vec<Option<Operand>> = alloc::vec![None; count];
                for (spot, arg) in arguments.args.into_iter().enumerate() {
                    placed[fields[spot]] = Some(arg);
                }
                let fields = placed
                    .into_iter()
                    .map(|field| field.expect("every field has a spot"))
                    .collect();
                let rvalue = Rvalue::Construct {
                    meaning,
                    constructor,
                    fields,
                };
                self.assign(dest, rvalue, node);
                return Operand::Local(dest);
            }
            Target::Dispatch {
                member,
                receiver: by,
                signature,
            } => {
                let resolved = self.dispatched(
                    member,
                    &by,
                    signature,
                    &mut arguments,
                    dest,
                    node,
                );
                let Some(resolved) = resolved else {
                    return Operand::Local(dest);
                };
                resolved
            }
            Target::Value(operand) => (Callee::Value(operand), Vec::new()),
            Target::Convert(conversion) => {
                let [value] = arguments.args.as_slice() else {
                    unreachable!("a conversion takes one value")
                };
                return self.convert(conversion, *value, dest);
            }
        };
        let target = self.open(node);
        self.terminate(
            Terminator::Call {
                callee,
                args: arguments.args,
                meanings,
                dest,
                target,
            },
            node,
        );
        self.switch_to(target);
        Operand::Local(dest)
    }

    fn lower_supplies(
        &mut self,
        slots: &[usize],
        supplies: &[ir::Supply],
        open: &[usize],
        target: &Target,
        node: usize,
    ) -> Vec<Option<usize>> {
        let locals: Vec<Option<usize>> = (0..slots.len())
            .map(|spot| {
                let filled = open.is_empty()
                    || supplies.iter().any(|supply| supply.spot == spot);
                filled.then(|| self.slot_local(slots[spot]))
            })
            .collect();
        let local = |spot: usize| locals[spot].expect("a filled spot");
        let mut index = 0;
        while index < supplies.len() {
            let supply = &supplies[index];
            if supply.element {
                let mut elements = Vec::new();
                while index < supplies.len()
                    && supplies[index].element
                    && supplies[index].spot == supply.spot
                {
                    elements.push(self.lower(supplies[index].value));
                    index += 1;
                }
                self.assign(local(supply.spot), Rvalue::Array(elements), node);
            } else {
                self.lower_to(supply.value, Some(local(supply.spot)));
                index += 1;
            }
            if let Target::Value(Operand::Local(private)) = *target {
                let argument = local(supply.spot);
                let tuple_ty = ir::Type::Tuple(alloc::vec![
                    self.locals[private].ty.clone(),
                    self.locals[argument].ty.clone()
                ]);
                let admitted = self.local(tuple_ty, None);
                self.assign(
                    admitted,
                    Rvalue::Admit {
                        values: [
                            Operand::Local(private),
                            Operand::Local(argument),
                        ],
                        spot: supply.spot,
                    },
                    node,
                );
                self.assign(
                    private,
                    Rvalue::Field(Operand::Local(admitted), 0),
                    node,
                );
                self.assign(
                    argument,
                    Rvalue::Field(Operand::Local(admitted), 1),
                    node,
                );
            }
        }
        locals
    }

    /// A call node: supplies in source order into the call's locals, then
    /// the defaults in recorded order, then the call; open spots make a
    /// thunk.
    pub(super) fn call(
        &mut self,
        site: &Site<'_>,
        dest: Option<usize>,
    ) -> Operand {
        let Site {
            node,
            callee,
            slots,
            supplies,
            open,
            ..
        } = *site;
        if slots.is_empty() {
            self.count(node);
            return self.lower_to(callee, dest);
        }
        let ty = &self.tables().nodes[node].ty;
        let bound_member = matches!(
            &self.tables().nodes[callee].kind,
            ir::Kind::Dispatch {
                receiver: ir::Receiver::Value(value),
                ..
            } if !supplies.iter().any(|supply| supply.value == *value)
        );
        // A member already bound its receiver before this call. Lower that
        // member value through its capture thunk, rather than expect the
        // receiver among this call's remaining supplies.
        let mut target = if bound_member {
            Target::Value(self.lower(callee))
        } else {
            self.callee(callee)
        };
        if let Target::Dispatch { signature, .. } = &mut target {
            // The actual Site, including synthetic operators and applied inner
            // calls, owns the supplied formal types and residual result.
            let (_, result) = site.open_signature(ty);
            *signature = crate::value::resource::Forest(alloc::vec![slots
                .iter()
                .rev()
                .fold(result, |result, slot| {
                    ir::Type::Function {
                        parameter: alloc::boxed::Box::new(
                            self.tables().slots[*slot].ty.clone(),
                        ),
                        result: alloc::boxed::Box::new(result),
                    }
                })]);
        }
        if let Target::Value(value) = target {
            let private =
                self.local(self.tables().nodes[callee].ty.clone(), None);
            self.assign(private, Rvalue::Use(value), node);
            target = Target::Value(Operand::Local(private));
        }
        // A spot gets a local here when this body fills it: every spot of
        // a full call, only the supplied ones when the rest go to a thunk.
        let locals = self.lower_supplies(slots, supplies, open, &target, node);
        let local = |spot: usize| locals[spot].expect("a filled spot");
        let receiver_spot = site.receiver_spot(&target);
        let argument = argument_spot(slots.len(), receiver_spot).map(|spot| {
            self.lowering.fixed(&self.tables().slots[slots[spot]].ty)
        });
        // A value callee already owns its raw arity. Supplying a complete
        // prefix must enter those arguments now, even when its result is partial.
        let prefix = slots.len() - open.len();
        let prefix_value = prefix > 0
            && matches!(target, Target::Value(_))
            && site.defaults.is_empty()
            && !open.is_empty()
            && open.iter().copied().eq(prefix..slots.len())
            && supplies.len() == prefix
            && supplies.iter().all(|supply| !supply.element)
            && (0..prefix)
                .all(|spot| supplies.iter().any(|supply| supply.spot == spot));
        if prefix_value {
            self.count(node);
            let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
            let arguments = Arguments {
                args: (0..prefix)
                    .map(|spot| Operand::Local(local(spot)))
                    .collect(),
                receiver: None,
                argument,
            };
            return self.apply(target, arguments, dest, node);
        }
        if open.is_empty() {
            for (spot, default) in site.defaults {
                self.lower_to(*default, Some(local(*spot)));
            }
            self.count(node);
            let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
            let args = (0..slots.len())
                .map(|spot| Operand::Local(local(spot)))
                .collect();
            let receiver =
                receiver_spot.map(|spot| Operand::Local(local(spot)));
            let arguments = Arguments {
                args,
                receiver,
                argument,
            };
            return self.apply(target, arguments, dest, node);
        }
        let closure =
            self.thunk(site, &locals, target, receiver_spot, argument);
        self.count(node);
        self.materialize(closure, dest, ty, node)
    }
}
