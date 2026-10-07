use super::Range;
use super::{
    bool_op, test, Check, Error, Frequency, Known, Limit, Module, Origin, Pass,
    Phase, Place, Reason, Span, Type, TypeId, TypeKind, TypeName, TypeReason,
};
use crate::ast::PipeOp;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

impl Pass<'_> {
    fn enter_part(
        &mut self,
        kind: Place,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        let previous = core::mem::replace(&mut self.place_kind, kind);
        self.enter(ty, known, place, frequency);
        self.place_kind = previous;
    }

    pub(super) fn enter(
        &mut self,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        match ty {
            Type::Limited { name, .. } => {
                self.enter(name, known, place, frequency);
            }
            Type::Named { name, arguments }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Vec" | "Deque")
                    && arguments.len() == 1 =>
            {
                let parts: Vec<_> = known
                    .parts
                    .iter()
                    .filter(|(name, _)| name != "len")
                    .collect();
                if !parts.is_empty() {
                    for (_, part) in parts {
                        self.enter_part(
                            Place::Element,
                            &arguments[0],
                            part,
                            place,
                            Frequency::PerElement,
                        );
                    }
                } else if !known.parts.iter().any(|(name, k)| {
                    name == "len" && k.range == Some(Range::exact(0.into()))
                }) {
                    self.enter_part(
                        Place::Element,
                        &arguments[0],
                        &Known::default(),
                        place,
                        Frequency::PerElement,
                    );
                }
            }
            Type::Named { name, arguments }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Maybe" | "Result")
                    && !arguments.is_empty() =>
            {
                self.payloads(arguments, known, place, frequency);
            }
            Type::Array { element, .. } => self.enter_part(
                Place::Element,
                element,
                &Known::default(),
                place,
                Frequency::PerElement,
            ),
            Type::Named { name, .. } => {
                self.named_entry(
                    name,
                    known,
                    place,
                    frequency,
                    &mut Vec::new(),
                );
            }
            Type::Function { .. } => {
                let known = Known {
                    narrowing: true,
                    ..known.clone()
                };
                self.outcome(
                    (ty.to_string(), Origin::This, place),
                    place,
                    frequency,
                    &known,
                    &Known::default(),
                );
            }
            Type::Both(parts) => {
                for part in parts {
                    self.enter(part, known, place, frequency);
                }
            }
            Type::Either(_) | Type::Not(_) => {
                let result = self.evaluate_type(ty, known, 0);
                self.outcome(
                    (ty.to_string(), Origin::This, place),
                    place,
                    frequency,
                    known,
                    &result,
                );
            }
            _ => (),
        }
    }
    fn payloads(
        &mut self,
        arguments: &[Type],
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        match known.constructor.as_deref() {
            Some("Nothing") => (),
            Some("Just" | "Ok" | "Err") => {
                let index =
                    usize::from(known.constructor.as_deref() == Some("Err"));
                if let Some(argument) = arguments.get(index) {
                    let payload = known
                        .parts
                        .first()
                        .map_or_else(Known::default, |(_, part)| part.clone());
                    self.enter_part(
                        Place::Payload,
                        argument,
                        &payload,
                        place,
                        frequency,
                    );
                }
            }
            _ => {
                let payload = Known {
                    narrowing: known.narrowing,
                    ..Known::default()
                };
                for (index, argument) in arguments.iter().enumerate() {
                    let name = if index == 0 { "Ok" } else { "Err" };
                    let part = known
                        .parts
                        .iter()
                        .find(|(label, _)| label == name)
                        .map_or(&payload, |(_, part)| part);
                    self.enter_part(
                        Place::Payload,
                        argument,
                        part,
                        place,
                        frequency,
                    );
                }
            }
        }
    }
    fn evaluate_type(&self, ty: &Type, known: &Known, depth: usize) -> Known {
        if depth > crate::parse::MAX_NESTING {
            return Known::default();
        }
        match ty {
            Type::Limited { name, .. } => {
                self.evaluate_type(name, known, depth + 1)
            }
            Type::Not(inner) => {
                let mut result = self.evaluate_type(inner, known, depth + 1);
                result.boolean = result.boolean.map(|b| !b);
                result
            }
            Type::Both(parts) | Type::Either(parts) => {
                let op = if matches!(ty, Type::Both(_)) {
                    PipeOp::And
                } else {
                    PipeOp::Or
                };
                parts.iter().fold(
                    Known {
                        boolean: Some(op == PipeOp::And),
                        ..Known::default()
                    },
                    |result, part| {
                        bool_op(
                            op,
                            &result,
                            &self.evaluate_type(part, known, depth + 1),
                        )
                    },
                )
            }
            Type::Named { name, .. } => {
                if known.names.contains(name) {
                    return Known {
                        boolean: Some(true),
                        ..Known::default()
                    };
                }
                let Some((module, base, limits)) = self.declaration(name)
                else {
                    return Known::default();
                };
                let mut result = base.map_or_else(
                    || Known {
                        boolean: Some(true),
                        ..Known::default()
                    },
                    |base| {
                        self.evaluate_type(
                            &self.syntax_type(module, &name.origin, base),
                            known,
                            depth + 1,
                        )
                    },
                );
                for limit in limits {
                    if let Limit::Test(id) = limit {
                        result = bool_op(
                            PipeOp::And,
                            &result,
                            &test(module, *id, known),
                        );
                    }
                }
                result
            }
            _ => Known {
                boolean: Some(true),
                ..Known::default()
            },
        }
    }
    pub(super) fn syntax_type(
        &self,
        module: &Module,
        origin: &Origin,
        id: TypeId,
    ) -> Type {
        match &module.types[id.0].kind {
            TypeKind::Both(parts) => Type::Both(
                parts
                    .iter()
                    .map(|p| self.syntax_type(module, origin, *p))
                    .collect(),
            ),
            TypeKind::Either(parts) => Type::Either(
                parts
                    .iter()
                    .map(|p| self.syntax_type(module, origin, *p))
                    .collect(),
            ),
            TypeKind::Not(inner) => Type::Not(alloc::boxed::Box::new(
                self.syntax_type(module, origin, *inner),
            )),
            _ => {
                self.type_name(module, origin, id)
                    .map_or(Type::Unit, |name| Type::Named {
                        name,
                        arguments: Vec::new(),
                    })
            }
        }
    }
    fn outcome(
        &mut self,
        (name, origin, limit): (String, Origin, Span),
        place: Span,
        frequency: Frequency,
        known: &Known,
        result: &Known,
    ) {
        match result.boolean {
            Some(true) => (),
            Some(false) => self.errors.push(Error {
                span: place,
                reason: TypeReason::Limit {
                    message: format!("refuted entry against {name}: {known}"),
                },
            }),
            None => {
                if self.typed.checks.iter().any(|c| {
                    c.place == place
                        && c.place_kind == self.place_kind
                        && c.limit == limit
                        && c.limit_origin == origin
                }) {
                    return;
                }
                self.typed.checks.push(Check {
                    place,
                    place_kind: self.place_kind.clone(),
                    limit,
                    phase: Phase::Eval,
                    reason: if known.narrowing {
                        Reason::Narrowing
                    } else if result.narrowing && known.is_exact() {
                        Reason::CompileTimeEvaluation
                    } else if known.is_exact() {
                        Reason::NoRule
                    } else {
                        Reason::UnknownInput
                    },
                    frequency,
                    place_origin: Origin::This,
                    limit_origin: origin,
                    name,
                    known: known.clone(),
                    gate: false,
                });
            }
        }
    }
    pub(super) fn named_entry(
        &mut self,
        name: &TypeName,
        known: &Known,
        place: Span,
        frequency: Frequency,
        visited: &mut Vec<TypeName>,
    ) {
        if known.names.contains(name)
            || visited.contains(name)
            || self.active.contains(name)
        {
            return;
        }
        visited.push(name.clone());
        let Some((module, base, limits)) = self.declaration(name) else {
            return;
        };
        let base = base.map(|base| {
            (
                self.syntax_type(module, &name.origin, base),
                module.types[base.0].span,
            )
        });
        let tests: Vec<_> = limits
            .iter()
            .filter_map(|l| {
                if let Limit::Test(id) = l {
                    Some((
                        module.expressions[id.0].span,
                        test(module, *id, known),
                    ))
                } else {
                    None
                }
            })
            .collect();
        self.active.push(name.clone());
        if let Some((base, span)) = base {
            let start = self.typed.checks.len();
            self.enter(&base, known, place, frequency);
            for check in &mut self.typed.checks[start..] {
                if check.limit == place && check.limit_origin == Origin::This {
                    check.limit = span;
                    check.limit_origin = name.origin.clone();
                }
            }
        }
        for (limit, result) in tests {
            self.outcome(
                (name.name.clone(), name.origin.clone(), limit),
                place,
                frequency,
                known,
                &result,
            );
        }
        self.active.pop();
    }
}
