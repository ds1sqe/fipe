//! A dot reads a field, then a member of the head's meaning. Unknown
//! heads carry the same lookup as a member obligation; selectors do too.

use super::{
    arms::heads_in, lower::Scope, lower::SelfKind, lower::Vars, Callee,
    Expected, Infer, Key, Owed, Reason,
};
use crate::{
    ast::{Entry, ExprKind, ImplMember, Module, Name, PatternKind},
    resolve::{Origin, Target},
    token::Span,
    typed::Typed,
    types::{Obligation, Scheme, Subst, Type},
};
use alloc::{string::String, vec::Vec};

impl<'a> Infer<'a> {
    /// `head.a.b`: each member of the value before it.
    pub(super) fn members(
        &mut self,
        head: Type,
        members: &[String],
        span: Span,
    ) -> Callee {
        let mut current = head;
        for (index, member) in members.iter().enumerate() {
            let applied = super::meaning(&self.apply(&current)).clone();
            if matches!(applied, Type::Var(_) | Type::Rigid(_)) {
                current = self.fresh();
                let prior =
                    self.state.obligations.iter().rev().find_map(|o| match &o
                        .obligation
                    {
                        Obligation::Member { head, name, ty, .. }
                            if !o.solved
                                && name == member
                                && self.apply(head) == applied =>
                        {
                            Some(ty.clone())
                        }
                        _ => None,
                    });
                if let Some(prior) = prior {
                    self.unify(&prior, &current, span, span);
                } else {
                    self.require_member(applied, member, current.clone(), span);
                }
                continue;
            }
            if let Some(ty) = self.field(&applied, member) {
                current = ty;
                continue;
            }
            if self.private_field(&applied, member) {
                self.error(
                    span,
                    Reason::PrivateField {
                        field: member.clone(),
                    },
                );
                return Callee::plain(self.fresh());
            }
            let Some(mut callee) = self.self_function(member, &applied, span)
            else {
                let reason = if self.hidden_member(&applied, member) {
                    Reason::PrivateMember {
                        member: member.clone(),
                    }
                } else {
                    Reason::NoField {
                        ty: applied,
                        other: span,
                    }
                };
                self.error(span, reason);
                return Callee::plain(self.fresh());
            };
            let slot = callee.self_slot.unwrap_or_default();
            callee.supplied = Some((slot, applied, span));
            if index + 1 == members.len() {
                return callee;
            }
            current = self.apply_callee(callee, &[], None, span, None);
        }
        Callee::plain(current)
    }

    /// Resolve the member obligations whose head is known.
    pub(super) fn dots(&mut self) {
        if self.state.solving_dots {
            return;
        }
        self.state.solving_dots = true;
        loop {
            let mut ready = Vec::new();
            while self.state.dots_start < self.state.obligations.len()
                && self.state.obligations[self.state.dots_start].solved
            {
                self.state.dots_start += 1;
            }
            for index in self.state.dots_start..self.state.obligations.len() {
                if self.state.obligations[index].solved {
                    continue;
                }
                let owed = self.state.obligations[index].clone();
                let head = self.apply(owed.obligation.subject());
                if matches!(head, Type::Var(_)) {
                    continue;
                }
                let waiting = match &owed.obligation {
                    Obligation::Claim { class, .. } => {
                        let class = self.apply(class);
                        matches!(&class, Type::Named { name, arguments } if arguments.first().is_some_and(|a| matches!(a, Type::Var(_))) && self.claims(&head, true).iter().filter(|c| matches!(c, Type::Named { name: n, .. } if n == name)).count() > 1)
                    }
                    Obligation::Member { name, ty, .. } => {
                        self.member_choices(&head, name).len() > 1
                            && match self.apply(ty) {
                                Type::Function { parameter, .. } => {
                                    matches!(*parameter, Type::Var(_))
                                }
                                Type::Var(_) => true,
                                _ => false,
                            }
                    }
                };
                if !waiting {
                    ready.push(index);
                }
            }
            if ready.is_empty() {
                self.state.solving_dots = false;
                return;
            }
            for index in ready {
                // Solving applies callees, which may solve this one first.
                let owed = &mut self.state.obligations[index];
                if !owed.solved {
                    owed.solved = true;
                    let owed = owed.clone();
                    self.member(&owed);
                }
            }
        }
    }

    /// `head.name: ty` on a known head: its field, else the remaining
    /// application of a `self`-spot function to it.
    fn member(&mut self, owed: &Owed) {
        let Obligation::Member {
            head,
            name,
            ty,
            dot,
        } = &owed.obligation
        else {
            self.claim(owed);
            return;
        };
        let original = self.apply(head);
        let (head, dot) = (
            super::meaning(&original).clone(),
            if *dot == Span::default() {
                owed.span
            } else {
                *dot
            },
        );
        if matches!(head, Type::Rigid(_)) {
            let supplied = self.assumed_member(&head, name, dot);
            if let Some(supplied) = supplied {
                self.unify(ty, &supplied, dot, owed.span);
                return;
            }
            self.error(
                dot,
                Reason::NoField {
                    ty: head,
                    other: owed.span,
                },
            );
            return;
        }
        if let Some(literal) = owed.literal.clone() {
            if let Type::Named { name: owner, .. } = &head {
                if owner.origin == Origin::Prelude {
                    self.check_literal(literal, &original, dot);
                    return;
                }
            }
        }
        if name == "literal" || name == "read" {
            if let Type::Named { name: owner, .. } = &head {
                let target = Target::Type {
                    origin: owner.origin.clone(),
                    name: owner.name.clone(),
                };
                let callee = self.type_member(&target, name, dot);
                self.unify(ty, &callee.whole(), dot, owed.span);
                return;
            }
        }
        if self.member_choices(&head, name).len() > 1 {
            let expected = self.apply(ty);
            let source = match &expected {
                Type::Function { parameter, .. } => (**parameter).clone(),
                _ => self.fresh(),
            };
            let selected = self.select_member(&head, name, &source, dot);
            self.unify(ty, &selected, dot, owed.span);
            return;
        }
        let value = if let Some(field) = self.field(&head, name) {
            field
        } else if self.private_field(&head, name) {
            self.error(
                dot,
                Reason::PrivateField {
                    field: name.clone(),
                },
            );
            return;
        } else if let Some(mut callee) = self.self_function(name, &head, dot) {
            let slot = callee.self_slot.unwrap_or_default();
            callee.supplied = Some((slot, head, dot));
            self.apply_callee(callee, &[], None, dot, None)
        } else if self.hidden_member(&head, name) {
            self.error(
                dot,
                Reason::PrivateMember {
                    member: name.clone(),
                },
            );
            return;
        } else {
            self.error(
                dot,
                Reason::MissingMember {
                    ty: head,
                    name: name.clone(),
                },
            );
            return;
        };
        self.unify(ty, &value, dot, owed.span);
    }

    fn assumed_member(
        &mut self,
        head: &Type,
        name: &str,
        dot: Span,
    ) -> Option<Type> {
        let assumed: Vec<_> = self
            .state
            .obligations
            .iter()
            .filter(|o| o.assumed)
            .map(|o| o.obligation.clone())
            .collect();
        let mut members = Vec::new();
        for obligation in assumed {
            match obligation {
                Obligation::Member { .. } => members.push(obligation),
                Obligation::Claim { head: h, class, .. } if h == *head => {
                    members.extend(self.member_limits(
                        &class,
                        head,
                        dot,
                        &mut Vec::new(),
                    ));
                }
                Obligation::Claim { .. } => {}
            }
        }
        members.into_iter().find_map(|o| match o {
            Obligation::Member {
                head: h,
                name: n,
                ty: t,
                ..
            } if h == *head && n == *name => Some(t),
            _ => None,
        })
    }

    /// A readable field of a record type.
    fn field(&mut self, ty: &Type, name: &str) -> Option<Type> {
        if let Some(fields) = self.fields(ty) {
            if let Some(field) =
                fields.into_iter().find(|f| f.name == name && f.public)
            {
                return Some(field.ty);
            }
        }
        self.member_limits(ty, ty, Span::default(), &mut Vec::new())
            .into_iter()
            .find_map(|o| {
                let Obligation::Member {
                    name: n,
                    ty: member,
                    ..
                } = o
                else {
                    return None;
                };
                (n == name && (name == "eq" || !contains_type(&member, ty)))
                    .then_some(member)
            })
    }

    /// A function `name` in scope with a `self` spot: this module's
    /// items, the head type's own members, those of a file module's type
    /// in its module, then the prelude and the commands.
    fn self_function(
        &mut self,
        name: &str,
        ty: &Type,
        span: Span,
    ) -> Option<Callee> {
        if let Some(member) = self.array_members(ty, Some(name), span).1 {
            return Some(member);
        }
        if self.hidden_member(ty, name) {
            return None;
        }
        if let Some(callee) = self.overloaded_member(ty, name) {
            return Some(callee);
        }
        let mut keys = Vec::new();
        if let Type::Named { name: owner, .. } = ty {
            if matches!(owner.origin, Origin::This | Origin::Line(_)) {
                keys.push(Key::Member(owner.name.clone(), String::from(name)));
            }
        }
        for key in keys {
            let Some(index) = self.item_index(&key) else {
                continue;
            };
            let has_self = self.items[index]
                .heads
                .iter()
                .flatten()
                .any(|head| head.name.as_deref() == Some("self"));
            if has_self {
                return Some(self.item_callee(index, span));
            }
        }
        // An accepted line's function, then its member of the type.
        let member = match ty {
            Type::Named { name: owner, .. } => self.frame_member(owner, name),
            _ => None,
        };
        if let Some(site) = member {
            let has_self = self
                .frame
                .and_then(|frame| frame.locals.get(&(site.start, site.end)))
                .and_then(|local| local.heads.as_ref())
                .is_some_and(|heads| {
                    heads.iter().any(|h| h.name.as_deref() == Some("self"))
                });
            if has_self {
                return Some(self.local(site, span));
            }
        }
        // A file module's type: its member, typed in that module.
        if let Type::Named { name: owner, .. } = ty {
            if let Origin::File(stem) = &owner.origin {
                let callee = self.file_member(stem, &owner.name, name, span);
                if let Some(callee) = callee.filter(|c| c.self_slot.is_some()) {
                    return Some(callee);
                }
            }
        }
        if let Type::Named { name: owner, .. } = ty {
            if let Some(callee) = self
                .supplied_member(&owner.origin, &owner.name, ty, name, span)
                .filter(|c| c.self_slot.is_some())
            {
                return Some(callee);
            }
        }
        self.derived_member(ty, name, span)
    }

    /// `.name` as a value: `a -> b` with the member obligation
    /// `a.name: b`, solved now when the expected type knows `a`.
    pub(super) fn selector(
        &mut self,
        name: &Name,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        let record = self.fresh();
        let ty = Type::function(record.clone(), self.fresh());
        if let Some(expected) = expected {
            self.hint(&expected.ty, &ty);
        }
        let callee = self.members(record, &name.segments, span);
        let value = self.apply_callee(callee, &[], None, span, None);
        let Type::Function { result, .. } = &ty else {
            unreachable!()
        };
        self.unify(result, &value, span, span);
        ty
    }

    /// `T.member`: an owned, supplied or derived member, with `Self` as `T`.
    pub(super) fn type_member(
        &mut self,
        ty: &Target,
        member: &str,
        span: Span,
    ) -> Callee {
        let named = match ty {
            Target::Type { origin, name } => {
                let count = self
                    .owner(origin)
                    .and_then(|m| self.type_item(m, name))
                    .map_or(0, |(parameters, _)| parameters.len());
                let arguments = (0..count).map(|_| self.fresh()).collect();
                Some(Type::named(origin.clone(), name, arguments))
            }
            Target::SelfType => self.self_type.clone(),
            _ => None,
        };
        let Some(named @ Type::Named { .. }) = named else {
            self.error(span, Reason::NotAFunction);
            return Callee::plain(self.fresh());
        };
        if matches!(member, "from" | "try_from") && self.claims(&named, true).iter().any(|class| matches!(class, Type::Named { name, .. } if name.origin == Origin::Prelude && name.name == (if member == "from" { "From" } else { "TryFrom" }))) {
            return self.conversion_callee(named, member);
        }
        if self.hidden_member(&named, member) {
            self.error(
                span,
                Reason::PrivateMember {
                    member: String::from(member),
                },
            );
            return Callee::plain(self.fresh());
        }
        if let Some(callee) = self.overloaded_member(&named, member) {
            return callee;
        }
        let Type::Named { name: owner, .. } = &named else {
            unreachable!()
        };
        let owner = owner.clone();
        if matches!(owner.origin, Origin::This | Origin::Line(_)) {
            let key = Key::Member(owner.name.clone(), String::from(member));
            if let Some(index) = self.item_index(&key) {
                return self.item_callee(index, span);
            }
        }
        if let Some(site) = self.frame_member(&owner, member) {
            return self.local(site, span);
        }
        if let Origin::File(stem) = &owner.origin {
            if let Some(callee) =
                self.file_member(stem, &owner.name, member, span)
            {
                return callee;
            }
        }
        self.supplied_member(&owner.origin, &owner.name, &named, member, span)
            .or_else(|| self.derived_member(&named, member, span))
            .unwrap_or_else(|| {
                let missing = self.claims(&named, false).into_iter().find(|class| self.member_limits(class,&named,span,&mut Vec::new()).iter().any(|o| matches!(o,Obligation::Member { name, .. } if name == member)));
                let reason = missing.map_or_else(|| Reason::MissingMember { ty: named.clone(), name: String::from(member) }, |claim| Reason::MissingClaim { ty: named.clone(), claim });
                self.error(span,reason);
                Callee::plain(self.fresh())
            })
    }

    pub(super) fn derived_member(
        &mut self,
        ty: &Type,
        member: &str,
        span: Span,
    ) -> Option<Callee> {
        if member == "default"
            && self.has_claim(ty, &super::prelude("Default"))
            && self.default_available(ty)
        {
            return Some(Callee::plain(ty.clone()));
        }
        if !matches!(member, "eq" | "cmp" | "show") {
            return None;
        }
        let class = super::prelude(match member {
            "eq" => "Eq",
            "cmp" => "Ord",
            _ => "Show",
        });
        if !self.has_claim(ty, &class) {
            return None;
        }
        let result = super::prelude(match member {
            "eq" => "Bool",
            "cmp" => "Ordering",
            _ => "String",
        });
        if let Type::Array { element, .. } = ty {
            self.require_claim((**element).clone(), class, span);
            return Some(Self::derived_callee(ty, result, member, span));
        }
        if let Type::Tuple(parts) = ty {
            for part in parts {
                self.require_claim(part.clone(), class.clone(), span);
            }
            return Some(Self::derived_callee(ty, result, member, span));
        }
        if *ty == Type::Unit {
            return Some(Self::derived_callee(ty, result, member, span));
        }
        let Type::Named { name, arguments } = ty else {
            return None;
        };
        let module = self.owner(&name.origin)?;
        let scope = self.scope(&name.origin)?;
        let (parameters, members) = self.type_item(module, &name.name)?;
        let mut vars = Vars::new(SelfKind::Type(ty.clone()));
        for (parameter, argument) in parameters.iter().zip(arguments) {
            vars.names
                .push((super::last(&parameter.name), argument.clone()));
        }
        let fields: Vec<_> = if let Some(members) = members {
            members
                .iter()
                .flat_map(|m| match m {
                    crate::ast::Member::Field(field) => {
                        core::slice::from_ref(field)
                    }
                    crate::ast::Member::Constructor { fields, .. } => {
                        fields.as_slice()
                    }
                })
                .map(|f| self.lower(&scope, f.ty, &mut vars))
                .collect()
        } else if matches!(name.origin, Origin::Prelude)
            && matches!(name.name.as_str(), "Vec" | "Deque")
        {
            arguments.clone()
        } else {
            return None;
        };
        for field in fields {
            let present = self.state.obligations.iter().any(|o| matches!(&o.obligation, Obligation::Claim { head, class: c, .. } if self.apply(head) == field && *c == class));
            if !present {
                self.require_claim(field, class.clone(), span);
            }
        }
        Some(Self::derived_callee(ty, result, member, span))
    }

    fn derived_callee(
        ty: &Type,
        result: Type,
        member: &str,
        span: Span,
    ) -> Callee {
        let mut spots = Vec::new();
        if member != "show" {
            spots.push(super::Slot {
                place_kind: crate::typed::Place::Parameter(None),
                name: None,
                ty: ty.clone(),
                default: false,
                value: None,
                site: span,
            });
        }
        let slot = spots.len();
        spots.push(super::Slot {
            place_kind: crate::typed::Place::Parameter(Some(String::from(
                "self",
            ))),
            name: Some(String::from("self")),
            ty: ty.clone(),
            default: false,
            value: None,
            site: span,
        });
        let mut callee = Callee::plain(result);
        callee.spots = Some(spots);
        callee.self_slot = Some(slot);
        callee
    }

    pub(super) fn conversion_members(
        &mut self,
        origin: &Origin,
        owner: &str,
        ty: &Type,
        member: &str,
        span: Span,
    ) -> Vec<Callee> {
        let mut callees = Vec::new();
        if let Origin::File(stem) = origin {
            let Some((module, typed)) = self.file_typed(stem, span) else {
                return callees;
            };
            for members in super::lower::impls(module, owner) {
                for entry in members {
                    let ImplMember::Named { public, name, .. } = entry else {
                        continue;
                    };
                    if super::last(name) != member
                        || (!*public && !module.claim_member(name.span))
                    {
                        continue;
                    }
                    if let Some((_, local)) =
                        typed.frozen.iter().find(|(site, _)| {
                            *site == (name.span.start, name.span.end)
                        })
                    {
                        let ty = self.imported(&local.scheme, stem, span);
                        callees
                            .push(self.with_heads(ty, local.heads.as_deref()));
                    }
                }
            }
        } else if matches!(origin, Origin::Line(_)) {
            for (module, _) in self.impl_scopes(origin, owner) {
                for members in super::lower::impls(module, owner) {
                    for entry in members {
                        let ImplMember::Named { name, .. } = entry else {
                            continue;
                        };
                        if super::last(name) == member
                            && self.frame.is_some_and(|f| {
                                f.locals.contains_key(&(
                                    name.span.start,
                                    name.span.end,
                                ))
                            })
                        {
                            callees.push(self.local(name.span, span));
                        }
                    }
                }
            }
        } else {
            callees = self.supplied_members(origin, owner, ty, member, span);
        }
        callees
    }

    fn supplied_member(
        &mut self,
        origin: &Origin,
        owner: &str,
        named: &Type,
        member: &str,
        span: Span,
    ) -> Option<Callee> {
        self.supplied_members(origin, owner, named, member, span)
            .into_iter()
            .next()
    }

    fn supplied_members(
        &mut self,
        origin: &Origin,
        owner: &str,
        named: &Type,
        member: &str,
        span: Span,
    ) -> Vec<Callee> {
        let mut callees = Vec::new();
        for (module, scope) in self.impl_scopes(origin, owner) {
            for members in super::lower::impls(module, owner) {
                for entry in members {
                    let ImplMember::Named {
                        name,
                        annotation: Some(ty),
                        value: None,
                        ..
                    } = entry
                    else {
                        continue;
                    };
                    if super::last(name) != member {
                        continue;
                    }
                    let mut vars = Vars::new(SelfKind::Type(named.clone()));
                    if let Type::Named { arguments, .. } = named {
                        if let Some((parameters, _)) =
                            self.type_item(module, owner)
                        {
                            for (parameter, argument) in
                                parameters.iter().zip(arguments)
                            {
                                vars.names.push((
                                    super::last(&parameter.name),
                                    argument.clone(),
                                ));
                            }
                        }
                    }
                    let (slots, result, self_slot) =
                        self.signature(&scope, *ty, &mut vars);
                    let self_slot = self_slot.or_else(|| {
                        (!matches!(
                            member,
                            "read"
                                | "literal"
                                | "from"
                                | "try_from"
                                | "pure"
                                | "empty"
                        ))
                        .then(|| slots.len().checked_sub(1))
                        .flatten()
                    });
                    let mut callee = self
                        .finish_callee(slots, result, self_slot, vars, span);
                    callee.home = Some(origin.clone());
                    callees.push(callee);
                }
            }
        }
        callees
    }

    /// The modules that may hold impls of type `ty`: its own, or for an
    /// accepted line's type, this module and the lines with its impls.
    pub(super) fn impl_scopes(
        &self,
        origin: &Origin,
        ty: &str,
    ) -> Vec<(&'a Module, Scope<'a>)> {
        let Origin::Line(line) = origin else {
            return self
                .owner(origin)
                .zip(self.scope(origin))
                .into_iter()
                .collect();
        };
        let mut scopes = alloc::vec![(self.module, Scope::This)];
        let lines = self
            .frame
            .and_then(|frame| frame.impls.get(&(*line, String::from(ty))));
        for index in lines.into_iter().flatten() {
            let origin = Origin::Line(*index);
            if let (Some(module), Some(scope)) =
                (self.owner(&origin), self.scope(&origin))
            {
                scopes.push((module, scope));
            }
        }
        scopes
    }

    /// The site of an accepted line's inherent member of its type.
    fn frame_member(
        &self,
        owner: &crate::types::TypeName,
        member: &str,
    ) -> Option<Span> {
        let Origin::Line(line) = owner.origin else {
            return None;
        };
        let key = ((line, owner.name.clone()), String::from(member));
        let (start, end) = *self.frame?.members.get(&key)?;
        Some(Span { start, end })
    }

    /// A binding of a file module: that module typed on its own, its
    /// binding's type instantiated here.
    pub(super) fn file_value(
        &mut self,
        stem: &str,
        name: &str,
        span: Span,
    ) -> Callee {
        let Some((module, typed)) = self.file_typed(stem, span) else {
            return Callee::plain(self.fresh());
        };
        // The module's final binding of the name is its export.
        let binding = module.entries.iter().rev().find_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let ExprKind::Bind { pattern, value, .. } =
                &module.expressions[id.0].kind
            else {
                return None;
            };
            match &module.patterns[pattern.0].kind {
                PatternKind::Name(n) if super::last(n) == name => {
                    Some((*id, *value))
                }
                _ => None,
            }
        });
        let Some((id, value)) = binding else {
            return Callee::plain(self.fresh());
        };
        let Some(scheme) = typed.binding(id) else {
            return Callee::plain(self.fresh());
        };
        let ty = self.imported(scheme, stem, span);
        let heads = match &module.expressions[value.0].kind {
            ExprKind::Arms(arms) => Some(heads_in(module, arms)),
            _ => None,
        };
        self.with_heads(ty, heads.as_deref())
    }

    /// Whether `ty` is a file module's type with a non-`pub` inherent
    /// member `member`: [`Self::self_function`] found no function for
    /// `member`, so a caller reports this instead of [`Reason::NoField`].
    fn hidden_member(&self, ty: &Type, member: &str) -> bool {
        let Type::Named { name: owner, .. } = ty else {
            return false;
        };
        let Origin::File(stem) = &owner.origin else {
            return false;
        };
        self.file_member_hidden(stem, &owner.name, member)
    }

    /// Whether file module `stem`'s inherent impl of `ty` names `member`
    /// without `pub`.
    fn file_member_hidden(&self, stem: &str, ty: &str, member: &str) -> bool {
        let origin = Origin::File(String::from(stem));
        let Some(module) = self.owner(&origin) else {
            return false;
        };
        super::lower::impls(module, ty).flatten().any(|m| {
            matches!(
                m,
                ImplMember::Named { public: false, name, .. }
                    if super::last(name) == member && !module.claim_member(name.span)
            )
        })
    }

    /// The `pub` inherent member `member` of type `ty` declared in file
    /// module `stem`, typed there and instantiated here.
    fn file_member(
        &mut self,
        stem: &str,
        ty: &str,
        member: &str,
        span: Span,
    ) -> Option<Callee> {
        let origin = Origin::File(String::from(stem));
        let module = self.owner(&origin)?;
        let site =
            super::lower::impls(module, ty).flatten().find_map(
                |m| match m {
                    ImplMember::Named { public, name, .. }
                        if (*public || module.claim_member(name.span))
                            && super::last(name) == member =>
                    {
                        Some(name.span)
                    }
                    _ => None,
                },
            )?;
        let Some((_, typed)) = self.file_typed(stem, span) else {
            return Some(Callee::plain(self.fresh()));
        };
        let local = typed
            .frozen
            .iter()
            .rev()
            .find(|(key, _)| *key == (site.start, site.end))
            .map(|(_, local)| local.clone());
        let Some(local) = local else {
            return Some(Callee::plain(self.fresh()));
        };
        let ty = self.imported(&local.scheme, stem, span);
        Some(self.with_heads(ty, local.heads.as_deref()))
    }

    /// File module `stem` typed on its own for a use at `span`; `None`
    /// when the use closes an import cycle, which is reported here.
    fn file_typed(
        &mut self,
        stem: &str,
        span: Span,
    ) -> Option<(&'a Module, alloc::rc::Rc<Typed>)> {
        let origin = Origin::File(String::from(stem));
        let module = self.owner(&origin)?;
        if self.visiting.iter().any(|s| s == stem) {
            self.error(span, Reason::ImportCycle);
            return None;
        }
        let checked = super::files::check_file(
            stem,
            self.declarations,
            self.files,
            self.file_cache,
            &self.visiting,
        )?;
        if checked
            .type_errors
            .iter()
            .any(|e| e.reason == Reason::ImportCycle)
        {
            self.error(span, Reason::ImportCycle);
            return None;
        }
        Some((module, checked.typed))
    }

    /// A scheme typed in file module `stem`, instantiated here: its
    /// variables are another substitution's, so all are quantified, and
    /// the module's own declarations are named by the module.
    fn imported(&mut self, scheme: &Scheme, stem: &str, span: Span) -> Type {
        let mut imported = scheme.rehome(&Origin::File(String::from(stem)));
        imported.variables = Scheme::generalize(
            &imported.ty,
            &imported.obligations,
            &[],
            &Subst::default(),
        )
        .variables;
        self.instantiate(&imported, span)
    }
}

pub(super) fn contains_type(ty: &Type, needle: &Type) -> bool {
    if ty == needle {
        return true;
    }
    match ty {
        Type::Function { parameter, result } => {
            contains_type(parameter, needle) || contains_type(result, needle)
        }
        Type::Named { arguments, .. }
        | Type::Sum(arguments)
        | Type::Tuple(arguments)
        | Type::Either(arguments)
        | Type::Both(arguments) => {
            arguments.iter().any(|t| contains_type(t, needle))
        }
        Type::Limited { meaning, .. } => contains_type(meaning, needle),
        Type::Array { element, length } => {
            contains_type(element, needle) || contains_type(length, needle)
        }
        _ => false,
    }
}

impl Infer<'_> {
    fn private_field(&mut self, ty: &Type, name: &str) -> bool {
        self.fields(ty).is_some_and(|fields| {
            fields.iter().any(|f| f.name == name && !f.public)
        })
    }
    pub(super) fn finish_dots(&mut self) {
        for index in self.state.dots_start..self.state.obligations.len() {
            let owed = &self.state.obligations[index];
            if !owed.solved
                && !matches!(
                    self.apply(owed.obligation.subject()),
                    Type::Var(_) | Type::Rigid(_)
                )
            {
                let owed = owed.clone();
                self.state.obligations[index].solved = true;
                self.member(&owed);
            }
        }
    }
}
