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
        let Some((parameters, _)) = super::lower::type_item(module, &name.name)
        else {
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
        for entry in &module.entries {
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
        for entry in &module.entries {
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
        let blocks = self.written_claims(head);
        if !shaped {
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
            for entry in &module.entries {
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
                if let Some((params, _)) = super::lower::type_item(
                    self.owner(&name.origin).unwrap(),
                    &name.name,
                ) {
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

    fn claim_member_count(
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
            for entry in &module.entries {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Impl {
                    target,
                    claim: Some(id),
                    members,
                } = &item.kind
                else {
                    continue;
                };
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
        (matches!(head, Type::Array { .. } | Type::Unit)
            && derived(&class).is_some())
            || self.claims(head, true).contains(&class)
    }

    pub(super) fn require_claim(&mut self, head: Type, class: Type, dot: Span) {
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
        if derived(&class).is_some() {
            match &head {
                Type::Array { element, .. } => {
                    self.require_claim((**element).clone(), class, dot);
                    return;
                }
                Type::Unit => return,
                _ => {}
            }
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
        let mut matches = Vec::new();
        for candidate in &candidates {
            let mut subst = self.state.subst.clone();
            if subst.unify(&class, candidate).is_ok() {
                matches.push((candidate.clone(), subst));
            }
        }
        if let [(selected, subst)] = matches.as_slice() {
            self.state.subst = subst.clone();
            if let Some(literal) = owed.literal.clone() {
                if matches!(&head, Type::Named { name, .. } if name.origin == crate::resolve::Origin::Prelude)
                {
                    self.check_literal(literal, &original, dot);
                }
            }
            if !matches!(head, Type::Rigid(_)) {
                self.check_supplied_claim(&head, selected, dot);
                if let Some(member) = derived(selected) {
                    self.derive_claim(&head, selected, member, dot);
                }
            }
        } else if matches.len() > 1 {
            self.error(
                dot,
                super::Reason::Conversion {
                    target: head,
                    candidates,
                },
            );
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
                        } else if let Some(member) = derived(&class) {
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
        if self.claim_block_count(&head, &class) > 1 {
            self.block_error(span, &class, "has a second block");
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
            if count > 1 {
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
                self.check_member_type(&ty, &actual, site, span);
            }
        }
    }

    fn claim_block_count(&mut self, head: &Type, class: &Type) -> usize {
        let Type::Named { name: owner, .. } = head else {
            return 0;
        };
        let mut count = 0;
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for other in &module.entries {
                let Entry::Item(other) = other else { continue };
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
        match super::meaning(ty) {
            Type::Unit => true,
            Type::Named { name, .. }
                if name.origin == crate::resolve::Origin::Prelude
                    && matches!(
                        name.name.as_str(),
                        "Bool" | "Vec" | "Deque" | "Maybe"
                    ) =>
            {
                true
            }
            Type::Named { name, .. } => {
                if self.state.lowering.contains(name) {
                    return false;
                }
                self.state.lowering.push(name.clone());
                let fields = self.fields(ty);
                let available = fields.is_some_and(|fields| {
                    fields
                        .iter()
                        .all(|f| f.default || self.default_available(&f.ty))
                });
                self.state.lowering.pop();
                available
            }
            Type::Array { element, .. } => self.default_available(element),
            _ => false,
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
            for entry in &module.entries {
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
        let Some((parameters, _)) = super::lower::type_item(module, &name.name)
        else {
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
    fn check_supplied_claim(&mut self, head: &Type, class: &Type, span: Span) {
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
        let mut vars = Vec::new();
        head.variables(&mut vars);
        class.variables(&mut vars);
        if vars.is_empty() {
            let pair = (head.clone(), class.clone());
            if self.state.checked_claims.contains(&pair) {
                return;
            }
            self.state.checked_claims.push(pair);
        }
        let required = self.member_limits(class, head, span, &mut Vec::new());
        for (module, scope) in self.impl_scopes(&owner.origin, &owner.name) {
            for entry in &module.entries {
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
                    let actual = self.supplied_claim_type(
                        module, &scope, head, *ty, span, &text,
                    );
                    self.check_member_type(expected, &actual, span, span);
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
        text: &str,
    ) -> Type {
        let Type::Named {
            name: owner,
            arguments,
        } = head
        else {
            return self.fresh();
        };
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        if let Some((parameters, _)) =
            super::lower::type_item(module, &owner.name)
        {
            vars.names.extend(
                parameters
                    .iter()
                    .zip(arguments)
                    .map(|(p, a)| (super::last(&p.name), a.clone())),
            );
        }
        let (slots, result, self_slot) = self.signature(scope, ty, &mut vars);
        let self_slot = self_slot.or_else(|| {
            (!matches!(
                text,
                "literal" | "read" | "zero" | "default" | "from" | "try_from"
            ))
            .then(|| slots.len().checked_sub(1))
            .flatten()
        });
        let mut callee =
            self.finish_callee(slots, result, self_slot, vars, span);
        if let Some(slot) = self_slot {
            callee.supplied = Some((slot, head.clone(), span));
            self.apply_callee(callee, &[], None, span, None)
        } else {
            callee.whole()
        }
    }
}

impl Infer<'_> {
    fn check_member_type(
        &mut self,
        expected: &Type,
        actual: &Type,
        span: Span,
        other: Span,
    ) {
        let expected = self.apply(expected);
        let actual = self.apply(actual);
        let mut e = &expected;
        let mut a = &actual;
        while let (
            Type::Function {
                parameter: ep,
                result: er,
            },
            Type::Function {
                parameter: ap,
                result: ar,
            },
        ) = (e, a)
        {
            if matches!(**ap, Type::Limited { .. }) && ap != ep {
                self.error(
                    span,
                    super::Reason::Mismatch {
                        expected,
                        actual,
                        other,
                    },
                );
                return;
            }
            e = er;
            a = ar;
        }
        self.unify(&expected, &actual, span, other);
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
