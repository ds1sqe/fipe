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
        let parameters: Vec<Type> = (0..count).map(|_| self.fresh()).collect();
        let result = self.fresh();
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
        let mut sibling = Expected::new(result, input);
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
            }
            PatternKind::Default { name, value } => {
                let expected = Expected::new(ty.clone(), input);
                self.expr(*value, Some(&expected));
                self.bind_local(name, ty.clone());
            }
            PatternKind::Group(inner) => self.pattern(*inner, ty, input),
            // Not yet checked, which resolve reports: its names bind.
            PatternKind::Tuple(parts) => {
                for part in parts {
                    let part_ty = self.fresh();
                    self.pattern(*part, &part_ty, span);
                }
            }
            PatternKind::List(items) => {
                let element = self.fresh();
                let list = prelude_of("Vec", alloc::vec![element.clone()]);
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
                let Some((record, declared)) = self.record_shape(target) else {
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
            let meaning = Type::named(origin.clone(), name, Vec::new());
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
            let payload_ty = match fields.as_slice() {
                [field] => field.ty.clone(),
                [] => Type::Unit,
                _ => self.fresh(),
            };
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
        let side =
            self.bracket_side(&applied, good, payload.is_some(), span, input);
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
                };
            }
            PatternKind::Default { name, .. } => {
                break Head {
                    name: Some(super::last(name)),
                    default: true,
                };
            }
            PatternKind::Group(inner) => id = *inner,
            _ => {
                break Head {
                    name: None,
                    default: false,
                };
            }
        }
    };
    arms.first()
        .map(|arm| arm.patterns.iter().map(|p| head(*p)).collect())
        .unwrap_or_default()
}
