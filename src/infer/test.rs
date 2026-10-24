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
            self.push_owed(owed);
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
            self.state.entries.iter().map(|(_, ty, ..)| self.apply(ty)),
        );
        for index in 0..self.module.patterns.len() {
            if let Some(crate::resolve::Target::Type { origin, name }) =
                self.resolved.pattern(crate::ast::PatternId(index))
            {
                pending.push(Type::named(origin.clone(), name, Vec::new()));
            }
        }
        self.record_supplied_type_tests(pending);
    }

    pub(super) fn record_dispatch_input_tests(
        &mut self,
        dispatch: &[typed::Dispatch],
    ) {
        if !self.state.errors.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        for row in dispatch.iter().flat_map(|dispatch| &dispatch.rows) {
            let typed::Selected::Declared { origin, site } = &row.selected
            else {
                continue;
            };
            let Some(module) = self.owner(origin) else {
                continue;
            };
            let Some(annotation) = module.entries.iter().find_map(|entry| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let ItemKind::Impl { members, .. } = &item.kind else {
                    return None;
                };
                members.iter().find_map(|member| match member {
                    crate::ast::ImplMember::Named {
                        name,
                        annotation,
                        value: None,
                        ..
                    } if name.span == *site => *annotation,
                    _ => None,
                })
            }) else {
                continue;
            };
            let mut annotation = annotation;
            if let crate::ast::TypeKind::Qualified { ty, .. } =
                &module.types[annotation.0].kind
            {
                annotation = *ty;
            }
            let mut ty = &row.ty;
            while let crate::ast::TypeKind::Function { result: next, .. } =
                &module.types[annotation.0].kind
            {
                let Type::Function { parameter, result } = ty else {
                    break;
                };
                pending.push(parameter.as_ref().clone());
                ty = result;
                annotation = *next;
            }
        }
        // Type tests are checked in their closed, rigid owner context. They
        // contribute no caller obligations or executable caller checks.
        self.record_supplied_type_tests(pending);
    }

    fn record_supplied_type_tests(&mut self, mut pending: Vec<Type>) {
        let mut seen = BTreeSet::new();
        while let Some(ty) = pending.pop() {
            match ty {
                Type::Named { name, arguments } => {
                    pending.extend(arguments);
                    if seen.insert(name.clone()) {
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
        // Local, loaded-file and accepted-line bodies already have their
        // checked owner. Discover their bases without checking them again.
        if matches!(
            owner.origin,
            Origin::This | Origin::File(_) | Origin::Line(_)
        ) {
            return;
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
            self.record_declared_type_test(
                owner,
                &owner_type,
                &earlier,
                &slot,
                *value,
            );
        }
    }

    fn record_declared_type_test(
        &mut self,
        owner: &crate::types::TypeName,
        owner_type: &Type,
        earlier: &Slot,
        slot: &Slot,
        value: crate::ast::ExprId,
    ) {
        if self.state.tests.iter().any(|test| {
            test.origin == owner.origin
                && test.owner == *owner_type
                && test.value == value
        }) {
            return;
        }
        let (_, valid) = self.declared_context(
            &owner.origin,
            slot,
            core::slice::from_ref(earlier),
            slot.site,
            true,
            Some(owner_type),
        );
        if valid.is_ok() {
            if let Some(expression) =
                self.file_cache.expression(&(owner.origin.clone(), value.0))
            {
                self.state.tests.push(typed::Test {
                    origin: owner.origin.clone(),
                    owner: owner_type.clone(),
                    self_site: earlier.site,
                    value,
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

#[cfg(test)]
mod declared_input_discovery_tests {
    extern crate std;
    use super::*;

    const HEADER: &str = "pub type Tagged<T>: Integer where self < 10\npub type M<T> { pub token: T }\nimpl M { op: Tagged<T> -> (self: Self) -> Integer }\n";
    const MAIN: &str =
        "use methods\nget = m => m.op\nf = get (methods.M 7)\nf 3\n";

    #[test]
    fn closed_row_test_discovery_preserves_existing_caller_obligations() {
        let mut declarations = crate::decl::Declarations::new();
        declarations.module("methods", HEADER).unwrap();
        let module = crate::parse::parse(crate::lex::lex(MAIN)).unwrap();
        let resolved =
            crate::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed = crate::infer::infer(&module, &resolved, &declarations, &[])
            .unwrap();
        let caller =
            crate::parse::parse(crate::lex::lex("plus = x y => x + y"))
                .unwrap();
        let caller_resolved =
            crate::resolve::resolve(&caller, &declarations, &[]).unwrap();
        let caller_typed =
            crate::infer::infer(&caller, &caller_resolved, &declarations, &[])
                .unwrap();
        let mut frame = crate::frame::Frame::default();
        frame.accept(caller, caller_resolved, &caller_typed);
        let current = crate::parse::parse(crate::lex::lex("plus")).unwrap();
        let (resolved, errors) = crate::resolve::resolve_in(
            &current,
            &declarations,
            &[],
            Some(&frame),
        );
        assert!(errors.is_empty());
        let cache = crate::infer::files::FileCache::default();
        let mut infer = Infer::new(
            &current,
            &resolved,
            &declarations,
            &[],
            Some(&frame),
            &[],
            &cache,
        );
        let site = crate::token::Span { start: 0, end: 4 };
        let _callee = infer.local(site, current.span);
        assert!(
            !infer.state.obligations.is_empty(),
            "real accepted generic plus instantiates its Add obligation"
        );
        let count = infer.state.subst.count();
        let before: Vec<_> = (0..count)
            .map(|id| infer.apply(&Type::Var(crate::types::Var(id))))
            .collect();
        let snapshot_obligations = |infer: &Infer<'_>| {
            infer
                .state
                .obligations
                .iter()
                .map(|owed| {
                    (
                        owed.obligation.clone(),
                        owed.span,
                        owed.solved,
                        owed.assumed,
                        owed.literal.is_some(),
                        owed.literal_argument,
                    )
                })
                .collect::<Vec<_>>()
        };
        let owed = snapshot_obligations(&infer);
        infer.record_dispatch_input_tests(&typed.dispatch);
        assert!(infer.state.errors.is_empty());
        assert_eq!(
            (0..count)
                .map(|id| infer.apply(&Type::Var(crate::types::Var(id))))
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(snapshot_obligations(&infer), owed);
        assert!(infer.state.tests.iter().any(|test| test.origin == Origin::Host("methods".into())
            && matches!(&test.owner, Type::Named { name, .. } if name.name == "Tagged") && test.expression.is_some()));
        let tests = infer.state.tests.len();
        infer.record_dispatch_input_tests(&typed.dispatch);
        assert_eq!(
            infer.state.tests.len(),
            tests,
            "cached repeat does not duplicate owner tests"
        );
        assert_eq!(snapshot_obligations(&infer), owed);
    }

    #[test]
    fn invalid_declared_row_input_test_is_a_check_error() {
        let mut declarations = crate::decl::Declarations::new();
        declarations
            .module("methods", &HEADER.replace("self < 10", "self + True"))
            .unwrap();
        let module = crate::parse::parse(crate::lex::lex(MAIN)).unwrap();
        let errors = crate::check::check(
            &module,
            &declarations,
            &[],
            crate::check::Options::default(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|error| matches!(error,
            crate::check::Error::Type(crate::infer::Error { reason: crate::infer::Reason::DeclarationValue { name, origin, .. }, .. })
            if name == "Tagged" && *origin == Origin::Host("methods".into()))));
    }
}
