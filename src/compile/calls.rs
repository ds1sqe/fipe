//! Calls: supplies, defaults, element supplies, open spots as thunks.
use super::builder::Builder;
use crate::{
    ir,
    mir::{self, Callee, Operand, Rvalue, Terminator},
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
    /// A member resolved at run time by its receiver.
    Dispatch {
        member: usize,
        receiver: ir::Receiver,
    },
    /// A function value.
    Value(Operand),
    /// A conversion adapter, by its node.
    Convert(usize),
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
    fn supplied(&self) -> Vec<usize> {
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
    fn open_signature(&self, ty: &ir::Type) -> (Vec<ir::Type>, ir::Type) {
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
                self.count();
                self.lowering
                    .function_body(*index)
                    .map_or(Target::Value(Operand::Global(*index)), |body| {
                        Target::Static(Callee::Body(body), meanings.clone())
                    })
            }
            ir::Kind::Declared(index) => {
                self.count();
                Target::Static(self.declared(*index), Vec::new())
            }
            ir::Kind::Constructor {
                meaning,
                constructor,
                fields,
            } => {
                self.count();
                Target::Construct {
                    meaning: *meaning,
                    constructor: *constructor,
                    fields: fields.clone(),
                }
            }
            ir::Kind::Dispatch { receiver, member } => {
                self.count();
                Target::Dispatch {
                    member: *member,
                    receiver: receiver.clone(),
                }
            }
            ir::Kind::Derived { .. } => {
                todo!("08.1 phase 2: a derived operation as a callee")
            }
            ir::Kind::Convert { .. } => {
                self.count();
                Target::Convert(node)
            }
            _ => Target::Value(self.lower(node)),
        }
    }

    /// The prelude's declarations are native; any other namespace's are
    /// the host's.
    pub(super) fn declared(&self, declaration: usize) -> Callee {
        if self.lowering.native(declaration) {
            Callee::Native(declaration)
        } else {
            Callee::Host(declaration)
        }
    }

    /// A call with every spot in hand: the terminator, the construction
    /// or the conversion.
    pub(super) fn apply(
        &mut self,
        target: Target,
        args: Vec<Operand>,
        receiver: Option<Operand>,
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
                for (spot, arg) in args.into_iter().enumerate() {
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
            } => {
                let witness = match by {
                    ir::Receiver::Value(_) => mir::Witness::Value(
                        receiver.expect("a value receiver is supplied"),
                    ),
                    ir::Receiver::Meaning(ty) => mir::Witness::Meaning(ty),
                };
                (Callee::Dispatch { member, witness }, Vec::new())
            }
            Target::Value(operand) => (Callee::Value(operand), Vec::new()),
            Target::Convert(conversion) => {
                let [value] = args.as_slice() else {
                    unreachable!("a conversion takes one value")
                };
                return self.convert(conversion, *value, dest);
            }
        };
        let target = self.open(node);
        self.terminate(
            Terminator::Call {
                callee,
                args,
                meanings,
                dest,
                target,
            },
            node,
        );
        self.switch_to(target);
        Operand::Local(dest)
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
        self.count();
        if slots.is_empty() {
            return self.lower_to(callee, dest);
        }
        let ty = &self.tables().nodes[node].ty;
        let target = self.callee(callee);
        // A spot gets a local here when this body fills it: every spot of
        // a full call, only the supplied ones when the rest go to a thunk.
        let locals: Vec<Option<usize>> = (0..slots.len())
            .map(|spot| {
                let filled = open.is_empty()
                    || supplies.iter().any(|supply| supply.spot == spot);
                filled.then(|| self.slot_local(slots[spot], node))
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
        }
        let receiver_spot = site.receiver_spot(&target);
        if open.is_empty() {
            for (spot, default) in site.defaults {
                self.lower_to(*default, Some(local(*spot)));
            }
            let dest = dest.unwrap_or_else(|| self.local(ty.clone(), None));
            let args = (0..slots.len())
                .map(|spot| Operand::Local(local(spot)))
                .collect();
            let receiver =
                receiver_spot.map(|spot| Operand::Local(local(spot)));
            return self.apply(target, args, receiver, dest, node);
        }
        let closure = self.thunk(site, &locals, target, receiver_spot);
        self.materialize(closure, dest, ty, node)
    }

    /// The function of a call's open spots: its captures are the
    /// supplied values, its spots the open ones in recorded order; the
    /// defaults run when it is called, reading both.
    fn thunk(
        &mut self,
        site: &Site<'_>,
        locals: &[Option<usize>],
        target: Target,
        receiver_spot: Option<usize>,
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
        let args = inner.iter().copied().map(Operand::Local).collect();
        let receiver = receiver_spot.map(|spot| Operand::Local(inner[spot]));
        builder.apply(target, args, receiver, 0, node);
        builder.terminate(Terminator::Return, node);
        let body = builder.finish(None);
        Rvalue::Closure {
            body,
            captures,
            meanings: self.inherited(),
        }
    }
}
