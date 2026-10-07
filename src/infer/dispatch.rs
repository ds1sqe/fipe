//! Resolve each tag-dispatched member to indexed-lowering inputs.
use super::Infer;
use crate::{typed::Shape, types::Type};
use alloc::vec::Vec;

impl Infer<'_> {
    pub(super) fn record_dispatch(
        &mut self,
        shapes: &[Shape],
    ) -> Vec<crate::typed::Dispatch> {
        if !self.state.errors.is_empty() {
            return Vec::new();
        }
        let requests = self.dispatch_requests();
        if requests.is_empty() {
            return Vec::new();
        }
        let mut receivers: Vec<_> =
            shapes.iter().map(|shape| shape.ty.clone()).collect();
        let mut pending: Vec<_> = self
            .state
            .types
            .iter()
            .flatten()
            .map(|ty| self.apply(ty))
            .collect();
        while let Some(ty) = pending.pop() {
            match &ty {
                Type::Tuple(parts)
                | Type::Either(parts)
                | Type::Both(parts)
                | Type::Sum(parts) => pending.extend(parts.iter().cloned()),
                Type::Array { element, length }
                | Type::Function {
                    parameter: element,
                    result: length,
                } => {
                    pending.push((**element).clone());
                    pending.push((**length).clone());
                }
                Type::Named { arguments, .. } => {
                    pending.extend(arguments.iter().cloned());
                }
                Type::Limited { meaning, .. } | Type::Not(meaning) => {
                    pending.push((**meaning).clone());
                }
                _ => (),
            }
            if matches!(
                ty,
                Type::Unit
                    | Type::Named { .. }
                    | Type::Tuple(_)
                    | Type::Array { .. }
            ) && !receivers.contains(&ty)
            {
                receivers.push(ty);
            }
        }
        requests
            .into_iter()
            .map(|(name, receiverless)| {
                let mut rows = Vec::new();
                for receiver in &receivers {
                    rows.extend(self.dispatch_rows(
                        receiver,
                        &name,
                        receiverless,
                    ));
                }
                crate::typed::Dispatch {
                    name,
                    receiverless,
                    rows,
                }
            })
            .collect()
    }

    fn dispatch_requests(&self) -> Vec<(alloc::string::String, bool)> {
        use crate::typed::Selected;
        let mut requests = Vec::new();
        for selected in self
            .state
            .selected
            .values()
            .chain(
                self.state
                    .pattern_equals
                    .values()
                    .map(|comparison| &comparison.selected),
            )
            .chain(self.state.casts.iter().flatten().filter_map(|cast| {
                cast.operation.as_ref().map(|operation| &operation.selected)
            }))
            .chain(
                self.state
                    .accesses
                    .values()
                    .flatten()
                    .map(|step| &step.selected),
            )
        {
            let request = match selected {
                Selected::Member { name, .. } => (name.clone(), false),
                Selected::Static { name, .. } => (name.clone(), true),
                _ => continue,
            };
            if !requests.contains(&request) {
                requests.push(request);
            }
        }
        for dispatch in self
            .frame
            .into_iter()
            .flat_map(|frame| &frame.lines)
            .flat_map(|line| &line.typed.dispatch)
            .chain(
                self.state
                    .default_instances
                    .values()
                    .flat_map(|instance| &instance.typed.dispatch),
            )
        {
            let request = (dispatch.name.clone(), dispatch.receiverless);
            if !requests.contains(&request) {
                requests.push(request);
            }
        }
        for file in self.file_cache.files.borrow().values() {
            for dispatch in &file.typed.dispatch {
                let request = (dispatch.name.clone(), dispatch.receiverless);
                if !requests.contains(&request) {
                    requests.push(request);
                }
            }
        }
        requests
    }

    fn dispatch_rows(
        &mut self,
        receiver: &Type,
        name: &str,
        receiverless: bool,
    ) -> Vec<crate::typed::DispatchRow> {
        // Candidate checks must not add obligations to the checked program.
        // ponytail: clone checker state per candidate; isolate a smaller trial state if large rowsets make it costly.
        let saved = self.state.clone();
        let rows = self.dispatch_candidates(receiver, name, receiverless);
        let count = self.state.subst.count();
        self.state = saved;
        while self.state.subst.count() < count {
            self.state.subst.fresh();
        }
        rows
    }

    fn dispatch_candidates(
        &mut self,
        receiver: &Type,
        name: &str,
        receiverless: bool,
    ) -> Vec<crate::typed::DispatchRow> {
        use crate::{
            token::Span,
            typed::{DispatchRow, Selected},
        };
        if !receiverless {
            if let Some((ty, selected @ Selected::Field { .. })) =
                self.field(receiver, name)
            {
                return alloc::vec![DispatchRow {
                    receiver: receiver.clone(),
                    receiver_spot: None,
                    argument: None,
                    selected,
                    meanings: Vec::new(),
                    ty
                }];
            }
        }
        let callee = if receiverless {
            if !matches!(receiver, Type::Named { .. }) {
                return Vec::new();
            }
            let previous = self.self_type.replace(receiver.clone());
            let callee = self.type_member(
                &crate::resolve::Target::SelfType,
                name,
                Span::default(),
            );
            self.self_type = previous;
            Some(callee)
        } else {
            self.self_function(name, receiver, Span::default())
        };
        let Some(callee) = callee else {
            return Vec::new();
        };
        let candidates = if callee.conversion.is_some() {
            if let Type::Named { name: owner, .. } = receiver {
                let mut choices = self.conversion_members(
                    &owner.origin,
                    &owner.name,
                    receiver,
                    name,
                    Span::default(),
                );
                if owner.origin == crate::resolve::Origin::This {
                    let key = super::Key::Member(
                        owner.name.clone(),
                        alloc::string::String::from(name),
                    );
                    let indices: Vec<_> = self
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(index, item)| {
                            (item.key == key).then_some(index)
                        })
                        .collect();
                    for index in indices {
                        choices.push(self.item_callee(index, Span::default()));
                    }
                }
                choices
            } else {
                Vec::new()
            }
        } else {
            alloc::vec![callee]
        };
        candidates
            .into_iter()
            .filter_map(|callee| self.dispatch_row(receiver, &callee))
            .collect()
    }

    fn dispatch_row(
        &mut self,
        receiver: &Type,
        callee: &super::Callee,
    ) -> Option<crate::typed::DispatchRow> {
        use crate::typed::DispatchRow;
        if let Some(slot) = callee
            .self_slot
            .and_then(|index| callee.spots.as_ref()?.get(index))
        {
            if self.state.subst.unify(&slot.ty, receiver).is_err() {
                return None;
            }
        }
        let selected = callee.selected.as_ref()?.applied(&self.state.subst);
        let argument = callee
            .spots
            .as_ref()
            .and_then(|spots| {
                spots
                    .iter()
                    .enumerate()
                    .find(|(index, _)| Some(*index) != callee.self_slot)
            })
            .map(|(_, slot)| self.apply(&slot.ty));
        Some(DispatchRow {
            receiver: receiver.clone(),
            receiver_spot: callee.self_slot,
            argument,
            selected,
            meanings: callee.meanings.iter().map(|ty| self.apply(ty)).collect(),
            ty: self.apply(&callee.whole()),
        })
    }
}
