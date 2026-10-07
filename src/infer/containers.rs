//! Structural containers read their supplied members from the prelude.
use super::{
    lower::{Scope, SelfKind, Vars},
    Callee, Infer,
};
use crate::{
    ast::{Entry, ImplMember, ItemKind, TypeKind},
    resolve::Origin,
    token::Span,
    types::Type,
};
use alloc::vec::Vec;

impl Infer<'_> {
    pub(super) fn array_claim(
        &mut self,
        head: &Type,
        class: &Type,
        dot: Span,
    ) -> bool {
        for candidate in self.array_members(head, None, dot).0 {
            if matches!((&candidate, class), (Type::Named { name: a, .. }, Type::Named { name: b, .. }) if a == b)
            {
                self.unify(&candidate, class, dot, dot);
                return true;
            }
        }
        false
    }

    pub(super) fn array_members(
        &mut self,
        head: &Type,
        member: Option<&str>,
        span: Span,
    ) -> (Vec<Type>, Option<Callee>) {
        let Type::Array { element, length } = head else {
            return (Vec::new(), None);
        };
        let module = &self.declarations.prelude.module;
        let scope = Scope::Declared {
            module,
            origin: Origin::Prelude,
        };
        let mut claims = Vec::new();
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
            let TypeKind::Array {
                element: e,
                length: n,
            } = &module.types[target.0].kind
            else {
                continue;
            };
            let mut vars = Vars::new(SelfKind::Type(head.clone()));
            for (id, ty) in [(e, element), (n, length)] {
                if let TypeKind::Named { name, .. } = &module.types[id.0].kind {
                    vars.names.push((super::last(name), (**ty).clone()));
                }
            }
            if let Some(claim) = claim {
                claims.push(self.lower(&scope, *claim, &mut vars));
            }
            for entry in members {
                if let ImplMember::Named {
                    name,
                    annotation: Some(ty),
                    ..
                } = entry
                {
                    if member == Some(super::last(name).as_str()) {
                        let (slots, result, receiver) =
                            self.signature(&scope, *ty, &mut vars);
                        return (
                            claims,
                            Some(self.finish_callee(
                                slots, result, receiver, vars, span,
                            )),
                        );
                    }
                }
            }
        }
        (claims, None)
    }
}

impl Infer<'_> {
    pub(super) fn sequence_source(
        &mut self,
        head: &Type,
        span: Span,
    ) -> Option<Type> {
        let Type::Named { name, .. } = head else {
            return None;
        };
        let key = super::Key::Member(
            name.name.clone(),
            alloc::string::String::from("sequence"),
        );
        let callee = if matches!(name.origin, Origin::This | Origin::Line(_)) {
            self.item_index(&key).map(|i| self.item_callee(i, span))
        } else {
            None
        }
        .or_else(|| {
            self.conversion_members(
                &name.origin,
                &name.name,
                head,
                "sequence",
                span,
            )
            .into_iter()
            .next()
        })?;
        let Type::Function { parameter, result } = callee.whole() else {
            return None;
        };
        self.unify(head, &result, span, span);
        Some(self.apply(&parameter))
    }
}
