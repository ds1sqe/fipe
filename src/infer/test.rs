//! Retain supplied limit bodies as checked functions of self.
use super::{
    lower::{SelfKind, Vars},
    Infer, Slot,
};
use crate::{
    ast::{Entry, ItemKind, Limit},
    resolve::Origin,
    typed,
    types::Type,
};
use alloc::{collections::BTreeSet, string::String, vec::Vec};

impl Infer<'_> {
    pub(super) fn assume_test_owner(
        &mut self,
        owner: &Type,
        site: crate::token::Span,
    ) {
        let limited = self.limited(owner.clone());
        self.self_type = Some(super::meaning(&limited).clone());
        let mut vars = Vars::new(SelfKind::Type(owner.clone()));
        self.parameter_claims(owner, &mut vars, site);
        for obligation in vars.obligations {
            let mut owed = super::Owed::new(obligation, site);
            owed.solved = true;
            owed.assumed = true;
            self.state.obligations.push(owed);
        }
    }

    pub(super) fn record_local_test(
        &mut self,
        owner: &Type,
        self_site: crate::token::Span,
        value: crate::ast::ExprId,
    ) {
        self.state.tests.push(typed::Test {
            origin: Origin::This,
            owner: owner.clone(),
            self_site,
            value,
            expression: None,
        });
    }

    pub(super) fn record_supplied_tests(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let mut pending: Vec<_> = self
            .state
            .types
            .iter()
            .flatten()
            .map(|ty| self.apply(ty))
            .collect();
        pending.extend(
            self.state
                .entries
                .iter()
                .map(|(_, ty, _, _)| self.apply(ty)),
        );
        for index in 0..self.module.patterns.len() {
            if let Some(crate::resolve::Target::Type { origin, name }) =
                self.resolved.pattern(crate::ast::PatternId(index))
            {
                pending.push(Type::named(origin.clone(), name, Vec::new()));
            }
        }
        let mut seen = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            match ty {
                Type::Named { name, arguments } => {
                    pending.extend(arguments);
                    if seen.insert(name.clone())
                        && matches!(
                            name.origin,
                            Origin::Prelude
                                | Origin::Commands
                                | Origin::Host(_)
                        )
                    {
                        self.record_type_tests(&name, &mut pending);
                    }
                }
                Type::Limited { name, meaning } => {
                    pending.push(*name);
                    pending.push(*meaning);
                }
                Type::Array { element, length }
                | Type::Function {
                    parameter: element,
                    result: length,
                } => {
                    pending.push(*element);
                    pending.push(*length);
                }
                Type::Tuple(parts)
                | Type::Either(parts)
                | Type::Both(parts)
                | Type::Sum(parts) => pending.extend(parts),
                Type::Not(inner) => pending.push(*inner),
                _ => (),
            }
        }
    }

    fn record_type_tests(
        &mut self,
        owner: &crate::types::TypeName,
        pending: &mut Vec<Type>,
    ) {
        let Some(module) = self.owner(&owner.origin) else {
            return;
        };
        let Some((name, parameters, base, limits)) =
            module.entries.iter().find_map(|entry| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let ItemKind::Type {
                    name,
                    parameters,
                    base,
                    limits,
                    ..
                } = &item.kind
                else {
                    return None;
                };
                (super::last(name) == owner.name)
                    .then_some((name, parameters, base, limits))
            })
        else {
            return;
        };
        let owner_type = Type::Named {
            name: owner.clone(),
            arguments: parameters
                .iter()
                .map(|parameter| Type::Rigid(super::last(&parameter.name)))
                .collect(),
        };
        let mut vars = Vars::new(SelfKind::Type(owner_type.clone()));
        vars.names.extend(parameters.iter().map(|parameter| {
            let name = super::last(&parameter.name);
            (name.clone(), Type::Rigid(name))
        }));
        if let (Some(base), Some(scope)) = (base, self.scope(&owner.origin)) {
            let parts = match &module.types[base.0].kind {
                crate::ast::TypeKind::Both(parts) => parts.clone(),
                _ => alloc::vec![*base],
            };
            for part in parts {
                pending.push(self.lower(&scope, part, &mut vars));
            }
        }
        let limited = self.limited(owner_type.clone());
        let earlier = Slot {
            place_kind: typed::Place::Parameter(Some(String::from("self"))),
            name: Some(String::from("self")),
            ty: super::meaning(&limited).clone(),
            default: false,
            value: None,
            site: name.span,
        };
        for limit in limits {
            let Limit::Test(value) = limit else { continue };
            let slot = Slot {
                place_kind: typed::Place::Result,
                name: None,
                ty: super::prelude("Bool"),
                default: false,
                value: Some(*value),
                site: module.expressions[value.0].span,
            };
            let (_, valid) = self.declared_context(
                &owner.origin,
                &slot,
                core::slice::from_ref(&earlier),
                slot.site,
                true,
                Some(&owner_type),
            );
            if valid.is_ok() {
                if let Some(expression) =
                    self.file_cache.expression(&(owner.origin.clone(), value.0))
                {
                    self.state.tests.push(typed::Test {
                        origin: owner.origin.clone(),
                        owner: owner_type.clone(),
                        self_site: name.span,
                        value: *value,
                        expression: Some(expression),
                    });
                }
            } else {
                self.error(
                    self.module.span,
                    super::Reason::DeclarationValue {
                        name: owner.name.clone(),
                        origin: owner.origin.clone(),
                        declaration: slot.site,
                    },
                );
            }
        }
    }
}
