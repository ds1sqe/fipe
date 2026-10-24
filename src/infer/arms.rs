//! Arms and patterns (ARM; types.md §Fields, functions and branches):
//! one input type per head position across the arms, `Bool` guards,
//! one result type, and every pattern checked against its input.

use super::{prelude, prelude_of, Expected, Head, Infer, Reason};
use crate::{
    ast::{Arm, Module, PatternId, PatternKind},
    resolve::Target,
    token::Span,
    types::Type,
};
use alloc::vec::Vec;

impl Infer<'_> {
    pub(super) fn arms(
        &mut self,
        arms: &[Arm],
        expected: Option<&Expected>,
    ) -> Type {
        let Some(first) = arms.first() else {
            return self.fresh();
        };
        let count = first.patterns.len();
        if count > crate::parse::MAX_NESTING {
            self.error(first.span, Reason::TypeDepth);
            return self.fresh();
        }
        let mut declared = expected.map(|e| self.apply(&e.ty));
        let parameters: Vec<Type> = (0..count)
            .map(|_| {
                if let Some(Type::Function { parameter, result }) =
                    declared.take()
                {
                    declared = Some(*result);
                    *parameter
                } else {
                    self.fresh()
                }
            })
            .collect();
        let declared_result = declared.is_some();
        let result = declared.unwrap_or_else(|| self.fresh());
        let ty = parameters
            .iter()
            .rev()
            .fold(result.clone(), |ty, p| Type::function(p.clone(), ty));
        if let Some(expected) = expected {
            self.hint(&expected.ty, &ty);
        }
        let env = self.state.env.len();
        self.state.env.extend(parameters.iter().cloned());
        let input = expected.map_or(first.span, |e| e.span);
        let result_span = module_result_span(self.module, input);
        let mut sibling = Expected::new(result, result_span);
        sibling.cast = declared_result && expected.is_some_and(|e| e.cast);
        for (index, arm) in arms.iter().enumerate() {
            if arm.patterns.len() != count {
                let mut actual = self.fresh();
                for _ in &arm.patterns {
                    actual = Type::function(self.fresh(), actual);
                }
                let expected = self.apply(&ty);
                let other = first.span;
                self.error(
                    arm.span,
                    Reason::Mismatch {
                        expected,
                        actual,
                        other,
                    },
                );
                continue;
            }
            for (pattern, parameter) in arm.patterns.iter().zip(&parameters) {
                self.pattern(*pattern, parameter, input);
            }
            if let Some(guard) = arm.guard {
                let bool = Expected::new(prelude("Bool"), arm.span);
                self.expr(guard, Some(&bool));
            }
            self.expr(arm.body, Some(&sibling));
            if index == 0 {
                sibling.span = self.span(arm.body);
            }
        }
        for arm in arms {
            let heads = heads_in(self.module, core::slice::from_ref(arm));
            let spots: Vec<_> = heads
                .iter()
                .zip(&parameters)
                .map(|(head, ty)| {
                    (head.name.clone(), self.is_bool(ty), head.site)
                })
                .collect();
            for error in crate::decl::spots::validate(&spots, false) {
                if let crate::decl::Reason::Spots { problem } = error.reason {
                    self.error(error.span, Reason::Spots { problem });
                }
            }
        }
        self.state.env.truncate(env);
        ty
    }

    /// The spot list of an arm list in this module.
    pub(super) fn heads(&self, arms: &[Arm]) -> Vec<Head> {
        heads_in(self.module, arms)
    }

    /// Check a pattern against its input type, which `input` gave, and
    /// bind its names.
    pub(super) fn pattern(&mut self, id: PatternId, ty: &Type, input: Span) {
        self.insert(|s| &mut s.pattern_types, id.0, ty.clone());
        let module = self.module;
        let span = module.patterns[id.0].span;
        match &module.patterns[id.0].kind {
            PatternKind::Wildcard => {}
            PatternKind::Name(name) if !name.is_uppercase() => {
                self.bind_local(name, ty.clone());
            }
            PatternKind::Name(_) | PatternKind::Constructor { .. } => {
                self.constructor_pattern(id, ty, input);
            }
            PatternKind::Literal(literal) => {
                let expected = Expected::new(ty.clone(), input);
                let literal_ty = self.literal(literal, Some(&expected), span);

                self.unify(ty, &literal_ty, span, input);
                self.require_claim(ty.clone(), prelude("Eq"), span);
            }
            PatternKind::Default { name, value } => {
                let expected = Expected::new(ty.clone(), input);
                self.expr(*value, Some(&expected));
                self.bind_local(name, ty.clone());
            }
            PatternKind::Group(inner) => self.pattern(*inner, ty, input),
            PatternKind::Tuple(parts) => {
                let types: Vec<_> =
                    parts.iter().map(|_| self.fresh()).collect();
                self.unify(ty, &Type::Tuple(types.clone()), span, input);
                for (part, part_ty) in parts.iter().zip(types) {
                    self.pattern(*part, &self.apply(&part_ty), span);
                }
            }
            PatternKind::List(items) => {
                let element = self.fresh();
                let list = if super::is_prelude(&self.apply(ty), "Vec") {
                    prelude_of("Vec", alloc::vec![element.clone()])
                } else {
                    Type::Array {
                        element: alloc::boxed::Box::new(element.clone()),
                        length: alloc::boxed::Box::new(Type::Number(
                            (items.len() as u64).into(),
                        )),
                    }
                };
                self.unify(ty, &list, span, input);
                for item in items {
                    self.pattern(*item, &element, span);
                }
            }
            PatternKind::Outcome { good, payload } => {
                self.outcome_pattern(*good, *payload, ty, span, input);
            }
            PatternKind::Record { fields, .. } => {
                let target = self.resolved.pattern(id);
                let Some((record, declared)) = self.record_shape(target, span)
                else {
                    return;
                };
                self.unify(ty, &record, span, input);
                for field in fields {
                    let name = super::last(&field.name);
                    let field_ty = declared
                        .iter()
                        .find(|f| f.name == name)
                        .map_or_else(|| self.fresh(), |f| f.ty.clone());
                    match field.pattern {
                        Some(pattern) => self.pattern(pattern, &field_ty, span),
                        None => self.bind_local(&field.name, field_ty),
                    }
                }
            }
        }
    }

    /// `Just x`, `Nothing`: the constructor's result is the input, its
    /// one field the payload's.
    fn constructor_pattern(&mut self, id: PatternId, ty: &Type, input: Span) {
        let module = self.module;
        let span = module.patterns[id.0].span;
        let gate = match self.resolved.pattern(id) {
            Some(Target::Type { origin, name }) => Some((origin, name)),
            Some(Target::Constructor {
                origin,
                ty: owner,
                name,
            }) if owner == name
                && matches!(self.apply(ty), Type::Either(_)) =>
            {
                Some((origin, name))
            }
            _ => None,
        };
        if let Some((origin, name)) = gate {
            let meaning =
                self.limited(Type::named(origin.clone(), name, Vec::new()));
            if let Type::Either(parts) = self.apply(ty) {
                if !parts.contains(&meaning) {
                    self.unify(ty, &meaning, span, input);
                }
            } else {
                self.unify(ty, &meaning, span, input);
            }
            if let PatternKind::Constructor { payload, .. } =
                module.patterns[id.0].kind
            {
                self.pattern(payload, &meaning, input);
            }
            return;
        }
        let Some(Target::Constructor {
            origin,
            ty: owner,
            name,
        }) = self.resolved.pattern(id)
        else {
            return;
        };
        let (result, fields) = self.constructor_fields(origin, owner, name);
        self.unify(ty, &result, span, input);
        if let PatternKind::Constructor { payload, .. } =
            &module.patterns[id.0].kind
        {
            let payload_ty = fields.first().map(|field| field.ty.clone());
            let payload_ty = payload_ty.unwrap_or_else(|| {
                // A constructor without fields takes no payload; the
                // payload's names are still bound, at nothing known.
                let payload_span = module.patterns[payload.0].span;
                let name = name.clone();
                self.error(payload_span, Reason::NoPayload { name });
                self.fresh()
            });
            self.pattern(*payload, &payload_ty, span);
        }
    }

    /// `[:e]`, `[!e]`: a side of the input's outcome family.
    fn outcome_pattern(
        &mut self,
        good: bool,
        payload: Option<PatternId>,
        ty: &Type,
        span: Span,
        input: Span,
    ) {
        let applied = self.apply(ty);
        let side = self.bracket_side(
            &applied,
            (good, payload.is_some(), true),
            span,
            input,
        );
        if let Some(payload) = payload {
            let payload_ty = side.unwrap_or_else(|| self.fresh());
            self.pattern(payload, &payload_ty, span);
        }
    }
}

/// The spot list of an arm list: its first arm's head names, and which
/// default.
pub(super) fn heads_in(module: &Module, arms: &[Arm]) -> Vec<Head> {
    let head = |mut id: PatternId| loop {
        match &module.patterns[id.0].kind {
            PatternKind::Name(name) if !name.is_uppercase() => {
                break Head {
                    name: Some(super::last(name)),
                    default: false,
                    value: None,
                    site: module.patterns[id.0].span,
                };
            }
            PatternKind::Default { name, value } => {
                break Head {
                    name: Some(super::last(name)),
                    default: true,
                    value: Some(*value),
                    site: name.span,
                };
            }
            PatternKind::Group(inner) => id = *inner,
            _ => {
                break Head {
                    name: None,
                    default: false,
                    value: None,
                    site: module.patterns[id.0].span,
                };
            }
        }
    };
    arms.first()
        .map(|arm| arm.patterns.iter().map(|p| head(*p)).collect())
        .unwrap_or_default()
}

fn module_result_span(module: &Module, span: Span) -> Span {
    let Some(mut ty) = module.types.iter().find(|ty| ty.span == span) else {
        return span;
    };
    loop {
        let next = match ty.kind {
            crate::ast::TypeKind::Function { result, .. } => result,
            crate::ast::TypeKind::Qualified { ty, .. } => ty,
            _ => return ty.span,
        };
        ty = &module.types[next.0];
    }
}

impl Infer<'_> {
    pub(super) fn record_pattern_equals(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let patterns: Vec<_> = self
            .state
            .pattern_types
            .iter()
            .filter(|(id, _)| {
                matches!(
                    self.module.patterns[**id].kind,
                    PatternKind::Literal(_)
                )
            })
            .map(|(id, ty)| (*id, self.apply(ty)))
            .collect();
        for (id, ty) in patterns {
            let span = self.module.patterns[id].span;
            let head = super::meaning(&ty).clone();
            let mut comparison = crate::typed::Comparison {
                selected: crate::typed::Selected::Member {
                    head: head.clone(),
                    name: alloc::string::String::from("eq"),
                    argument: Some(head.clone()),
                },
                meanings: Vec::new(),
                ty: Type::function(
                    head.clone(),
                    Type::function(head.clone(), prelude("Bool")),
                ),
                self_slot: 1,
            };
            if !matches!(head, Type::Var(_) | Type::Rigid(_)) {
                if let Some(callee) = self.self_function("eq", &head, span) {
                    if let Some(spots) = &callee.spots {
                        for spot in spots {
                            self.unify(&spot.ty, &head, span, span);
                        }
                    }
                    if let Some(selected) = &callee.selected {
                        comparison.selected = selected.clone();
                        comparison.meanings.clone_from(&callee.meanings);
                        comparison.ty = self.apply(&callee.whole());
                        comparison.self_slot = callee.self_slot.unwrap_or(1);
                    }
                }
            }
            self.insert(|s| &mut s.pattern_equals, id, comparison);
        }
    }
}
