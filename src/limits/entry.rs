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
        use crate::ir::Projection as P;
        // The entering value's own type follows it into its parts, as
        // far as its structure shows them; a record's field has the
        // place's field type when the value is of the place's type.
        let value_ty = self.value_ty.as_ref().and_then(|value| {
            match (&projection, crate::infer::meaning(value)) {
                (P::Part(i), Type::Tuple(parts)) => parts.get(*i).cloned(),
                (P::Element, Type::Array { element, .. }) => {
                    Some((**element).clone())
                }
                (P::Element, Type::Named { arguments, .. }) => {
                    arguments.first().cloned()
                }
                (P::Payload(i), Type::Named { arguments, .. }) => {
                    arguments.get(*i).cloned()
                }
                (P::Field(_) | P::CaseField { .. }, _)
                    if self.value_is_place =>
                {
                    Some(ty.clone())
                }
                _ => None,
            }
        });
        let previous_ty = core::mem::replace(&mut self.value_ty, value_ty);
        let is_place = self.value_is_place;
        let previous = core::mem::replace(&mut self.place_kind, kind);
        self.path.push(projection);
        self.enter(ty, known, place, frequency);
        self.path.pop();
        self.place_kind = previous;
        self.value_ty = previous_ty;
        self.value_is_place = is_place;
    }

    /// A top-level entry, with what is known at each variable parameter
    /// when the value is a function argument (07-checks §What is known
    /// 8).
    pub(super) fn enter_value(
        &mut self,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
        takes: Option<&[Option<Known>]>,
    ) {
        if matches!(ty, Type::Function { .. }) {
            self.function_entry(
                ty,
                known,
                place,
                frequency,
                takes.unwrap_or(&[]),
            );
        } else {
            self.enter(ty, known, place, frequency);
        }
    }

    /// A function value enters a place of function type only when
    /// proved (07-checks §What is known 8): each parameter of the place
    /// enters the value's, and the value's result enters the place's,
    /// flipping at each arrow. Refuted or unproved, it is a check error
    /// in every mode; but a parameter at a callee's variable where what
    /// goes in is not known to fit is an open entry, tested on each call
    /// of the value (ruled 2026-10-22).
    fn function_entry(
        &mut self,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
        takes: &[Option<Known>],
    ) {
        let value_ty = self.value_ty.clone();
        let outer = core::mem::take(&mut self.opens);
        let outcome = self.function_outcome(
            ty,
            value_ty.as_ref(),
            known,
            (takes, 0),
            place,
        );
        let opens = core::mem::replace(&mut self.opens, outer);
        let message = match outcome {
            Some(true) if opens.is_empty() => return,
            Some(true) => {
                self.open_function(ty, &opens, known, place, frequency);
                return;
            }
            Some(false) => format!("refuted entry against {ty}"),
            None => format!("unproved function entry against {ty}"),
        };
        self.errors.push(Error {
            span: place,
            reason: TypeReason::Limit { message },
        });
    }

    /// The open entry of a function value at a callee's variables: the
    /// value's own limits at each open parameter, tested per call.
    pub(super) fn open_function(
        &mut self,
        ty: &Type,
        opens: &[(usize, Type)],
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        let mut parameters = Vec::new();
        let mut rest = ty;
        let last = opens.iter().map(|(i, _)| *i).max().unwrap_or(0);
        while let Type::Function { parameter, result } = rest {
            if parameters.len() > last {
                break;
            }
            let own = opens.iter().find(|(i, _)| *i == parameters.len());
            parameters.push(
                own.map_or_else(|| (**parameter).clone(), |(_, t)| t.clone()),
            );
            rest = result;
        }
        let target = parameters
            .into_iter()
            .rev()
            .fold(rest.clone(), |r, p| Type::function(p, r));
        let start = self.typed.checks.len();
        let unknown = Known {
            function: known.function.clone(),
            ..Known::default()
        };
        self.outcome(
            (target.to_string(), Origin::This, place),
            place,
            frequency,
            &unknown,
            &Known::default(),
        );
        for check in &mut self.typed.checks[start..] {
            check.target = Some(target.clone());
        }
    }

    /// Whether a value of type `value_ty`, known as `value`, enters a
    /// place of type `place`; `takes` is what is known at each of the
    /// place's parameters that is a variable of a callee's scheme, from
    /// the position given. A parameter there not known to fit is left
    /// to `opens`, with the value's own parameter type.
    pub(super) fn function_outcome(
        &mut self,
        place: &Type,
        value_ty: Option<&Type>,
        value: &Known,
        (takes, position): (&[Option<Known>], usize),
        span: Span,
    ) -> Option<bool> {
        if value.is_dead()
            || value.failure == Some(super::known::Failure::Dependency)
        {
            return Some(true);
        }
        let Type::Function { parameter, .. } = place else {
            return self.settled(place, value, span);
        };
        let at = takes.first().cloned().flatten();
        let (own_parameter, own_result) =
            match value_ty.map(crate::infer::meaning) {
                Some(Type::Function { parameter, result }) => {
                    (Some(&**parameter), Some(&**result))
                }
                _ => (None, None),
            };
        // A value known as a function is tested by its known parameter
        // limits: its own type may have lost them at a type variable.
        let known_parameter = value
            .function
            .as_deref()
            .map(|function| function.parameters.first().cloned().flatten());
        if known_parameter == Some(None) {
            return self.function_result(
                place,
                own_result,
                value,
                (takes, position),
                span,
                Some(true),
            );
        }
        let known_parameter = known_parameter.flatten();
        let own_parameter = known_parameter.as_ref().or(own_parameter);
        let entering = at.clone().unwrap_or_else(|| {
            let mut known = Known::default();
            self.place_facts(parameter, &mut known);
            known
        });
        // Parameters wider: what the place gives enters the value's
        // parameter. A value whose own type is unknown is not proved.
        let accepts = match own_parameter {
            None => None,
            Some(own) if at.is_none() && own == &**parameter => Some(true),
            Some(own @ Type::Function { .. }) => self.function_outcome(
                own,
                Some(parameter),
                &entering,
                (&[], 0),
                span,
            ),
            Some(own) => self.settled(own, &entering, span),
        };
        let accepts = match (accepts, own_parameter) {
            (None, Some(own)) if at.is_some() => {
                self.opens.push((position, own.clone()));
                Some(true)
            }
            _ => accepts,
        };
        if accepts == Some(false) {
            return accepts;
        }
        self.function_result(
            place,
            own_result,
            value,
            (takes, position),
            span,
            accepts,
        )
    }

    /// The rest of [`Self::function_outcome`] once the first parameter
    /// `accepts`: results narrower, what the value gives past it.
    fn function_result(
        &mut self,
        place: &Type,
        own_result: Option<&Type>,
        value: &Known,
        (takes, position): (&[Option<Known>], usize),
        span: Span,
        accepts: Option<bool>,
    ) -> Option<bool> {
        let Type::Function { result, .. } = place else {
            return accepts;
        };
        let mut rest = value
            .function
            .as_deref()
            .map_or_else(Known::default, |function| function.after(1));
        // A value with no function knowledge is known by its type's
        // result limits; one with it, by what it is known to give: an
        // arm list without a signature has none of the type it was
        // checked against.
        if let (Some(own), None) = (own_result, &value.function) {
            self.place_facts(own, &mut rest);
        }
        let given = if matches!(**result, Type::Function { .. }) {
            self.function_outcome(
                result,
                own_result,
                &rest,
                (takes.get(1..).unwrap_or(&[]), position + 1),
                span,
            )
        } else {
            self.settled(result, &rest, span)
        };
        match (accepts, given) {
            (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }
    }

    /// The outcome of entering `ty` with `known`, recording nothing.
    fn settled(
        &mut self,
        ty: &Type,
        known: &Known,
        span: Span,
    ) -> Option<bool> {
        let (checks, errors) = (self.typed.checks.len(), self.errors.len());
        let seen = core::mem::take(&mut self.checks_seen);
        let path = core::mem::take(&mut self.path);
        let value_ty = self.value_ty.take();
        self.enter(ty, known, span, Frequency::Once);
        let outcome = if self.errors.len() > errors {
            Some(false)
        } else if self.typed.checks.len() > checks {
            None
        } else {
            Some(true)
        };
        self.typed.checks.truncate(checks);
        self.errors.truncate(errors);
        self.checks_seen = seen;
        self.path = path;
        self.value_ty = value_ty;
        outcome
    }

    pub(super) fn enter(
        &mut self,
        ty: &Type,
        known: &Known,
        place: Span,
        frequency: Frequency,
    ) {
        // A value that never arrives, as a `fault` call's, is dead code
        // and proves anything (07-checks §What is known 3, 4).
        if known.failure == Some(super::known::Failure::Never) {
            return;
        }
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
                self.function_entry(ty, known, place, frequency, &[]);
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
        if !arguments.iter().any(super::has_limits) && !known.carries() {
            return;
        }
        let value_is_place = self.value_ty.as_ref().is_some_and(|value| {
            matches!(crate::infer::meaning(value),
                Type::Named { name: n, arguments: a }
                    if n == name && a.as_slice() == arguments)
        });
        self.value_is_place = value_is_place;
        let Some(module) = self.owner(&name.origin) else {
            return;
        };
        let Some(index) = self.declarations_by_name().get(name) else {
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
            // Elements known alike, the bytes of a text, meet the limits
            // they have passed; otherwise nothing is known of one.
            let each = known.element.as_deref().cloned().unwrap_or_default();
            self.enter_part(
                Place::Element,
                crate::ir::Projection::Element,
                element,
                &each,
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
            let fact = facts
                .get(index.to_string().as_str())
                .map_or_else(Known::default, |k| (*k).clone());
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
                let payload = Known::default();
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
