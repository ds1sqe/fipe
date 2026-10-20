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
        let mut requests = self.dispatch_requests();
        for request in self.packed_members() {
            if !requests.contains(&request) {
                requests.push(request);
            }
        }
        if requests.is_empty() {
            return Vec::new();
        }
        // Receivers are kept up to the names of their own variables: the
        // rows for `Maybe<a>` and `Maybe<b>` are the same rows, and a unit
        // of N arm lists over an unsolved input would otherwise try every
        // request N times over a state that grows with N. A session's
        // weak variable names a slot a later line may fix, so a receiver
        // over one stays apart.
        let weak = self.weak;
        let mut receivers: Vec<Type> = Vec::new();
        let mut keys: Vec<Type> = Vec::new();
        for shape in shapes {
            keep_receiver(&mut receivers, &mut keys, shape.ty.clone(), weak);
        }
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
            ) {
                keep_receiver(&mut receivers, &mut keys, ty, weak);
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

    /// The members a value packed here can be asked for: each member
    /// limit of the packed meaning, its bases' included. A later line
    /// reaches the value through the packed type alone, which no longer
    /// names its tag, so the rows for the tag are recorded where it packs.
    fn packed_members(&mut self) -> Vec<(alloc::string::String, bool)> {
        let targets: Vec<Type> = self
            .state
            .casts
            .iter()
            .flatten()
            .filter(|cast| cast.row == crate::typed::CastRow::Pack)
            .map(|cast| super::meaning(&self.apply(&cast.to)).clone())
            .fold(Vec::new(), |mut targets, target| {
                // A meaning packed at many places asks for its limits once.
                if !targets.contains(&target) {
                    targets.push(target);
                }
                targets
            });
        let mut members = Vec::new();
        for target in targets {
            let limits = self.member_limits(
                &target,
                &target,
                crate::token::Span::default(),
                &mut Vec::new(),
            );
            for limit in limits {
                if let crate::types::Obligation::Member { name, .. } = limit {
                    if !members.contains(&(name.clone(), false)) {
                        members.push((name, false));
                    }
                }
            }
        }
        members
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
                Selected::Derived { kind, .. } => derived_member(*kind),
                _ => continue,
            };
            if !requests.contains(&request) {
                requests.push(request);
            }
        }
        // A prelude word that walks its argument (`show`, `sort`, `sum`)
        // performs a member of each value it meets; a backend finds a
        // written one through these rows, never by a label.
        for (site, selected) in &self.state.selected {
            let Selected::Declared {
                origin: crate::resolve::Origin::Prelude,
                ..
            } = selected
            else {
                continue;
            };
            let Some(word) = self.module.expressions.iter().find_map(|expr| {
                // `read<Point>` is the word with type arguments.
                match &expr.kind {
                    crate::ast::ExprKind::Name(name)
                    | crate::ast::ExprKind::Instance { name, .. }
                        if (expr.span.start, expr.span.end) == *site =>
                    {
                        name.segments.last()
                    }
                    _ => None,
                }
            }) else {
                continue;
            };
            for request in walked_members(word) {
                let request =
                    (alloc::string::String::from(request.0), request.1);
                if !requests.contains(&request) {
                    requests.push(request);
                }
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
        let saved = self.snapshot();
        let rows = self.dispatch_candidates(receiver, name, receiverless);
        let count = self.state.subst.count();
        self.restore(saved);
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

/// Add `ty` to the receivers unless one differing only by the names of
/// its own variables is there; `keys` holds each kept receiver renamed.
fn keep_receiver(
    receivers: &mut Vec<Type>,
    keys: &mut Vec<Type>,
    ty: Type,
    weak: u32,
) {
    let key = renamed(&ty, weak);
    if !keys.contains(&key) {
        keys.push(key);
        receivers.push(ty);
    }
}

/// `ty` with its variables from `weak` on numbered from `weak` in order
/// of appearance; the session's weak variables below it keep their names.
fn renamed(ty: &Type, weak: u32) -> Type {
    let mut vars = Vec::new();
    ty.variables(&mut vars);
    let names: Vec<_> = vars
        .iter()
        .filter(|var| var.0 >= weak)
        .enumerate()
        .map(|(index, var)| {
            let fresh = u32::try_from(index)
                .map_or(u32::MAX, |index| weak.saturating_add(index));
            (*var, Type::Var(crate::types::Var(fresh)))
        })
        .collect();
    crate::types::replace(ty, &names)
}

/// The member a derived operation performs on each part it meets.
fn derived_member(kind: crate::ir::Derived) -> (alloc::string::String, bool) {
    let (name, receiverless) = match kind {
        crate::ir::Derived::Eq => ("eq", false),
        crate::ir::Derived::Cmp => ("cmp", false),
        crate::ir::Derived::Show => ("show", false),
        crate::ir::Derived::Default => ("default", true),
    };
    (alloc::string::String::from(name), receiverless)
}

/// The members a prelude word performs on the values it walks, by the
/// claims of its declaration in `prelude/prelude.fipe`.
fn walked_members(word: &str) -> &'static [(&'static str, bool)] {
    match word {
        "show" => &[("show", false)],
        "len" => &[("len", false)],
        "read" => &[("read", true)],
        "concat" => &[("concat", false)],
        "sum" => &[("add", false), ("zero", true)],
        "count" => &[("eq", false)],
        "sort" => &[("cmp", false)],
        _ => &[],
    }
}
