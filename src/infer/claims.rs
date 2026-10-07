//! Named limits are satisfied by claims on the subject's meaning.
use super::{
    lower::{Scope, SelfKind, Vars},
    Infer,
};
use crate::{
    ast::{Entry, ItemKind, TypeKind},
    token::Span,
    types::{Obligation, Scheme, Type},
};
use alloc::vec::Vec;

impl Infer<'_> {
    pub(super) fn trim_claims(&mut self, scheme: &mut Scheme) {
        for obligation in &mut scheme.obligations {
            if let Obligation::Claim { head, class, .. } = obligation {
                *class = self.claim_type(class, head, true);
            }
        }
        let claims: Vec<_> = scheme
            .obligations
            .iter()
            .filter_map(|o| match o {
                Obligation::Claim { head, class, .. } => {
                    let mut parts = Vec::new();
                    self.claim_parts(class, head, &mut parts);
                    Some((head.clone(), class.clone(), parts))
                }
                Obligation::Member { .. } => None,
            })
            .collect();
        scheme.obligations.retain(|o| match o {
            Obligation::Claim { head, class, .. } => {
                !claims.iter().any(|(h, c, parts)| {
                    h == head
                        && c != class
                        && parts.contains(&self.claim_type(class, head, false))
                })
            }
            Obligation::Member { .. } => true,
        });
    }

    fn claim_type(&mut self, class: &Type, head: &Type, trim: bool) -> Type {
        let Type::Named { name, arguments } = class else {
            return class.clone();
        };
        let Some(module) = self.owner(&name.origin) else {
            return class.clone();
        };
        let Some(scope) = self.scope(&name.origin) else {
            return class.clone();
        };
        let Some((parameters, _)) = self.type_item(module, &name.name) else {
            return class.clone();
        };
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        let mut args = Vec::new();
        let mut defaults = Vec::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let default =
                parameter.default.map(|d| self.lower(&scope, d, &mut vars));
            let argument = arguments
                .get(index)
                .cloned()
                .or_else(|| default.clone())
                .unwrap_or_else(|| self.fresh());
            vars.names
                .push((super::last(&parameter.name), argument.clone()));
            args.push(argument);
            defaults.push(default);
        }
        if trim {
            while args
                .last()
                .zip(defaults.last())
                .is_some_and(|(a, d)| d.as_ref() == Some(a))
            {
                args.pop();
                defaults.pop();
            }
        }
        Type::named(name.origin.clone(), &name.name, args)
    }

    /// A combination also claims each of its named parts.
    fn claim_parts(
        &mut self,
        class: &Type,
        head: &Type,
        found: &mut Vec<Type>,
    ) {
        let class = self.claim_type(class, head, false);
        if found.contains(&class) {
            return;
        }
        found.push(class.clone());
        let Type::Named { name, arguments } = &class else {
            return;
        };
        let Some(module) = self.owner(&name.origin) else {
            return;
        };
        let Some(scope) = self.scope(&name.origin) else {
            return;
        };
        for entry in self.type_entries(module, &name.name) {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Type {
                name: n,
                parameters,
                base: Some(base),
                members: None,
                ..
            } = &item.kind
            else {
                continue;
            };
            if super::last(n) != name.name {
                continue;
            }
            let mut vars = Vars::new(SelfKind::Type(head.clone()));
            vars.names.extend(
                parameters
                    .iter()
                    .zip(arguments)
                    .map(|(p, a)| (super::last(&p.name), a.clone())),
            );
            let parts = match &module.types[base.0].kind {
                TypeKind::Both(parts) => parts.clone(),
                _ => alloc::vec![*base],
            };
            for part in parts {
                let part = self.lower(&scope, part, &mut vars);
                self.claim_parts(&part, head, found);
            }
        }
    }
}

impl Infer<'_> {
    /// Listed claims and blocks are read only in the meaning's own scopes.
    pub(super) fn claims(
        &mut self,
        head: &Type,
        ready_only: bool,
    ) -> Vec<Type> {
        let head = super::meaning(head);
        if matches!(head, Type::Array { .. }) {
            return self.array_members(head, None, Span::default()).0;
        }
        if let Some((_, _, claims)) = self
            .state
            .claims
            .iter()
            .find(|(h, r, _)| h == head && *r == ready_only)
        {
            return claims.clone();
        }
        let Type::Named { name, arguments } = head else {
            return Vec::new();
        };
        let Some(module) = self.owner(&name.origin) else {
            return Vec::new();
        };
        let Some(scope) = self.scope(&name.origin) else {
            return Vec::new();
        };
        let mut listed = Vec::new();
        let mut shaped = false;
        for entry in self.type_entries(module, &name.name) {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Type {
                name: n,
                parameters,
                base,
                members,
                ..
            } = &item.kind
            else {
                continue;
            };
            if super::last(n) != name.name {
                continue;
            }
            shaped = members.is_some();
            let mut vars = Vars::new(SelfKind::Type(head.clone()));
            vars.names.extend(
                parameters
                    .iter()
                    .zip(arguments)
                    .map(|(p, a)| (super::last(&p.name), a.clone())),
            );
            if let Some(base) = base.filter(|_| shaped) {
                let parts = match &module.types[base.0].kind {
                    TypeKind::Both(parts) => parts.clone(),
                    _ => alloc::vec![base],
                };
                for part in parts {
                    let class = self.lower(&scope, part, &mut vars);
                    if !self
                        .member_limits(
                            &class,
                            head,
                            Span::default(),
                            &mut Vec::new(),
                        )
                        .is_empty()
                    {
                        self.claim_parts(&class, head, &mut listed);
                    }
                }
            }
        }
        let mut blocks = self.written_claims(head);
        if !shaped {
            let mut parts = Vec::new();
            self.claim_parts(head, head, &mut parts);
            for part in parts {
                let required = self.member_limits(
                    &part,
                    head,
                    Span::default(),
                    &mut Vec::new(),
                );
                if !required.is_empty() && required.iter().all(|r| matches!(r, Obligation::Member { name, ty, .. } if derived(&part) == Some("eq") || (self.claim_receiver(&part, head, name) && !super::dot::contains_type(ty, head)))) {
                    blocks.push(part);
                }
            }
            self.state
                .claims
                .push((head.clone(), ready_only, blocks.clone()));
            return blocks;
        }
        if ready_only {
            listed.retain(|c| {
                blocks.contains(c)
                    || self.derivable(c, head)
                    || self.claim_members_available(head, c)
            });
        }
        self.state
            .claims
            .push((head.clone(), ready_only, listed.clone()));
        listed
    }

    fn written_claims(&mut self, head: &Type) -> Vec<Type> {
        let Type::Named { name, arguments } = head else {
            return Vec::new();
        };
        let mut blocks = Vec::new();
        for (module, scope) in self.impl_scopes(&name.origin, &name.name) {
            for entry in self.type_entries(module, &name.name) {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Impl {
                    target,
                    claim: Some(claim),
                    ..
                } = &item.kind
                else {
                    continue;
                };
                if super::lower::named(module, *target).as_deref()
                    != Some(&name.name)
                {
                    continue;
                }
                if !self.impl_owner(&scope, *target, head) {
                    continue;
                }
                let mut vars = Vars::new(SelfKind::Type(head.clone()));
                if let Some((params, _)) = self
                    .type_item(self.owner(&name.origin).unwrap(), &name.name)
                {
                    vars.names.extend(
                        params
                            .iter()
                            .zip(arguments)
                            .map(|(p, a)| (super::last(&p.name), a.clone())),
                    );
                }
                let class = self.lower(&scope, *claim, &mut vars);
                self.claim_parts(&class, head, &mut blocks);
            }
        }
        blocks
    }

    fn derivable(&mut self, class: &Type, head: &Type) -> bool {
        let mut parts = Vec::new();
        self.claim_parts(class, head, &mut parts);
        parts.iter().all(|part| {
            if derived(part).is_some() { return true; }
            let Type::Named { name, .. } = part else { return false };
            self.owner(&name.origin).is_some_and(|module| module.entries.iter().any(|e| matches!(e, Entry::Item(crate::ast::Item { kind: ItemKind::Type { name: n, members: None, base: Some(_), limits, .. }, .. }) if super::last(n) == name.name && limits.is_empty())))
        })
    }

    pub(super) fn claim_member_count(
        &mut self,
        head: &Type,
        class: &Type,
        member: &str,
    ) -> usize {
        let Type::Named { name: owner, .. } = head else {
            return 0;
        };
        let mut parts = Vec::new();
        self.claim_parts(class, head, &mut parts);
        let mut count = 0;
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for entry in self.type_entries(module, &owner.name) {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Impl {
                    target,
                    claim: Some(id),
                    members,
                } = &item.kind
                else {
                    continue;
                };
                if super::lower::named(module, *target).as_deref()
                    != Some(&owner.name)
                {
                    continue;
                }
                if !self.impl_owner(&scope, *target, head) {
                    continue;
                }
                let mut vars = Vars::new(SelfKind::Type(head.clone()));
                let other = self.lower(&scope, *id, &mut vars);
                let other = self.claim_type(&other, head, false);
                let mut other_parts = Vec::new();
                self.claim_parts(&other, head, &mut other_parts);
                if parts.contains(&other) || other_parts.contains(class) {
                    count += members.iter().filter(|m| matches!(m, crate::ast::ImplMember::Named { name, .. } if super::last(name) == member)).count();
                }
            }
        }
        count
    }

    fn claim_members_available(&mut self, head: &Type, class: &Type) -> bool {
        let mut parts = Vec::new();
        self.claim_parts(class, head, &mut parts);
        let required =
            self.member_limits(class, head, Span::default(), &mut Vec::new());
        !required.is_empty()
            && required.iter().all(|r| match r {
                Obligation::Member { name, .. } => {
                    self.claim_member_count(head, class, name) > 0
                        || parts
                            .iter()
                            .any(|p| derived(p) == Some(name.as_str()))
                }
                Obligation::Claim { .. } => false,
            })
    }

    pub(super) fn has_claim(&mut self, head: &Type, class: &Type) -> bool {
        let class = self.claim_type(class, head, false);
        ((matches!(head, Type::Array { .. } | Type::Unit)
            || matches!(head, Type::Tuple(_))
                && !super::is_prelude(&class, "Default"))
            && derived(&class).is_some())
            || self.claims(head, true).contains(&class)
    }

    pub(super) fn require_claim(&mut self, head: Type, class: Type, dot: Span) {
        let parts = self.combination_parts(&class, &head);
        if !parts.is_empty() {
            for part in parts {
                self.require_claim(head.clone(), part, dot);
            }
            return;
        }
        self.state.obligations.push(super::Owed::new(
            Obligation::Claim { head, class, dot },
            dot,
        ));
    }

    pub(super) fn claim(&mut self, owed: &super::Owed) {
        let Obligation::Claim { head, class, dot } = &owed.obligation else {
            return;
        };
        let original = self.apply(head);
        let head = super::meaning(&original).clone();
        let class = self.claim_type(&self.apply(class), &head, false);
        let dot = if *dot == Span::default() {
            owed.span
        } else {
            *dot
        };
        let parts = self.combination_parts(&class, &head);
        if !parts.is_empty() {
            for part in parts {
                self.require_claim(head.clone(), part, dot);
            }
            return;
        }
        if derived(&class).is_some() {
            match &head {
                Type::Array { element, .. } => {
                    self.require_claim((**element).clone(), class, dot);
                    return;
                }
                Type::Tuple(parts) if !super::is_prelude(&class, "Default") => {
                    for part in parts {
                        self.require_claim(part.clone(), class.clone(), dot);
                    }
                    return;
                }
                Type::Unit => return,
                _ => {}
            }
        }
        if self.array_claim(&head, &class, dot) {
            return;
        }
        let mut candidates = self.claim_candidates(&head);
        candidates.retain(|candidate| matches!((&class,candidate), (Type::Named { name: a, .. },Type::Named { name: b, .. }) if a == b));
        if candidates.len() > 1 {
            let sources: Vec<_> = candidates
                .iter()
                .filter_map(|c| match c {
                    Type::Named { arguments, .. } => arguments.first().cloned(),
                    _ => None,
                })
                .collect();
            let source = match &class {
                Type::Named { arguments, .. } => {
                    arguments.first().cloned().unwrap_or_else(|| self.fresh())
                }
                _ => self.fresh(),
            };
            let Some(index) =
                self.choose_meaning(&source, &sources, &head, dot)
            else {
                return;
            };
            candidates = alloc::vec![candidates[index].clone()];
        }
        if let [selected] = candidates.as_slice() {
            if self.state.subst.unify(&class, selected).is_err() {
                self.error(
                    dot,
                    super::Reason::MissingClaim {
                        ty: head,
                        claim: class,
                    },
                );
                return;
            }
            if let Some(literal) = owed.literal.clone() {
                if matches!(&head, Type::Named { name, .. } if name.origin == crate::resolve::Origin::Prelude)
                {
                    self.check_literal(literal, &original, owed.span, dot);
                }
            }
            if !matches!(head, Type::Rigid(_)) {
                if let Some(member) = derived(selected) {
                    self.derive_claim(&head, selected, member, dot);
                }
            }
        } else if candidates.len() > 1 {
            self.error(
                dot,
                super::Reason::Conversion {
                    source: None,
                    target: head,
                    candidates,
                },
            );
        } else {
            self.missing_claim(owed, &original, head, class, dot);
        }
    }

    fn missing_claim(
        &mut self,
        owed: &super::Owed,
        original: &Type,
        head: Type,
        class: Type,
        dot: Span,
    ) {
        if let Some(literal) = owed
            .literal
            .clone()
            .filter(|_| owed.literal_argument && owed.span != dot)
        {
            self.check_literal(literal, original, owed.span, dot);
            if let Some(super::Error {
                reason:
                    super::Reason::Mismatch {
                        expected, actual, ..
                    },
                ..
            }) = self.state.errors.last_mut()
            {
                core::mem::swap(expected, actual);
            }
        } else {
            self.error(
                dot,
                super::Reason::MissingClaim {
                    ty: head,
                    claim: class,
                },
            );
        }
    }

    fn claim_candidates(&mut self, head: &Type) -> Vec<Type> {
        if matches!(head, Type::Rigid(_)) {
            let assumptions: Vec<_> = self
                .state
                .obligations
                .iter()
                .filter_map(|o| match &o.obligation {
                    Obligation::Claim { head: h, class, .. }
                        if o.assumed && h == head =>
                    {
                        Some(class.clone())
                    }
                    _ => None,
                })
                .collect();
            let mut candidates = Vec::new();
            for assumed in assumptions {
                self.claim_parts(&assumed, head, &mut candidates);
            }
            candidates
        } else {
            self.claims(head, true)
        }
    }

    fn derive_claim(
        &mut self,
        head: &Type,
        class: &Type,
        member: &str,
        span: Span,
    ) {
        // A written block supplies the implementation; otherwise its parts
        // carry the derivation's claim obligations.
        let Type::Named { name, arguments } = head else {
            return;
        };
        if name.origin == crate::resolve::Origin::Prelude
            && matches!(name.name.as_str(), "Vec" | "Deque")
            && member != "default"
        {
            for argument in arguments {
                let present = self.state.obligations.iter().any(|o| matches!(&o.obligation, Obligation::Claim { head: h, class: c, .. } if self.apply(h) == *argument && c == class));
                if !present {
                    self.require_claim(argument.clone(), class.clone(), span);
                }
            }
            return;
        }
        if self.claim_member_count(head, class, member) > 0 {
            return;
        }
        if member == "default" {
            if !self.default_available(head) {
                self.block_error(span, class, "has no type default");
            }
            return;
        }
        let _ = self.derived_member(head, member, span);
    }
}

fn derived(class: &Type) -> Option<&'static str> {
    let Type::Named { name, .. } = class else {
        return None;
    };
    if name.origin != crate::resolve::Origin::Prelude {
        return None;
    }
    match name.name.as_str() {
        "Eq" => Some("eq"),
        "Ord" => Some("cmp"),
        "Show" => Some("show"),
        "Default" => Some("default"),
        _ => None,
    }
}

impl Infer<'_> {
    pub(super) fn check_claim_blocks(&mut self) {
        for entry in &self.module.entries {
            let Entry::Item(item) = entry else { continue };
            self.check_alias(&item.kind);
            match &item.kind {
                ItemKind::Impl {
                    target,
                    claim: Some(id),
                    members,
                } => {
                    self.check_claim_block(*target, *id, members);
                }
                ItemKind::Impl {
                    target,
                    claim: None,
                    members,
                } => {
                    self.check_plain_block(*target, members, item.span);
                }
                ItemKind::Type {
                    name,
                    base: Some(base),
                    members: Some(_),
                    ..
                } => {
                    let head = Type::named(
                        crate::resolve::Origin::This,
                        &super::last(name),
                        Vec::new(),
                    );
                    let parts = match &self.module.types[base.0].kind {
                        TypeKind::Both(parts) => parts.clone(),
                        _ => alloc::vec![*base],
                    };
                    for part in parts {
                        let mut vars = Vars::new(SelfKind::Type(head.clone()));
                        let class = self.lower(&Scope::This, part, &mut vars);
                        let holding = matches!(&class,Type::Named { name, .. } if name.origin == crate::resolve::Origin::Prelude && matches!(name.name.as_str(),"Bits" | "Size"));
                        if !holding
                            && self
                                .member_limits(
                                    &class,
                                    &head,
                                    Span::default(),
                                    &mut Vec::new(),
                                )
                                .is_empty()
                        {
                            self.block_error(
                                self.module.types[part.0].span,
                                &class,
                                "is not a member limit or holding",
                            );
                        }
                    }
                    let listed = self.claims(&head, false);
                    self.check_listed_members(
                        &head,
                        &listed,
                        self.module.types[base.0].span,
                    );
                    let ready = self.claims(&head, true);
                    for class in listed {
                        let required = self.member_limits(
                            &class,
                            &head,
                            self.module.types[base.0].span,
                            &mut Vec::new(),
                        );
                        self.check_claim_names(
                            &head,
                            &class,
                            &required,
                            self.module.types[base.0].span,
                        );
                        if !ready.contains(&class) && self.frame.is_none() {
                            self.block_error(
                                self.module.types[base.0].span,
                                &class,
                                "has no block or derived member",
                            );
                        } else if let Some(member) =
                            derived(&class).filter(|_| self.frame.is_none())
                        {
                            self.derive_claim(
                                &head,
                                &class,
                                member,
                                self.module.types[base.0].span,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn check_plain_block(
        &mut self,
        target: crate::ast::TypeId,
        members: &[crate::ast::ImplMember],
        span: Span,
    ) {
        let head = self.lower_here(target);
        for class in self.claims(&head, true) {
            let required =
                self.member_limits(&class, &head, span, &mut Vec::new());
            for member in members {
                if let crate::ast::ImplMember::Named { name, .. } = member {
                    if required.iter().any(|r| matches!(r, Obligation::Member { name: n, .. } if *n == super::last(name))) {
                                    self.block_error(name.span, &class, "has a member also written in a plain impl");
                                }
                }
            }
        }
    }

    fn check_claim_block(
        &mut self,
        target: crate::ast::TypeId,
        id: crate::ast::TypeId,
        members: &[crate::ast::ImplMember],
    ) {
        let head = self.lower_here(target);
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        let class = self.lower(&Scope::This, id, &mut vars);
        let class = self.claim_type(&class, &head, false);
        let span = self.module.types[id.0].span;
        let required = self.member_limits(&class, &head, span, &mut Vec::new());
        if required.is_empty() {
            self.block_error(span, &class, "is not a member limit");
            return;
        }
        if !self.claims(&head, false).contains(&class) {
            self.block_error(span, &class, "is not listed on the type");
        }
        if self.claim_block_count(&head, &class, span) > 1 {
            self.block_error(span, &class, "has a second block");
            return;
        }
        for member in members {
            let crate::ast::ImplMember::Named { name, .. } = member else {
                self.block_error(
                    span,
                    &class,
                    "does not ask for a constructor",
                );
                continue;
            };
            if !required.iter().any(|r| matches!(r,Obligation::Member { name: n, .. } if *n == super::last(name))) {
                            self.block_error(name.span,&class,"does not ask for this member");
                        }
        }
        self.check_claim_names(&head, &class, &required, span);
        for requirement in required {
            let Obligation::Member { name, ty, .. } = requirement else {
                continue;
            };
            let count = self.claim_member_count(&head, &class, &name);
            if count > 1
                && self.claim_block_count(
                    &head,
                    &class,
                    Span {
                        start: usize::MAX,
                        end: usize::MAX,
                    },
                ) == 1
            {
                self.block_error(span, &class, "has a member written twice");
            }
            let Some(site) = members.iter().find_map(|m| match m {
                crate::ast::ImplMember::Named { name: n, .. }
                    if super::last(n) == name =>
                {
                    Some(n.span)
                }
                _ => None,
            }) else {
                if count == 0 {
                    let mut parts = Vec::new();
                    self.claim_parts(&class, &head, &mut parts);
                    if let Some(part) =
                        parts.iter().find(|p| derived(p) == Some(name.as_str()))
                    {
                        self.derive_claim(&head, part, &name, span);
                    } else {
                        self.block_error(
                            span,
                            &class,
                            &alloc::format!("is missing member {name}"),
                        );
                    }
                }
                continue;
            };
            if let Some(index) = self.items.iter().position(|i| i.site == site)
            {
                let callee = self.item_callee(index, site);
                let actual = if let Some(slot) = callee.self_slot {
                    let mut callee = callee;
                    callee.supplied = Some((slot, head.clone(), site));
                    self.apply_callee(callee, &[], None, site, None)
                } else {
                    callee.whole()
                };
                let ty = self.universal_member(&class, &head, &ty, &actual);
                self.check_member_type(&ty, &actual, site, span);
            }
        }
    }

    fn claim_block_count(
        &mut self,
        head: &Type,
        class: &Type,
        span: Span,
    ) -> usize {
        let Type::Named { name: owner, .. } = head else {
            return 0;
        };
        let mut count = 0;
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for other in self.type_entries(module, &owner.name) {
                let Entry::Item(other) = other else { continue };
                if core::ptr::eq(module, self.module)
                    && other.span.start > span.start
                {
                    continue;
                }
                let ItemKind::Impl {
                    target,
                    claim: Some(c),
                    ..
                } = &other.kind
                else {
                    continue;
                };
                if super::lower::named(module, *target).as_deref()
                    != Some(&owner.name)
                {
                    continue;
                }
                let mut vars = Vars::new(SelfKind::Type(head.clone()));
                let c = self.lower(&scope, *c, &mut vars);
                if self.claim_type(&c, head, false) == *class {
                    count += 1;
                }
            }
        }
        count
    }

    fn block_error(&mut self, span: Span, claim: &Type, problem: &str) {
        self.error(
            span,
            super::Reason::ClaimBlock {
                claim: claim.clone(),
                problem: alloc::string::String::from(problem),
            },
        );
    }
}

impl Infer<'_> {
    pub(super) fn default_available(&mut self, ty: &Type) -> bool {
        self.type_default(ty).is_some()
    }

    pub(super) fn type_default(
        &mut self,
        ty: &Type,
    ) -> Option<crate::limits::known::Known> {
        use crate::limits::known::{Integer, Known, Range};
        let length_fact = |length| Known {
            parts: alloc::vec![(
                "len".into(),
                Known {
                    range: Some(Range::exact(Integer::new(false, length))),
                    ..Known::default()
                }
            )],
            ..Known::default()
        };
        if let Type::Limited { meaning, .. } = ty {
            let known = self.type_default(meaning)?;
            return crate::limits::default_proved(
                &self.limit_inputs(),
                ty,
                &known,
            )
            .then_some(known);
        }
        if !matches!(ty, Type::Named { name, .. } if name.origin == crate::resolve::Origin::Prelude)
            && self.claim_member_count(
                ty,
                &super::prelude("Default"),
                "default",
            ) > 0
        {
            return Some(Known::default());
        }
        match ty {
            Type::Unit => Some(Known::default()),
            Type::Named { name, .. }
                if name.origin == crate::resolve::Origin::Prelude
                    && matches!(
                        name.name.as_str(),
                        "Bool" | "Vec" | "Deque" | "Maybe"
                    ) =>
            {
                if matches!(name.name.as_str(), "Vec" | "Deque") {
                    return Some(Known {
                        meaning: Some(name.clone()),
                        ..length_fact(0.into())
                    });
                }
                Some(Known {
                    boolean: (name.name == "Bool").then_some(false),
                    constructor: (name.name == "Maybe")
                        .then(|| "Nothing".into()),
                    ..Known::default()
                })
            }
            Type::Named { name, .. } => {
                if (name.origin == crate::resolve::Origin::Prelude
                    && name.name == "String")
                    || self.state.lowering.contains(name)
                {
                    return None;
                }
                self.state.lowering.push(name.clone());
                let fields = self.fields(ty);
                let result = fields.and_then(|fields| {
                    let module = self.owner(&name.origin)?;
                    let (_, members) = self.type_item(module, &name.name)?;
                    let mut parts = Vec::new();
                    for field in fields {
                        let value = members?.iter().find_map(|m| match m {
                            crate::ast::Member::Field(f)
                                if super::last(&f.name) == field.name =>
                            {
                                f.default
                            }
                            _ => None,
                        });
                        let known = match value {
                            Some(id) => crate::limits::default_fact(module, id),
                            None => self.type_default(&field.ty)?,
                        };
                        parts.push((field.name, known));
                    }
                    Some(Known {
                        parts,
                        constructor: Some(name.name.clone()),
                        ..Known::default()
                    })
                });
                self.state.lowering.pop();
                result
            }
            Type::Array { element, length } => {
                self.type_default(element)?;
                Some(match length.as_ref() {
                    Type::Number(length) => length_fact(length.clone()),
                    _ => Known::default(),
                })
            }
            _ => None,
        }
    }
}

impl Infer<'_> {
    fn check_claim_names(
        &mut self,
        head: &Type,
        class: &Type,
        required: &[Obligation],
        span: Span,
    ) {
        let Type::Named { name: owner, .. } = head else {
            return;
        };
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for entry in self.type_entries(module, &owner.name) {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Impl {
                    target,
                    claim,
                    members,
                } = &item.kind
                else {
                    continue;
                };
                if super::lower::named(module, *target).as_deref()
                    != Some(&owner.name)
                {
                    continue;
                }
                let Some(claim) = claim else {
                    for member in members {
                        if let crate::ast::ImplMember::Named { name, .. } =
                            member
                        {
                            if required.iter().any(|r| matches!(r,Obligation::Member { name: n, .. } if *n == super::last(name))) {
                                self.block_error(span,class,"has a member also written in a plain impl");
                            }
                        }
                    }
                    continue;
                };
                let mut vars = Vars::new(SelfKind::Type(head.clone()));
                let other = self.lower(&scope, *claim, &mut vars);
                if matches!((class,&other),(Type::Named { name: a, .. },Type::Named { name: b, .. }) if a == b)
                {
                    continue;
                }
                let others =
                    self.member_limits(&other, head, span, &mut Vec::new());
                for a in required {
                    let Obligation::Member { name, ty, .. } = a else {
                        continue;
                    };
                    for b in &others {
                        if let Obligation::Member { name: n, ty: t, .. } = b {
                            if n == name
                                && self
                                    .state
                                    .subst
                                    .clone()
                                    .unify(ty, t)
                                    .is_err()
                            {
                                self.block_error(span,class,"conflicts with another claim's member type");
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Infer<'_> {
    pub(super) fn parameter_claims(
        &mut self,
        ty: &Type,
        vars: &mut Vars,
        span: Span,
    ) {
        let Type::Named { name, arguments } = ty else {
            return;
        };
        let Some(module) = self.owner(&name.origin) else {
            return;
        };
        let Some(scope) = self.scope(&name.origin) else {
            return;
        };
        let Some((parameters, _)) = self.type_item(module, &name.name) else {
            return;
        };
        let mut bound_vars = Vars::new(SelfKind::Type(ty.clone()));
        bound_vars.names.extend(
            parameters
                .iter()
                .zip(arguments)
                .map(|(p, a)| (super::last(&p.name), a.clone())),
        );
        for (parameter, head) in parameters.iter().zip(arguments) {
            let Some(bound) = parameter.bound else {
                continue;
            };
            let parts = match &module.types[bound.0].kind {
                TypeKind::Both(parts) => parts.clone(),
                _ => alloc::vec![bound],
            };
            for part in parts {
                let class = self.lower(&scope, part, &mut bound_vars);
                vars.obligations.push(Obligation::Claim {
                    head: head.clone(),
                    class,
                    dot: span,
                });
            }
        }
    }
}

impl Infer<'_> {
    pub(super) fn check_supplied_claim(
        &mut self,
        head: &Type,
        class: &Type,
        span: Span,
    ) {
        let Type::Named { name: owner, .. } = head else {
            return;
        };
        if !matches!(
            owner.origin,
            crate::resolve::Origin::Prelude
                | crate::resolve::Origin::Commands
                | crate::resolve::Origin::Host(_)
        ) {
            return;
        }
        let required = self.member_limits(class, head, span, &mut Vec::new());
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for entry in self.type_entries(module, &owner.name) {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Impl {
                    target,
                    claim: Some(id),
                    members,
                } = &item.kind
                else {
                    continue;
                };
                if super::lower::named(module, *target).as_deref()
                    != Some(&owner.name)
                {
                    continue;
                }
                let mut vars = Vars::new(SelfKind::Type(head.clone()));
                let claim = self.lower(&scope, *id, &mut vars);
                let mut parts = Vec::new();
                self.claim_parts(&claim, head, &mut parts);
                let mut required_parts = Vec::new();
                self.claim_parts(class, head, &mut required_parts);
                let claim = self.claim_type(&claim, head, false);
                if !parts.contains(class) && !required_parts.contains(&claim) {
                    continue;
                }
                for member in members {
                    let crate::ast::ImplMember::Named {
                        name,
                        annotation: Some(ty),
                        value: None,
                        ..
                    } = member
                    else {
                        continue;
                    };
                    let text = super::last(name);
                    let Some(expected) =
                        required.iter().find_map(|r| match r {
                            Obligation::Member { name, ty, .. }
                                if *name == text =>
                            {
                                Some(ty)
                            }
                            _ => None,
                        })
                    else {
                        continue;
                    };
                    let receiver = self.claim_receiver(class, head, &text);
                    let actual = self.supplied_claim_type(
                        module, &scope, head, *ty, name.span, receiver,
                    );
                    let expected =
                        self.universal_member(class, head, expected, &actual);
                    self.check_member_type(&expected, &actual, span, span);
                }
            }
        }
    }
}

impl<'a> Infer<'a> {
    fn supplied_claim_type(
        &mut self,
        module: &crate::ast::Module,
        scope: &Scope<'a>,
        head: &Type,
        ty: crate::ast::TypeId,
        span: Span,
        receiver: bool,
    ) -> Type {
        let Type::Named {
            name: owner,
            arguments,
        } = head
        else {
            return self.fresh();
        };
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        if let Some((parameters, _)) = self.type_item(module, &owner.name) {
            vars.names.extend(
                parameters
                    .iter()
                    .zip(arguments)
                    .map(|(p, a)| (super::last(&p.name), a.clone())),
            );
        }
        let (slots, result, self_slot) = self.signature(scope, ty, &mut vars);
        let self_slot = receiver
            .then(|| self_slot.or_else(|| slots.len().checked_sub(1)))
            .flatten();
        if receiver && self_slot.is_none() {
            self.error(span, super::Reason::NotAFunction);
        }
        let mut parameters: Vec<_> = slots.into_iter().map(|s| s.ty).collect();
        if let Some(slot) = self_slot {
            let receiver = parameters.remove(slot);
            self.unify(head, &receiver, span, span);
        }
        parameters
            .into_iter()
            .rev()
            .fold(result, |r, p| Type::function(p, r))
    }
}

impl Infer<'_> {
    /// Member-local variables are promises, not choices for the impl.
    fn universal_member(
        &self,
        class: &Type,
        head: &Type,
        expected: &Type,
        actual: &Type,
    ) -> Type {
        let mut fixed = Vec::new();
        head.variables(&mut fixed);
        class.variables(&mut fixed);
        let mut variables = Vec::new();
        expected.variables(&mut variables);
        let mut replacements: Vec<_> = variables
            .into_iter()
            .filter(|v| !fixed.contains(v))
            .map(|v| (v, Type::Rigid(alloc::format!("member{}", v.0))))
            .collect();
        // Sequence also admits the head's own number parameter.
        if super::is_prelude(class, "Sequence") {
            if let (
                Type::Function { parameter, .. },
                Type::Function {
                    parameter: source,
                    result,
                },
            ) = (expected, actual)
            {
                let mut trial = self.state.subst.clone();
                if trial.unify(result, head).is_ok() {
                    if let (
                        Type::Array { length, .. },
                        Type::Array {
                            length: supplied, ..
                        },
                        Type::Named { name, arguments },
                    ) = (&**parameter, trial.apply(source), head)
                    {
                        let own = self
                            .owner(&name.origin)
                            .and_then(|m| self.type_item(m, &name.name))
                            .is_some_and(|(ps, _)| {
                                ps.iter().zip(arguments).any(|(p, a)| {
                                    !p.name.is_uppercase() && *a == *supplied
                                })
                            });
                        if own {
                            if let Type::Var(var) = &**length {
                                replacements.retain(|(v, _)| v != var);
                                replacements.push((*var, *supplied));
                            }
                        }
                    }
                }
            }
        }
        crate::types::replace(expected, &replacements)
    }

    pub(super) fn check_member_type(
        &mut self,
        expected: &Type,
        actual: &Type,
        span: Span,
        other: Span,
    ) {
        let expected = self.apply(expected);
        let actual = self.apply(actual);
        let errors = self.state.errors.len();
        self.unify(&expected, &actual, span, other);
        if self.state.errors.len() == errors && !limits_fit(&actual, &expected)
        {
            let known = crate::limits::known::Known {
                narrowing: true,
                ..Default::default()
            };
            self.state.checks.push(crate::typed::Check {
                value: None,
                path: Vec::new(),
                target: Some(expected.clone()),
                place: span,
                place_kind: crate::typed::Place::Result,
                limit: other,
                phase: crate::typed::Phase::Eval,
                reason: crate::typed::CheckReason::Narrowing,
                frequency: crate::typed::Frequency::PerCall,
                place_origin: crate::resolve::Origin::This,
                limit_origin: crate::resolve::Origin::This,
                name: alloc::format!("{expected}"),
                test_count: 1,
                known,
                gate: false,
                when_integer: false,
                owner: None,
            });
        }
    }
}

impl<'a> Infer<'a> {
    fn impl_owner(
        &mut self,
        scope: &Scope<'a>,
        target: crate::ast::TypeId,
        head: &Type,
    ) -> bool {
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        let target = self.lower(scope, target, &mut vars);
        matches!((super::meaning(&target),super::meaning(head)),(Type::Named { name: a, .. },Type::Named { name: b, .. }) if a == b)
    }
}

impl Infer<'_> {
    pub(super) fn claim_receiver(
        &mut self,
        class: &Type,
        head: &Type,
        member: &str,
    ) -> bool {
        let mut parts = Vec::new();
        self.claim_parts(class, head, &mut parts);
        parts.iter().any(|part| {
            let Type::Named { name, .. } = part else { return false };
            self.owner(&name.origin).is_some_and(|module| module.entries.iter().any(|entry| {
                matches!(entry, Entry::Item(item) if matches!(&item.kind,
                    ItemKind::Type { name: n, limits, .. } if super::last(n) == name.name && limits.iter().any(|limit| matches!(limit,
                        crate::ast::Limit::Member { name, .. } if super::last(name) == member && name.segments.first().is_some_and(|n| n == "self")))))
            }))
        })
    }
}

impl Infer<'_> {
    fn combination_parts(&mut self, class: &Type, head: &Type) -> Vec<Type> {
        let Type::Named { name, arguments } = class else {
            return Vec::new();
        };
        let (Some(module), Some(scope)) =
            (self.owner(&name.origin), self.scope(&name.origin))
        else {
            return Vec::new();
        };
        let Some((parameters, base)) =
            module.entries.iter().find_map(|e| match e {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        name: n,
                        parameters,
                        base: Some(base),
                        members: None,
                        limits,
                    } if super::last(n) == name.name && limits.is_empty() => {
                        Some((parameters, base))
                    }
                    _ => None,
                },
                Entry::Line(_) => None,
            })
        else {
            return Vec::new();
        };
        let TypeKind::Both(parts) = &module.types[base.0].kind else {
            return Vec::new();
        };
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        vars.names.extend(
            parameters
                .iter()
                .zip(arguments)
                .map(|(p, a)| (super::last(&p.name), a.clone())),
        );
        parts
            .iter()
            .map(|p| self.lower(&scope, *p, &mut vars))
            .collect()
    }
}

impl Infer<'_> {
    pub(super) fn constrain_claim_member(
        &mut self,
        class: &Type,
        head: &Type,
        required: &[Obligation],
        item: &super::Item,
        other: Span,
    ) {
        let super::Key::Member(_, name) = &item.key else {
            return;
        };
        let Some(expected) = required.iter().find_map(|r| match r {
            Obligation::Member { name: n, ty, .. } if n == name => Some(ty),
            _ => None,
        }) else {
            return;
        };
        let slot = item.heads.as_ref().and_then(|heads| {
            heads.iter().position(|h| h.name.as_deref() == Some("self"))
        });
        if self.claim_receiver(class, head, name) != slot.is_some() {
            self.block_error(item.site, class, "has the wrong receiver");
            return;
        }
        let expected = slot.map_or_else(
            || expected.clone(),
            |slot| {
                let (mut parameters, result) = self
                    .peel(expected, slot)
                    .unwrap_or_else(|| (Vec::new(), expected.clone()));
                parameters.push(head.clone());
                parameters
                    .into_iter()
                    .rev()
                    .fold(result, |r, p| Type::function(p, r))
            },
        );
        self.check_member_type(&expected, &item.ty, item.site, other);
    }
}

impl Infer<'_> {
    fn check_alias(&mut self, kind: &ItemKind) {
        if let ItemKind::Type { name, .. } = kind {
            if self.base_cycle(&super::last(name)) {
                let class = Type::named(
                    crate::resolve::Origin::This,
                    &super::last(name),
                    Vec::new(),
                );
                self.block_error(name.span, &class, "has a cyclic base");
                return;
            }
        }
        if let ItemKind::Type {
            base: Some(base),
            members: None,
            limits,
            ..
        } = kind
        {
            if limits.is_empty()
                && !matches!(
                    self.module.types[base.0].kind,
                    TypeKind::Both(_) | TypeKind::Either(_) | TypeKind::Not(_)
                )
            {
                let class = self.lower_here(*base);
                self.block_error(
                    self.module.types[base.0].span,
                    &class,
                    "a single base with no limit is not a type",
                );
            }
        }
    }
}

// A supplied value may promise stronger results, but must accept every
// parameter admitted by the required member, including nested places.
fn limits_fit(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Var(_), _) | (_, Type::Var(_)) => true,
        (Type::Limited { name: a, .. }, Type::Limited { name: e, .. }) => {
            a == e
        }
        (_, Type::Limited { .. }) => false,
        (Type::Limited { meaning, .. }, _) => limits_fit(meaning, expected),
        (
            Type::Function {
                parameter: ap,
                result: ar,
            },
            Type::Function {
                parameter: ep,
                result: er,
            },
        ) => limits_fit(ep, ap) && limits_fit(ar, er),
        (
            Type::Named { arguments: a, .. },
            Type::Named { arguments: e, .. },
        )
        | (Type::Tuple(a), Type::Tuple(e))
        | (Type::Either(a), Type::Either(e)) => {
            a.iter().zip(e).all(|(a, e)| limits_fit(a, e))
        }
        (Type::Array { element: a, .. }, Type::Array { element: e, .. }) => {
            limits_fit(a, e)
        }
        _ => true,
    }
}

impl Infer<'_> {
    fn check_listed_members(
        &mut self,
        head: &Type,
        listed: &[Type],
        span: Span,
    ) {
        let mut seen: Vec<(&Type, alloc::string::String, Type)> = Vec::new();
        for class in listed {
            for requirement in
                self.member_limits(class, head, span, &mut Vec::new())
            {
                let Obligation::Member { name, ty, .. } = requirement else {
                    continue;
                };
                for (prior, member, other) in &seen {
                    let same_class = matches!((class,*prior), (Type::Named { name: a, .. }, Type::Named { name: b, .. }) if a == b);
                    if member == &name
                        && !same_class
                        && (!limits_fit(&ty, other)
                            || !limits_fit(other, &ty)
                            || self
                                .state
                                .subst
                                .clone()
                                .unify(&ty, other)
                                .is_err())
                    {
                        self.block_error(
                            span,
                            class,
                            "conflicts with another listed claim's member type",
                        );
                    }
                }
                seen.push((class, name, ty));
            }
        }
    }
}

impl Infer<'_> {
    fn base_cycle(&mut self, root: &str) -> bool {
        let mut pending =
            alloc::vec![(alloc::string::String::from(root), false)];
        let mut visiting = alloc::collections::BTreeSet::new();
        while let Some((name, complete)) = pending.pop() {
            if complete {
                visiting.remove(&name);
                self.state.acyclic_types.insert(name);
                continue;
            }
            if self.state.acyclic_types.contains(&name) {
                continue;
            }
            if !visiting.insert(name.clone()) {
                return true;
            }
            pending.push((name.clone(), true));
            for entry in self.type_entries(self.module, &name) {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Type {
                    base: Some(base), ..
                } = item.kind
                else {
                    continue;
                };
                let mut parts = alloc::vec![base];
                while let Some(part) = parts.pop() {
                    match &self.module.types[part.0].kind {
                        TypeKind::Both(types) | TypeKind::Either(types) => {
                            parts.extend(types);
                        }
                        TypeKind::Not(inner) => parts.push(*inner),
                        TypeKind::Named { name, .. }
                            if name.segments.len() == 1 =>
                        {
                            pending.push((super::last(name), false));
                        }
                        _ => (),
                    }
                }
            }
        }
        false
    }
}
