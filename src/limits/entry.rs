use super::Range;
use super::{
    bool_op, Check, Error, Frequency, Known, Limit, Module, Origin, Pass,
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
        projection: crate::ir::Projection,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        let previous = core::mem::replace(&mut self.place_kind, kind);
        self.path.push(projection);
        self.enter(ty, known, place, frequency);
        self.path.pop();
        self.place_kind = previous;
    }

    pub(super) fn enter(
        &mut self,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        let start = self.typed.checks.len();
        match ty {
            Type::Limited { name, .. } => {
                self.enter(name, known, place, frequency);
            }
            Type::Named { name, arguments }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Vec" | "Deque")
                    && arguments.len() == 1 =>
            {
                self.element_entries(&arguments[0], known, place);
            }
            Type::Named { name, arguments }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Maybe" | "Result")
                    && !arguments.is_empty() =>
            {
                self.payloads(arguments, known, place, frequency);
            }
            Type::Tuple(parts) => {
                self.tuple_entries(parts, known, place, frequency);
            }
            Type::Array { element, .. } => {
                self.element_entries(element, known, place);
            }
            Type::Named { name, arguments } => {
                self.named_parts(name, arguments, known, place, frequency);
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
        for check in &mut self.typed.checks[start..] {
            if check.target.is_none() {
                check.target = Some(ty.clone());
            }
        }
    }
    fn named_parts(
        &mut self,
        name: &TypeName,
        arguments: &[Type],
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        if !arguments.iter().any(super::has_limits) {
            return;
        }
        let Some(module) = self.owner(&name.origin) else {
            return;
        };
        let Some(index) = self.declarations_by_name.get(name) else {
            return;
        };
        let crate::ast::Entry::Item(item) = &module.entries[*index] else {
            return;
        };
        let crate::ast::ItemKind::Type {
            parameters,
            members,
            ..
        } = &item.kind
        else {
            return;
        };
        let bindings: Vec<_> = parameters
            .iter()
            .zip(arguments)
            .map(|(p, ty)| {
                (
                    p.name.segments.last().cloned().unwrap_or_default(),
                    ty.clone(),
                )
            })
            .collect();
        let constructor = known.constructor.as_deref().unwrap_or(&name.name);
        let fields =
            super::construction_fields(module, &name.name, constructor);
        if fields.is_empty() && known.constructor.is_none() {
            let parts = self.sum_parts(
                module,
                name,
                members.as_deref().unwrap_or(&[]),
                &bindings,
            );
            for (constructor, field, ty) in parts {
                self.enter_part(
                    Place::Payload,
                    crate::ir::Projection::CaseField { constructor, field },
                    &ty,
                    &Known::default(),
                    place,
                    frequency,
                );
            }
            return;
        }
        let parts: Vec<_> = fields
            .iter()
            .map(|field| {
                let label =
                    field.name.segments.last().cloned().unwrap_or_default();
                let ty = self.syntax_bound(
                    module,
                    &name.origin,
                    field.ty,
                    &bindings,
                );
                let fact = known.part(&label);
                (label, ty, fact)
            })
            .collect();
        for (index, (label, ty, fact)) in parts.into_iter().enumerate() {
            self.enter_part(
                Place::Field {
                    owner: name.name.clone(),
                    name: label,
                },
                crate::ir::Projection::Field(index),
                &ty,
                &fact,
                place,
                frequency,
            );
        }
    }
    fn sum_parts(
        &self,
        module: &Module,
        name: &TypeName,
        members: &[crate::ast::Member],
        bindings: &[(String, Type)],
    ) -> Vec<(usize, usize, Type)> {
        let constructors: Vec<_> = members
            .iter()
            .filter_map(|member| {
                if let crate::ast::Member::Constructor { fields, .. } = member {
                    Some(fields)
                } else {
                    None
                }
            })
            .collect();
        constructors
            .iter()
            .enumerate()
            .flat_map(|(constructor, fields)| {
                fields
                    .iter()
                    .enumerate()
                    .map(move |(field, value)| (constructor, field, value))
            })
            .map(|(constructor, field, value)| {
                (
                    constructor,
                    field,
                    self.syntax_bound(module, &name.origin, value.ty, bindings),
                )
            })
            .collect()
    }
    fn element_entries(&mut self, element: &Type, known: &Known, place: Span) {
        let parts: Vec<_> = known
            .parts
            .iter()
            .filter(|(name, _)| name != "len")
            .collect();
        if !parts.is_empty() {
            for (_, part) in parts {
                self.enter_part(
                    Place::Element,
                    crate::ir::Projection::Element,
                    element,
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
                crate::ir::Projection::Element,
                element,
                &Known {
                    narrowing: known.narrowing,
                    ..Known::default()
                },
                place,
                Frequency::PerElement,
            );
        }
    }
    fn tuple_entries(
        &mut self,
        parts: &[Type],
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        let facts: alloc::collections::BTreeMap<_, _> =
            known.parts.iter().map(|(n, k)| (n.as_str(), k)).collect();
        for (index, ty) in parts.iter().enumerate() {
            let fact = facts.get(index.to_string().as_str()).map_or_else(
                || Known {
                    narrowing: known.narrowing,
                    ..Known::default()
                },
                |k| (*k).clone(),
            );
            self.enter_part(
                Place::Part(index),
                crate::ir::Projection::Part(index),
                ty,
                &fact,
                place,
                frequency,
            );
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
                        crate::ir::Projection::Payload(index),
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
                        crate::ir::Projection::Payload(index),
                        argument,
                        part,
                        place,
                        frequency,
                    );
                }
            }
        }
    }
    pub(super) fn evaluate_type(
        &self,
        ty: &Type,
        known: &Known,
        depth: usize,
    ) -> Known {
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
                let meaning = module.entries.iter().any(|entry| {
                    matches!(entry,
                    crate::ast::Entry::Item(item) if matches!(&item.kind,
                    crate::ast::ItemKind::Type { name: n, members: Some(_), .. }
                    if n.segments.last() == Some(&name.name)))
                });
                let mut result = base.filter(|_| !meaning).map_or_else(
                    || Known {
                        boolean: if super::modifier(ty) {
                            Some(true)
                        } else {
                            known
                                .meaning
                                .as_ref()
                                .map(|actual| actual == name)
                                .or_else(|| {
                                    if known.boolean.is_some() {
                                        Some(
                                            name.origin == Origin::Prelude
                                                && name.name == "Bool",
                                        )
                                    } else if known.range.is_some() {
                                        Some(
                                            name.origin == Origin::Prelude
                                                && name.name == "Integer",
                                        )
                                    } else {
                                        None
                                    }
                                })
                        },
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
                            &self.test_value(module, &name.origin, *id, known),
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
        self.syntax_bound(module, origin, id, &[])
    }
    pub(super) fn syntax_bound(
        &self,
        module: &Module,
        origin: &Origin,
        id: TypeId,
        bindings: &[(String, Type)],
    ) -> Type {
        if let TypeKind::Named { name, arguments } = &module.types[id.0].kind {
            if arguments.is_empty() {
                if let Some((_, ty)) = bindings
                    .iter()
                    .find(|(n, _)| name.segments.last() == Some(n))
                {
                    return ty.clone();
                }
            }
        }
        match &module.types[id.0].kind {
            TypeKind::Named { name, arguments }
                if arguments.is_empty()
                    && self.type_name(module, origin, id).is_none() =>
            {
                Type::Rigid(name.segments.last().cloned().unwrap_or_default())
            }
            TypeKind::Tuple(parts) => Type::Tuple(
                parts
                    .iter()
                    .map(|p| self.syntax_bound(module, origin, *p, bindings))
                    .collect(),
            ),
            TypeKind::Both(parts) => Type::Both(
                parts
                    .iter()
                    .map(|p| self.syntax_bound(module, origin, *p, bindings))
                    .collect(),
            ),
            TypeKind::Either(parts) => Type::Either(
                parts
                    .iter()
                    .map(|p| self.syntax_bound(module, origin, *p, bindings))
                    .collect(),
            ),
            TypeKind::Not(inner) => Type::Not(alloc::boxed::Box::new(
                self.syntax_bound(module, origin, *inner, bindings),
            )),
            TypeKind::Number(n) => Type::Number(n.clone()),
            TypeKind::Sum(parts) => Type::sum(
                parts
                    .iter()
                    .map(|p| self.syntax_bound(module, origin, *p, bindings))
                    .collect(),
            ),
            TypeKind::Array { element, length } => Type::Array {
                element: alloc::boxed::Box::new(
                    self.syntax_bound(module, origin, *element, bindings),
                ),
                length: alloc::boxed::Box::new(
                    self.syntax_bound(module, origin, *length, bindings),
                ),
            },
            TypeKind::Function { parameter, result } => Type::function(
                self.syntax_bound(module, origin, *parameter, bindings),
                self.syntax_bound(module, origin, *result, bindings),
            ),
            _ => {
                self.type_name(module, origin, id)
                    .map_or(Type::Unit, |name| Type::Named {
                        name,
                        arguments: if let TypeKind::Named {
                            arguments, ..
                        } = &module.types[id.0].kind
                        {
                            arguments
                                .iter()
                                .map(|id| {
                                    self.syntax_bound(
                                        module, origin, *id, bindings,
                                    )
                                })
                                .collect()
                        } else {
                            Vec::new()
                        },
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
        match if result.failure == Some(super::known::Failure::Fault) {
            Some(false)
        } else {
            result.boolean
        } {
            Some(true) => (),
            Some(false) => self.errors.push(Error {
                span: place,
                reason: TypeReason::Limit {
                    message: {
                        let facts = known.to_string();
                        if facts.trim().is_empty() {
                            format!("refuted entry against {name}")
                        } else {
                            format!(
                                "refuted entry against {name}: {}",
                                facts.trim_end()
                            )
                        }
                    },
                },
            }),
            None => {
                if !self.checks_seen.insert((
                    place,
                    self.place_kind.clone(),
                    limit,
                    origin.clone(),
                    self.path.clone(),
                )) {
                    return;
                }
                self.typed.checks.push(Check {
                    value: None,
                    path: self.path.clone(),
                    target: None,
                    place,
                    place_kind: self.place_kind.clone(),
                    limit,
                    phase: if known.is_exact()
                        || known.demanded
                        || self.entry_phase == Some(Phase::Compile)
                    {
                        Phase::Compile
                    } else if frequency == Frequency::PerCall {
                        Phase::Eval
                    } else {
                        self.site_phase(place)
                    },
                    reason: if known.demanded
                        || known.is_exact()
                        || self.entry_phase == Some(Phase::Compile)
                    {
                        Reason::CompileTimeEvaluation
                    } else if known.narrowing {
                        Reason::Narrowing
                    } else {
                        Reason::UnknownInput
                    },
                    frequency,
                    place_origin: Origin::This,
                    limit_origin: origin,
                    name,
                    test_count: 1,
                    known: known.clone(),
                    gate: false,
                    when_integer: false,
                    owner: self.entry_owner(place),
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
                        self.test_value(module, &name.origin, *id, known),
                        self.timed_test(module, &name.origin, *id)
                            .map(|(phase, _)| phase),
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
        let test_count = tests.len();
        for (limit, result, timing) in tests {
            if result.boolean.is_none() {
                let blocked = match timing {
                    Some(Phase::Compile) => {
                        !known.is_exact()
                            && !known.demanded
                            && self.entry_phase != Some(Phase::Compile)
                    }
                    Some(Phase::Init) => {
                        frequency == Frequency::PerCall
                            || self.site_phase(place) == Phase::Eval
                    }
                    _ => false,
                };
                if blocked {
                    self.errors.push(Error {
                        span: place,
                        reason: TypeReason::Limit {
                            message: format!(
                                "{} test must be proved at this entry",
                                timing.unwrap()
                            ),
                        },
                    });
                    continue;
                }
            }
            let start = self.typed.checks.len();
            self.outcome(
                (name.name.clone(), name.origin.clone(), limit),
                place,
                frequency,
                known,
                &result,
            );
            for check in &mut self.typed.checks[start..] {
                check.test_count = test_count;
            }
            if result.boolean == Some(false)
                || result.failure == Some(super::known::Failure::Fault)
            {
                break;
            }
        }
        self.active.pop();
    }
}
