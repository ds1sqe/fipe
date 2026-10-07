//! Application: spots bind first, then APP ([evaluation.md] §Parameter
//! binding, steps 1 to 4). A callee with a spot list is a declared
//! function, a constructor, or a binding whose value is an arm list;
//! anything else applies positionally and curried.
//!
//! [evaluation.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/08-evaluation.md

use super::{
    is_prelude, prelude, Callee, Expected, Head, Infer, Key, Local, Reason,
    Slot, State,
};
use crate::{
    ast::{Argument, ArgumentKind, ExprId, ExprKind},
    resolve::{Origin, Target},
    token::Span,
    types::{Scheme, Type},
};

use alloc::{string::String, vec, vec::Vec};

/// What fills a spot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Supply {
    Expr(ExprId),
    Collected(Vec<ExprId>),
    Default(crate::limits::known::Known),
    /// A bare Bool flag.
    True(Span),
    /// A value typed already: the pipe's input or a dot's head.
    Typed(Type, Span),
}

impl Infer<'_> {
    /// The callee at `id`, with its spot list when it has one.
    pub(super) fn callee(&mut self, id: ExprId) -> Callee {
        let module = self.module;
        let span = self.span(id);
        let callee = match &module.expressions[id.0].kind {
            ExprKind::Name(_) => match self.resolved.expr(id) {
                Some(target) => self.target(target, span),
                None => Callee::plain(self.fresh()),
            },
            ExprKind::Instance { arguments, .. } => {
                let mut callee = match self.resolved.expr(id) {
                    Some(target) => self.target(target, span),
                    None => Callee::plain(self.fresh()),
                };
                let mut written = Vec::new();
                for (parameter, argument) in
                    callee.parameters.iter().zip(arguments)
                {
                    let ty = self.lower_here(*argument);
                    if let Type::Var(var) = parameter {
                        written.push((*var, ty.clone()));
                    }
                    self.unify(parameter, &ty, span, span);
                }
                callee.ty = crate::types::replace(&callee.ty, &written);
                for slot in callee.spots.iter_mut().flatten() {
                    slot.ty = crate::types::replace(&slot.ty, &written);
                }
                if let Some((target, _)) = &mut callee.conversion {
                    *target = crate::types::replace(target, &written);
                }
                callee
            }
            ExprKind::Field { value, field } => {
                let head = self.expr(*value, None);
                self.members(head, &[super::last(field)], span)
            }
            _ => return Callee::plain(self.expr(id, None)),
        };
        self.record_callee(&callee, span);
        self.state.types[id.0] = Some(callee.whole());
        callee
    }

    /// What a resolved name means as a value.
    pub(super) fn target(&mut self, target: &Target, span: Span) -> Callee {
        match target {
            Target::Local { site } => self.local(*site, span),
            Target::Value {
                origin: Origin::This,
                name,
            } => match self.item_index(&Key::Binding(name.clone())) {
                Some(index) => self.item_callee(index, span),
                // A method of a trait this module declares: 06's.
                None => Callee::plain(self.fresh()),
            },
            Target::Value {
                origin: Origin::File(stem),
                name,
            } => self.file_value(stem, name, span),
            // A method of a trait an accepted line declares: 06's.
            Target::Value {
                origin: Origin::Line(_),
                ..
            } => Callee::plain(self.fresh()),
            Target::Value { origin, name } => {
                self.declared_value(origin, name, span)
            }
            Target::Constructor { origin, ty, name } => {
                let callee = self.constructor(origin, ty, name);
                if let Some(field) = callee.sealed.clone() {
                    self.error(span, Reason::NoConstructor { field });
                }
                callee
            }
            Target::SelfType => self.self_constructor(),
            Target::Members { head, members } => {
                let root = matches!(**head, Target::TypeMember { .. });
                let head = self.target(head, span);
                if let (true, Some(selected)) = (root, &head.selected) {
                    self.state.roots.insert(
                        (span.start, span.end),
                        crate::typed::Root {
                            selected: selected.clone(),
                            meanings: head.meanings.clone(),
                        },
                    );
                }
                let value = self.apply_callee(head, &[], None, span, None);
                self.members(value, members, span)
            }
            Target::TypeMember { ty, member } => {
                self.type_member(ty, member, span)
            }
            Target::Word => Callee::plain(prelude("String")),
            Target::Type { origin, name } => {
                let limited =
                    self.limited(Type::named(origin.clone(), name, Vec::new()));
                let from = super::meaning(&limited).clone();
                self.state.conversions.insert(
                    (span.start, span.end),
                    crate::typed::Conversion {
                        mode: crate::ir::ConversionMode::Check,
                        from: from.clone(),
                        parameter: from.clone(),
                        to: limited.clone(),
                        result: limited.clone(),
                    },
                );
                Callee::plain(Type::function(from, limited))
            }
            Target::Module(_) | Target::Variable(_) => {
                self.error(span, Reason::NotAFunction);
                Callee::plain(self.fresh())
            }
        }
    }

    /// A binding's type with the spot list of its arm heads.
    pub(super) fn with_heads(
        &mut self,
        ty: Type,
        heads: Option<&[Head]>,
    ) -> Callee {
        let Some(heads) = heads else {
            return Callee::plain(ty);
        };
        let Some((parameters, result)) = self.peel(&ty, heads.len()) else {
            return Callee::plain(ty);
        };
        let slots = heads
            .iter()
            .zip(parameters)
            .map(|(head, ty)| Slot {
                place_kind: crate::typed::Place::Parameter(head.name.clone()),
                name: head.name.clone(),
                ty,
                default: head.default,
                value: head.value,
                site: head.site,
            })
            .collect::<Vec<_>>();
        let mut callee = Callee::plain(result);
        callee.self_slot = heads
            .iter()
            .position(|head| head.name.as_deref() == Some("self"));
        callee.spots = Some(slots);
        callee
    }

    /// The first `count` parameter types of a function type.
    pub(super) fn peel(
        &mut self,
        ty: &Type,
        count: usize,
    ) -> Option<(Vec<Type>, Type)> {
        let mut parameters = Vec::new();
        let mut current = ty.clone();
        for _ in 0..count {
            match self.apply(&current) {
                Type::Function { parameter, result } => {
                    parameters.push(*parameter);
                    current = *result;
                }
                Type::Var(_) => {
                    let (parameter, result) = (self.fresh(), self.fresh());
                    let function =
                        Type::function(parameter.clone(), result.clone());
                    self.hint(&current, &function);
                    parameters.push(parameter);
                    current = result;
                }
                _ => return None,
            }
        }
        Some((parameters, current))
    }

    pub(super) fn record_callee(&mut self, callee: &Callee, stage: Span) {
        self.state
            .callee_types
            .insert((stage.start, stage.end), callee.whole());
        self.state
            .meaning_arguments
            .insert((stage.start, stage.end), callee.meanings.clone());
        if let Some(selected) = &callee.selected {
            self.state
                .selected
                .insert((stage.start, stage.end), selected.clone());
        }
    }

    /// Apply a callee to written arguments and an optional piped input.
    pub(super) fn apply_callee(
        &mut self,
        mut callee: Callee,
        arguments: &[Argument],
        pipe: Option<(Type, Span)>,
        stage: Span,
        expected: Option<&Type>,
    ) -> Type {
        self.record_callee(&callee, stage);
        if callee.spots.is_none() {
            let mut ty = self.apply(&callee.ty);
            let mut slots = Vec::new();
            while let Type::Function { parameter, result } = ty {
                slots.push(Slot {
                    place_kind: crate::typed::Place::Parameter(None),
                    name: None,
                    ty: *parameter,
                    default: false,
                    value: None,
                    site: stage,
                });
                ty = *result;
            }
            let supplied = arguments.len() + usize::from(pipe.is_some());
            if !slots.is_empty()
                && (!matches!(ty, Type::Var(_))
                    || supplied <= slots.len()
                    || slots
                        .last()
                        .is_some_and(|s| is_prelude(&self.apply(&s.ty), "Vec")))
            {
                callee.spots = Some(slots);
                callee.ty = ty;
            }
        }
        if callee.spots.is_none() {
            let ty = self.curried(&callee.ty, arguments, pipe, stage);
            self.dots();
            return ty;
        }
        self.bind_spots(callee, arguments, pipe, stage, expected)
    }

    pub(super) fn written_call(
        &mut self,
        callee: Callee,
        arguments: &[Argument],
        style: crate::ast::CallStyle,
        pipe: Option<(Type, Span)>,
        stage: Span,
        expected: Option<&Type>,
    ) -> Type {
        use crate::ast::CallStyle;
        let alternative = match (style, arguments) {
            (
                CallStyle::Space,
                [Argument {
                    kind: ArgumentKind::Positional(id),
                    ..
                }],
            ) => match &self.module.expressions[id.0].kind {
                ExprKind::Tuple(parts) => Some((parts.clone(), false)),
                _ => None,
            },
            (CallStyle::Parenthesized, args) if args.len() >= 2 => args
                .iter()
                .map(|a| match a.kind {
                    ArgumentKind::Positional(id) => Some(id),
                    ArgumentKind::Flag { .. } => None,
                })
                .collect::<Option<Vec<_>>>()
                .map(|ids| (ids, true)),
            _ => None,
        };
        let snapshot = alternative
            .as_ref()
            .and_then(|_| callee.spots.clone())
            .map(|slots| {
                (slots, self.state.subst.clone(), callee.supplied.clone())
            });
        let error_start = self.state.errors.len();
        let result =
            self.apply_callee(callee, arguments, pipe.clone(), stage, expected);
        if let (Some((ids, tuple)), Some((slots, mut trial, supplied))) =
            (alternative, snapshot)
        {
            while trial.count() < self.state.subst.count() {
                trial.fresh();
            }
            let mut types: Vec<_> = ids
                .iter()
                .filter_map(|id| {
                    self.state.types[id.0].as_ref().map(|t| self.apply(t))
                })
                .collect();
            if types.len() != ids.len() {
                return result;
            }
            // Argument inference may have joined its variables to the written
            // spots. Freshen them together before trying different spots.
            let mut variables = Vec::new();
            for ty in &types {
                ty.variables(&mut variables);
            }
            let fresh: Vec<_> = variables
                .into_iter()
                .map(|v| (v, Type::Var(trial.fresh())))
                .collect();
            types = types
                .iter()
                .map(|t| crate::types::replace(t, &fresh))
                .collect();
            if tuple {
                types = vec![Type::Tuple(types)];
            }
            let mut filled = vec![false; slots.len()];
            let mut binds = true;
            if let Some((index, ty, _)) = supplied {
                filled[index] = true;
                binds &= trial.unify(&slots[index].ty, &ty).is_ok();
            }
            for ty in types {
                if let Some(index) =
                    slots.iter().enumerate().position(|(i, slot)| {
                        !filled[i]
                            && !is_prelude(&trial.apply(&slot.ty), "Bool")
                    })
                {
                    filled[index] = true;
                    binds &= trial.unify(&slots[index].ty, &ty).is_ok();
                } else {
                    binds = false;
                }
            }
            if let Some((ty, _)) = pipe {
                if let Some(index) = filled.iter().rposition(|v| !v) {
                    binds &= trial.unify(&slots[index].ty, &ty).is_ok();
                } else {
                    binds = false;
                }
            }
            if binds {
                if self.state.errors.len() == error_start {
                    self.state
                        .warnings
                        .push(crate::typed::Warning { span: stage, style });
                } else {
                    self.error(stage, Reason::CallReading { style });
                }
            }
        }
        result
    }

    fn bind_spots(
        &mut self,
        callee: Callee,
        arguments: &[Argument],
        pipe: Option<(Type, Span)>,
        stage: Span,
        expected: Option<&Type>,
    ) -> Type {
        let mark = self.state.obligations.len();
        let mut slots = callee.spots.unwrap_or_default();
        let result = callee.ty;
        let mut filled: Vec<Option<Supply>> = vec![None; slots.len()];
        let head_slot = callee.supplied.as_ref().map(|(index, _, _)| *index);
        if let Some((index, ty, span)) = callee.supplied {
            filled[index] = Some(Supply::Typed(ty, span));
        }
        let consumed = self.flags(&slots, arguments, &mut filled);
        self.positional(&slots, arguments, &consumed, &mut filled);
        if let Some((ty, span)) = pipe {
            match filled.iter().rposition(Option::is_none) {
                Some(index) => filled[index] = Some(Supply::Typed(ty, span)),
                None => self.error(stage, Reason::FilledSpot),
            }
        }
        if let Some(expected) = expected {
            let whole = slots
                .iter()
                .zip(&filled)
                .rev()
                .filter(|(slot, supply)| supply.is_none() && !slot.default)
                .fold(result.clone(), |ty, (slot, _)| {
                    Type::function(slot.ty.clone(), ty)
                });
            self.hint(expected, &whole);
        }
        self.supply(&slots, &mut filled, stage);
        if let Some((target, kind)) = &callee.conversion {
            let member =
                !matches!(kind.as_str(), "from" | "try_from" | "to" | "try");
            let source_slot =
                usize::from(member && callee.self_slot == Some(0));
            if let Some(Some(supply)) = filled.get(source_slot) {
                self.read_literal_at_meaning(
                    target,
                    kind,
                    supply,
                    &slots[source_slot].ty,
                );
                self.default_literals(mark, &[], &[]);
                let source = self.apply(&slots[source_slot].ty);
                let converted = self.convert(target, kind, &source, stage);
                self.unify(&result, &converted, stage, stage);
            } else if member {
                self.state.obligations.push(super::Owed::new(
                    crate::types::Obligation::Member {
                        head: target.clone(),
                        name: kind.clone(),
                        ty: Type::function(
                            slots[source_slot].ty.clone(),
                            result.clone(),
                        ),
                        dot: stage,
                    },
                    stage,
                ));
            }
        }
        let home = callee.home.as_ref().unwrap_or(&Origin::This);
        for (index, supply) in filled.iter().enumerate() {
            if supply.is_none() && slots[index].value.is_some() {
                self.declared_default(home, &slots, index, stage);
            }
        }
        self.dots();
        for (slot, supply) in slots.iter_mut().zip(&mut filled) {
            if supply.is_none()
                && slot.value.is_none()
                && matches!(slot.place_kind, crate::typed::Place::Field { .. })
            {
                if let Some(known) = self.type_default(&self.apply(&slot.ty)) {
                    slot.default = true;
                    *supply = Some(Supply::Default(known));
                }
            }
        }
        let supplied = slots
            .iter()
            .zip(&filled)
            .filter_map(|(slot, value)| {
                Some((slot.name.clone()?, value.clone()?))
            })
            .collect();
        self.state
            .supplied
            .insert((stage.start, stage.end), supplied);
        self.record_binding(
            &slots,
            &filled,
            head_slot,
            callee.home.as_ref(),
            stage,
        );
        // Step 4: a defaulted spot closes; the rest stay open in order.
        slots
            .iter()
            .zip(&filled)
            .rev()
            .filter(|(slot, supply)| supply.is_none() && !slot.default)
            .fold(result, |ty, (slot, _)| Type::function(slot.ty.clone(), ty))
    }

    pub(super) fn record_constructors(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let uses: Vec<_> = self
            .state
            .selected
            .iter()
            .filter_map(|(site, selected)| {
                let crate::typed::Selected::Constructor { ty, name } = selected
                else {
                    return None;
                };
                self.state.bound.get(site).map(|bound| {
                    (*site, self.apply(ty), name.clone(), bound.clone())
                })
            })
            .collect();
        for ((start, end), ty, label, mut bound) in uses {
            let Type::Named { name, .. } = &ty else {
                continue;
            };
            let span = Span { start, end };
            let callee = self.constructor_fields_kind(
                &name.origin,
                &name.name,
                &label,
                true,
                true,
            );
            self.unify(&callee.ty, &ty, span, span);
            let Some(slots) = callee.spots else { continue };
            let mut fields: Vec<_> = bound
                .spots
                .iter()
                .map(|(site, _)| {
                    slots
                        .iter()
                        .position(|slot| slot.site == *site)
                        .expect("constructor spot")
                })
                .collect();
            let saved = self.state.default_instances.clone();
            for (field, slot) in slots.iter().enumerate() {
                if fields.contains(&field) {
                    continue;
                }
                let value = if slot.value.is_some() {
                    self.declared_default(&name.origin, &slots, field, span);
                    self.state
                        .default_instances
                        .get(&(start, end, field))
                        .map_or_else(
                            || {
                                crate::typed::DefaultValue::Type(
                                    self.apply(&slot.ty),
                                )
                            },
                            |instance| {
                                crate::typed::DefaultValue::Expression(
                                    alloc::boxed::Box::new(instance.clone()),
                                )
                            },
                        )
                } else {
                    self.default_value(&slot.ty, span)
                };
                bound.defaults.push((bound.spots.len(), value));
                bound.spots.push((slot.site, self.apply(&slot.ty)));
                fields.push(field);
            }
            self.state.default_instances = saved;
            bound.defaults.sort_by_key(|(spot, _)| fields[*spot]);
            self.state.bound.insert((start, end), bound);
            self.state.constructor_fields.insert((start, end), fields);
        }
    }

    pub(super) fn record_defaults(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let uses: Vec<_> = self
            .state
            .selected
            .iter()
            .filter_map(|(site, selected)| {
                if let crate::typed::Selected::Derived {
                    kind: crate::ir::Derived::Default,
                    ty,
                } = selected
                {
                    Some((*site, self.apply(ty)))
                } else {
                    None
                }
            })
            .collect();
        for ((start, end), ty) in uses {
            let value = self.default_value(&ty, Span { start, end });
            self.state.derived_defaults.insert((start, end), value);
        }
    }

    pub(super) fn default_value(
        &mut self,
        ty: &Type,
        span: Span,
    ) -> crate::typed::DefaultValue {
        let ty = self.apply(ty);
        if let Type::Limited { meaning, .. } = &ty {
            return self.default_value(meaning, span);
        }
        if let Type::Named { name, .. } = &ty {
            if self.claim_member_count(&ty, &prelude("Default"), "default") > 0
            {
                let callee = self.type_member(
                    &Target::Type {
                        origin: name.origin.clone(),
                        name: name.name.clone(),
                    },
                    "default",
                    span,
                );
                self.unify(&callee.ty, &ty, span, span);
                if let Some(selected) = callee.selected {
                    return crate::typed::DefaultValue::Prepared {
                        selected: alloc::boxed::Box::new(
                            selected.applied(&self.state.subst),
                        ),
                        meanings: callee
                            .meanings
                            .iter()
                            .map(|ty| self.apply(ty))
                            .collect(),
                        ty: self.apply(&callee.ty),
                    };
                }
            }
        }
        match &ty {
            Type::Named { name, .. } if name.origin != Origin::Prelude => {
                let callee = self.constructor_fields_kind(
                    &name.origin,
                    &name.name,
                    &name.name,
                    true,
                    true,
                );
                self.unify(&callee.ty, &ty, span, span);
                let slots = callee.spots.unwrap_or_default();
                let saved = self.state.default_instances.clone();
                let mut fields = Vec::new();
                for (index, slot) in slots.iter().enumerate() {
                    let value = if slot.value.is_some() {
                        self.declared_default(
                            &name.origin,
                            &slots,
                            index,
                            span,
                        );
                        self.state
                            .default_instances
                            .get(&(span.start, span.end, index))
                            .map_or_else(
                                || {
                                    crate::typed::DefaultValue::Type(
                                        self.apply(&slot.ty),
                                    )
                                },
                                |instance| {
                                    crate::typed::DefaultValue::Expression(
                                        alloc::boxed::Box::new(
                                            instance.clone(),
                                        ),
                                    )
                                },
                            )
                    } else {
                        self.default_value(&slot.ty, span)
                    };
                    fields.push((slot.site, self.apply(&slot.ty), value));
                }
                self.state.default_instances = saved;
                return crate::typed::DefaultValue::Record { ty, fields };
            }
            Type::Array { element, .. } => {
                let element =
                    alloc::boxed::Box::new(self.default_value(element, span));
                return crate::typed::DefaultValue::Array { ty, element };
            }
            _ => (),
        }
        crate::typed::DefaultValue::Type(ty)
    }

    fn record_binding(
        &mut self,
        slots: &[Slot],
        filled: &[Option<Supply>],
        head: Option<usize>,
        home: Option<&Origin>,
        stage: Span,
    ) {
        use crate::typed::{Bound, DefaultValue, Supply as Source};
        let mut bound = Bound {
            spots: slots
                .iter()
                .map(|slot| (slot.site, slot.ty.clone()))
                .collect(),
            ..Bound::default()
        };
        let mut supplied = Vec::new();
        for (index, (slot, value)) in slots.iter().zip(filled).enumerate() {
            let source = match value {
                Some(Supply::Expr(id)) => {
                    Some((self.span(*id), Source::Expression(*id)))
                }
                Some(Supply::True(span)) => Some((*span, Source::Flag(*span))),
                Some(Supply::Typed(_, span)) => Some((
                    *span,
                    if head == Some(index) {
                        Source::Head(*span)
                    } else {
                        Source::Pipe(*span)
                    },
                )),
                Some(Supply::Collected(ids)) => {
                    supplied.extend(ids.iter().map(|id| {
                        (self.span(*id).start, index, Source::Element(*id))
                    }));
                    continue;
                }
                None | Some(Supply::Default(_)) => None,
            };
            if let Some((span, source)) = source {
                supplied.push((span.start, index, source));
            } else if slot.default {
                let default = slot.value.map_or_else(
                    || self.default_value(&slot.ty, stage),
                    |value| DefaultValue::Declared {
                        origin: home.cloned().unwrap_or(Origin::This),
                        value,
                    },
                );
                bound.defaults.push((index, default));
            } else {
                bound.open.push(index);
            }
        }
        supplied.sort_by_key(|(start, _, _)| *start);
        bound.supplies = supplied
            .into_iter()
            .map(|(_, index, source)| (index, source))
            .collect();
        self.state.bound.insert((stage.start, stage.end), bound);
    }

    /// Step 1: named arguments fill their spots.
    fn flags(
        &mut self,
        slots: &[Slot],
        arguments: &[Argument],
        filled: &mut [Option<Supply>],
    ) -> Vec<bool> {
        let mut consumed = vec![false; arguments.len()];
        for (index, argument) in arguments.iter().enumerate() {
            let ArgumentKind::Flag { name, value } = &argument.kind else {
                continue;
            };
            let slot = self.flag_slot(slots, name);
            if slot.is_none() && name.chars().count() > 1 {
                let chain: Option<Vec<_>> = name
                    .chars()
                    .map(|letter| {
                        slots
                            .iter()
                            .enumerate()
                            .find(|(_, slot)| {
                                self.is_bool(&slot.ty)
                                    && slot
                                        .name
                                        .as_ref()
                                        .is_some_and(|n| n.starts_with(letter))
                            })
                            .or_else(|| {
                                slots.iter().enumerate().find(|(_, slot)| {
                                    slot.name
                                        .as_ref()
                                        .is_some_and(|n| n.starts_with(letter))
                                })
                            })
                            .map(|(index, _)| index)
                    })
                    .collect();
                let reason = chain.as_ref().map_or_else(
                    || Some(Reason::UnknownSpot { name: name.clone() }),
                    |chain| {
                        (value.is_some()
                            || chain
                                .iter()
                                .any(|&i| !self.is_bool(&slots[i].ty)))
                        .then_some(Reason::NonBoolChaining)
                    },
                );
                if let Some(reason) = reason {
                    self.error(argument.span, reason);
                    if let Some(value) = value {
                        self.expr(*value, None);
                    }
                } else {
                    for slot in chain.unwrap_or_default() {
                        if filled[slot].is_some() {
                            self.error(argument.span, Reason::FilledSpot);
                        } else {
                            filled[slot] = Some(Supply::True(argument.span));
                        }
                    }
                }
                continue;
            }
            let supply = match (slot, value) {
                (None, _) => None,
                (Some(_), Some(value)) => Some(Supply::Expr(*value)),
                (Some(slot), None) if self.is_bool(&slots[slot].ty) => {
                    Some(Supply::True(argument.span))
                }
                (Some(_), None) => match arguments.get(index + 1) {
                    Some(Argument {
                        kind: ArgumentKind::Positional(next),
                        ..
                    }) => {
                        consumed[index + 1] = true;
                        Some(Supply::Expr(*next))
                    }
                    _ => None,
                },
            };
            let (Some(slot), Some(supply)) = (slot, supply) else {
                let name = name.clone();
                self.error(argument.span, Reason::UnknownSpot { name });
                if let Some(value) = value {
                    self.expr(*value, None);
                }
                continue;
            };
            if filled[slot].is_some() {
                self.error(argument.span, Reason::FilledSpot);
            } else {
                filled[slot] = Some(supply);
            }
        }
        consumed
    }

    /// A flag's spot: its name, or the one Bool spot whose name starts
    /// with a one-letter flag.
    fn flag_slot(&self, slots: &[Slot], flag: &str) -> Option<usize> {
        if let Some(index) =
            slots.iter().position(|s| s.name.as_deref() == Some(flag))
        {
            return Some(index);
        }
        let mut letters = flag.chars();
        let (Some(letter), None) = (letters.next(), letters.next()) else {
            return None;
        };
        let matching: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| {
                self.is_bool(&slot.ty)
                    && slot
                        .name
                        .as_ref()
                        .is_some_and(|name| name.starts_with(letter))
            })
            .map(|(index, _)| index)
            .collect();
        match matching.as_slice() {
            [index] => Some(*index),
            _ => None,
        }
    }

    /// Step 2: positional operands fill unfilled non-Bool spots in order.
    fn positional(
        &mut self,
        slots: &[Slot],
        arguments: &[Argument],
        consumed: &[bool],
        filled: &mut [Option<Supply>],
    ) {
        let trailing = slots
            .iter()
            .rposition(|slot| !self.is_bool(&slot.ty))
            .filter(|&i| {
                filled[i].is_none()
                    && is_prelude(&self.apply(&slots[i].ty), "Vec")
            });
        let mut next = 0;
        for (index, argument) in arguments.iter().enumerate() {
            let ArgumentKind::Positional(value) = &argument.kind else {
                continue;
            };
            if consumed[index] {
                continue;
            }
            while next < slots.len()
                && ((filled[next].is_some() && trailing != Some(next))
                    || self.is_bool(&slots[next].ty))
            {
                next += 1;
            }
            if next < slots.len() {
                if trailing == Some(next) {
                    if let Some(Supply::Collected(values)) = &mut filled[next] {
                        values.push(*value);
                    } else {
                        filled[next] = Some(Supply::Collected(vec![*value]));
                    }
                } else {
                    filled[next] = Some(Supply::Expr(*value));
                }
            } else {
                self.error(argument.span, Reason::ExtraArgument);
                self.expr(*value, None);
            }
        }
    }

    fn enter(
        &mut self,
        target: &Type,
        actual: &Type,
        span: Span,
        other: Span,
        place_kind: crate::typed::Place,
    ) {
        let target = self.apply(target);
        if let Some(index) = self.expression_ids.get(&span).copied() {
            if matches!(self.module.expressions[index].kind, ExprKind::List(_))
                && !matches!(
                    super::meaning(&target),
                    Type::Array { .. } | Type::Var(_) | Type::Rigid(_)
                )
            {
                let expected = Expected::new(target, span).at(place_kind);
                self.expr(ExprId(index), Some(&expected));
                return;
            }
        }
        if crate::limits::has_limits(&target)
            || crate::limits::has_limits(&self.apply(actual))
        {
            if let Some(index) = self.expression_ids.get(&span).copied() {
                self.state.entries.push((
                    ExprId(index),
                    target.clone(),
                    span,
                    place_kind,
                ));
            }
        }
        let before = self.numeric_var(actual).then(|| self.state.clone());
        let subst = self.state.subst.clone();
        let errors = self.state.errors.len();
        self.unify(&target, actual, span, other);
        if before.is_some() {
            self.dots();
        }
        if self.state.errors[errors..]
            .iter()
            .any(|e| matches!(e.reason, Reason::OutOfRange))
        {
            return;
        }
        if self.state.errors.len() != errors {
            let failed = self.state.clone();
            if let Some(before) = before {
                self.state = before;
            } else {
                self.state.subst = subst;
                self.state.errors.truncate(errors);
            }
            self.default_entering(actual);
            let source = self.apply(actual);
            if let Some(row) = self.cast_pair(&target, &source, span, other) {
                if let Some(index) = self.expression_ids.get(&span).copied() {
                    self.record_cast(ExprId(index), row, source, target);
                }
            } else if !self.state.errors[errors..]
                .iter()
                .any(|e| matches!(e.reason, Reason::OutcomeText { .. }))
            {
                self.state = failed;
            }
        }
    }

    /// Type every supply against its spot: typed values first, then
    /// operands, arm lists and selectors last so that their parameters
    /// meet known types.
    fn supply(
        &mut self,
        slots: &[Slot],
        filled: &mut [Option<Supply>],
        stage: Span,
    ) {
        for (slot, supply) in slots.iter().zip(filled.iter()) {
            match supply {
                Some(Supply::Typed(ty, span)) => {
                    self.argument_span(&slot.ty, *span);
                    self.enter(
                        &slot.ty,
                        ty,
                        *span,
                        stage,
                        slot.place_kind.clone(),
                    );
                }
                Some(Supply::True(span)) => {
                    self.unify(&slot.ty, &prelude("Bool"), *span, stage);
                    let ty = self.apply(&slot.ty);
                    if crate::limits::has_limits(&ty) {
                        let (errors, checks) = crate::limits::flag_entry(
                            &self.limit_inputs(),
                            &ty,
                            (*span, slot.place_kind.clone()),
                        );
                        self.state.errors.extend(errors);
                        self.state.checks.extend(checks);
                    }
                }
                _ => {}
            }
        }
        let module = self.module;
        let late = |id: &ExprId| {
            matches!(
                module.expressions[id.0].kind,
                ExprKind::Arms(_) | ExprKind::Selector(_)
            )
        };
        for pass in [false, true] {
            for (slot, supply) in slots.iter().zip(filled.iter_mut()) {
                if !pass {
                    if let Some(Supply::Collected(ids)) = supply {
                        if ids.len() == 1 {
                            let before = self.state.clone();
                            let id = ids[0];
                            self.expr(
                                id,
                                Some(&Expected {
                                    place_kind: slot.place_kind.clone(),
                                    ty: slot.ty.clone(),
                                    span: self.span(id),
                                    spot: true,
                                    cast: true,
                                }),
                            );
                            self.dots();
                            if self.state.errors.len() == before.errors.len() {
                                *supply = Some(Supply::Expr(id));
                                continue;
                            }
                            self.state = before;
                        }
                        let Type::Named { arguments, .. } =
                            self.apply(&slot.ty)
                        else {
                            unreachable!()
                        };
                        for id in ids {
                            self.expr(
                                *id,
                                Some(&Expected {
                                    place_kind: crate::typed::Place::Element,
                                    ty: arguments[0].clone(),
                                    span: self.span(*id),
                                    spot: true,
                                    cast: true,
                                }),
                            );
                        }
                        continue;
                    }
                }
                if let Some(Supply::Expr(id)) = supply {
                    if late(id) == pass {
                        let expected = Expected {
                            place_kind: slot.place_kind.clone(),
                            ty: slot.ty.clone(),
                            span: self.span(*id),
                            spot: true,
                            cast: true,
                        };
                        self.expr(*id, Some(&expected));
                    }
                }
            }
        }
    }

    /// Positional, curried application of a callee with no spot list.
    fn curried(
        &mut self,
        ty: &Type,
        arguments: &[Argument],
        pipe: Option<(Type, Span)>,
        stage: Span,
    ) -> Type {
        let mut operands = Vec::new();
        for argument in arguments {
            match &argument.kind {
                ArgumentKind::Positional(value) => operands.push(*value),
                ArgumentKind::Flag { name, value } => {
                    let name = name.clone();
                    self.error(argument.span, Reason::UnknownSpot { name });
                    if let Some(value) = value {
                        self.expr(*value, None);
                    }
                }
            }
        }
        let supplied = operands.len() + usize::from(pipe.is_some());
        let mut count = 0;
        let mut tail = self.apply(ty);
        while let Type::Function { result, .. } = tail {
            count += 1;
            tail = *result;
        }
        let count = count.max(supplied);
        let Some((parameters, result)) = self.peel(ty, count) else {
            self.error(stage, Reason::NotAFunction);
            for operand in operands {
                self.expr(operand, None);
            }
            return self.fresh();
        };
        let mut bound = crate::typed::Bound {
            spots: parameters.iter().map(|ty| (stage, ty.clone())).collect(),
            ..crate::typed::Bound::default()
        };
        if let Some((_, span)) = &pipe {
            bound
                .supplies
                .push((count - 1, crate::typed::Supply::Pipe(*span)));
        }
        bound.supplies.extend(
            operands.iter().enumerate().map(|(index, id)| {
                (index, crate::typed::Supply::Expression(*id))
            }),
        );
        let end = count - usize::from(pipe.is_some());
        bound.open.extend(operands.len()..end);
        let result = parameters[operands.len()..end]
            .iter()
            .rev()
            .fold(result, |result, parameter| {
                Type::function(parameter.clone(), result)
            });
        self.state.bound.insert((stage.start, stage.end), bound);
        // An overload can be chosen only after all this call's operands
        // have entered their spots (including a selector's argument).
        let solving = core::mem::replace(&mut self.state.solving_dots, true);
        if let Some((ty, span)) = pipe {
            self.argument_span(&parameters[count - 1], span);
            self.enter(
                &parameters[count - 1],
                &ty,
                span,
                stage,
                crate::typed::Place::Parameter(None),
            );
        }
        for (operand, parameter) in operands.into_iter().zip(parameters) {
            let expected = Expected {
                place_kind: crate::typed::Place::Parameter(None),
                ty: parameter,
                span: self.span(operand),
                spot: true,
                cast: true,
            };
            self.expr(operand, Some(&expected));
        }
        self.state.solving_dots = solving;
        result
    }

    /// `input | stage`: the input fills the stage's last open spot.
    pub(super) fn stage(
        &mut self,
        id: ExprId,
        input: Type,
        input_span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let module = self.module;
        let span = self.span(id);
        let pipe = Some((input.clone(), input_span));
        let result = match &module.expressions[id.0].kind {
            ExprKind::Call {
                callee,
                arguments,
                style,
            } => {
                let callee = self.callee(*callee);
                self.written_call(
                    callee, arguments, *style, pipe, span, expected,
                )
            }
            ExprKind::Name(_)
            | ExprKind::Instance { .. }
            | ExprKind::Field { .. } => {
                let callee = self.callee(id);
                self.apply_callee(callee, &[], pipe, span, expected)
            }
            _ => {
                let result = self.fresh();
                let function = Type::function(input, result.clone());
                self.expr(id, Some(&Expected::new(function, input_span)));
                return result;
            }
        };
        // The stage's call lists the piped value among its supplies, so
        // its node is the stage's result, not a function of the input.
        self.state.types[id.0] = Some(result.clone());
        result
    }

    pub(super) fn is_bool(&self, ty: &Type) -> bool {
        is_prelude(super::meaning(&self.apply(ty)), "Bool")
    }

    /// Step 4 for a callee declared elsewhere: its spot default, typed
    /// in its declaring module against the spot. Its errors are the
    /// declaration's; what it solves is this application's.
    fn declared_default(
        &mut self,
        home: &Origin,
        slots: &[Slot],
        index: usize,
        stage: Span,
    ) {
        let (_, valid) = self.declared_expression(
            home,
            &slots[index],
            &slots[..index],
            stage,
            true,
        );
        if valid {
            if let Some(value) = slots[index].value {
                if let Some(recorded) = self
                    .file_cache
                    .expressions
                    .borrow()
                    .get(&(home.clone(), value.0))
                    .cloned()
                {
                    self.state
                        .default_instances
                        .insert((stage.start, stage.end, index), recorded);
                }
            }
        }
        if !valid {
            self.error(
                stage,
                Reason::Default {
                    name: slots[index].name.clone().unwrap_or_default(),
                },
            );
        }
    }

    pub(super) fn declared_expression(
        &mut self,
        home: &Origin,
        slot: &Slot,
        earlier: &[Slot],
        stage: Span,
        is_default: bool,
    ) -> (Scheme, bool) {
        self.declared_context(home, slot, earlier, stage, is_default, None)
    }

    /// A numeric literal entering `to<T>` or `try<T>` on a limited
    /// numeric type is read at the meaning it reaches: exact there or a
    /// check error, and the gate then sees its value.
    fn read_literal_at_meaning(
        &mut self,
        target: &Type,
        kind: &str,
        supply: &Supply,
        slot: &Type,
    ) {
        let applied = self.apply(target);
        let literal = match supply {
            Supply::Expr(id) => super::casts::grouped_literal(self.module, *id),
            _ => None,
        };
        if matches!(kind, "to" | "try")
            && matches!(applied, Type::Limited { .. })
            && super::conversions::numeric(super::meaning(&applied))
            && matches!(
                literal,
                Some(
                    crate::ast::Literal::Int { .. }
                        | crate::ast::Literal::Float(_)
                )
            )
        {
            let meaning = super::meaning(&applied).clone();
            self.hint(slot, &meaning);
        }
    }

    pub(super) fn declared_context(
        &mut self,
        home: &Origin,
        slot: &Slot,
        earlier: &[Slot],
        stage: Span,
        is_default: bool,
        owner: Option<&Type>,
    ) -> (Scheme, bool) {
        // Recursive defaults use their signatures. Evaluation decides termination.
        let key = (home.clone(), slot.site);
        if self.state.defaults.contains(&key) {
            return (Scheme::mono(slot.ty.clone()), true);
        }
        let (Some(module), Some(value)) = (self.owner(home), slot.value) else {
            return (Scheme::mono(slot.ty.clone()), false);
        };
        // A limited type's test is closed over its owner and shares no
        // state with the line, so a test recorded earlier, in this
        // session or this unit, is read back rather than typed again.
        if *home != Origin::This
            && owner.is_some()
            && self
                .file_cache
                .expression(&(home.clone(), value.0))
                .is_some()
        {
            return (Scheme::mono(slot.ty.clone()), true);
        }
        let earlier: Vec<_> = earlier
            .iter()
            .filter_map(|slot| Some((slot.name.clone()?, slot.site, &slot.ty)))
            .collect();
        let names: Vec<_> = earlier
            .iter()
            .map(|(name, site, _)| (name.clone(), *site))
            .collect();
        let (resolved, resolve_errors) =
            self.default_resolution(home, module, value, &names, owner);
        let subst = core::mem::take(&mut self.state.subst);
        let mut inner = Infer {
            module,
            resolved: &resolved,
            declarations: self.declarations,
            files: self.files,
            frame: self.frame,
            visiting: self.visiting.clone(),
            file_cache: self.file_cache,
            indexes: self.indexes.clone(),
            expression_ids: super::expression_ids(module),
            weak: self.weak,
            state: State::new(subst, module.expressions.len()),
            items: Vec::new(),
            groups: Vec::new(),
            self_type: None,
        };
        if *home == Origin::This {
            inner.state.locals.clone_from(&self.state.locals);
            inner.state.status.clone_from(&self.state.status);
            inner.items.clone_from(&self.items);
            inner.groups.clone_from(&self.groups);
            inner.self_type.clone_from(&self.self_type);
        }
        if let Some(owner) = owner {
            inner.assume_test_owner(owner, slot.site);
        }
        inner.state.defaults.clone_from(&self.state.defaults);
        inner.state.defaults.push(key);
        for (_, site, ty) in earlier {
            let local = Local {
                scheme: Scheme::mono(ty.clone()),
                heads: None,
            };
            inner.state.locals.insert((site.start, site.end), local);
        }
        let (scheme, mut typed, errors) =
            inner.check_declared_body(value, slot, stage, is_default);
        if !is_default {
            typed
                .schemes
                .insert((slot.site.start, slot.site.end), scheme.clone());
            typed.hidden = inner.record_hidden(&typed.schemes);
        }
        let valid = inner.state.errors.is_empty()
            && resolve_errors.is_empty()
            && errors.is_empty();
        let recorded = valid.then(|| typed.clone());
        for mut check in typed.checks.into_iter().filter(|_| owner.is_none()) {
            check.place = stage;
            check.frequency = crate::limits::frequency(self.module, stage);
            check.place_origin = Origin::This;
            if check.limit_origin == Origin::This {
                check.limit_origin = home.clone();
            }
            check.target = check.target.map(|ty| ty.rehome(home));
            check.known.rehome(home);
            self.state.checks.push(check);
        }
        self.state.subst = inner.state.subst;
        for mut owed in inner
            .state
            .obligations
            .into_iter()
            .filter(|_| owner.is_none())
        {
            owed.span = stage;
            self.state.obligations.push(owed);
        }
        self.record_declared(home, value, resolved, recorded);
        (scheme, valid)
    }
    fn default_resolution(
        &self,
        home: &Origin,
        module: &crate::ast::Module,
        value: ExprId,
        names: &[(String, Span)],
        owner: Option<&Type>,
    ) -> (crate::resolve::Resolved, Vec<crate::resolve::Error>) {
        let owner_name = owner.and_then(|ty| {
            if let Type::Named { name, .. } = ty {
                Some(name.name.as_str())
            } else {
                None
            }
        });
        if *home == Origin::This {
            (self.resolved.clone(), Vec::new())
        } else {
            crate::resolve::resolve_declared_expression(
                module,
                self.declarations,
                home,
                value,
                names,
                owner_name,
            )
        }
    }

    fn check_declared_body(
        &mut self,
        value: ExprId,
        slot: &Slot,
        stage: Span,
        is_default: bool,
    ) -> (Scheme, crate::typed::Typed, Vec<super::Error>) {
        let expected = Expected {
            place_kind: slot.place_kind.clone(),
            ty: slot.ty.clone(),
            span: stage,
            spot: is_default,
            cast: true,
        };
        let mark = self.mark();
        self.expr(value, Some(&expected));
        self.dots();
        let scheme = if is_default {
            self.default_literals(0, &[], &[]);
            Scheme::mono(self.apply(&slot.ty))
        } else {
            self.generalize(
                &[super::bind::Part {
                    ty: slot.ty.clone(),
                    site: slot.site,
                    annotated: false,
                }],
                mark,
                &[],
            )
            .remove(0)
        };
        self.finish_dots();
        self.settle();
        let (typed, errors) = self.default_checks(value);
        (scheme, typed, errors)
    }

    fn record_declared(
        &self,
        home: &Origin,
        value: ExprId,
        resolved: crate::resolve::Resolved,
        recorded: Option<crate::typed::Typed>,
    ) {
        if let Some(typed) = recorded {
            self.file_cache.record(
                (home.clone(), value.0),
                crate::typed::DeclaredExpression {
                    origin: home.clone(),
                    value,
                    resolved: alloc::rc::Rc::new(resolved),
                    typed: alloc::rc::Rc::new(typed),
                },
            );
        }
    }
    fn default_checks(
        &mut self,
        value: ExprId,
    ) -> (crate::typed::Typed, Vec<super::Error>) {
        self.record_operations();
        let shapes = self.record_shapes();
        let dispatch = self.record_dispatch(&shapes);
        let mut typed = crate::typed::Typed {
            dispatch,
            shapes,
            rebuilds: self.state.rebuilds.clone(),
            modes: self.state.modes.clone(),
            casts: self.state.casts.clone(),
            expressions: self
                .state
                .types
                .iter()
                .map(|ty| ty.as_ref().map(|ty| self.apply(ty)))
                .collect(),
            supplied: self.state.supplied.clone(),
            checks: self
                .state
                .checks
                .iter()
                .cloned()
                .map(|mut check| {
                    check.target = check.target.map(|ty| self.apply(&ty));
                    check
                })
                .collect(),
            frozen: self
                .state
                .locals
                .iter()
                .map(|(site, local)| (*site, local.clone()))
                .collect(),
            ..self.record_decisions()
        };
        let entries: Vec<_> = self
            .state
            .entries
            .iter()
            .map(|(id, ty, span, kind)| {
                (*id, self.apply(ty), *span, kind.clone())
            })
            .collect();
        let errors = crate::limits::check(
            &self.limit_inputs(),
            &entries,
            Some(value),
            &mut typed,
        );
        (typed, errors)
    }
}

impl Infer<'_> {
    pub(super) fn argument_span(&mut self, parameter: &Type, span: Span) {
        let parameter = self.apply(parameter);
        if !matches!(parameter, Type::Var(_)) {
            return;
        }
        for owed in &mut self.state.obligations {
            if !owed.solved
                && owed.literal.is_some()
                && self.state.subst.apply(owed.obligation.subject())
                    == parameter
            {
                owed.span = span;
            }
            if !owed.solved
                && matches!(
                    owed.obligation,
                    crate::types::Obligation::Member { .. }
                )
                && self.state.subst.apply(owed.obligation.subject())
                    == parameter
            {
                owed.span = span;
                match &mut owed.obligation {
                    crate::types::Obligation::Member { dot, .. }
                    | crate::types::Obligation::Claim { dot, .. } => {
                        *dot = span;
                    }
                }
            }
        }
    }
}

impl Infer<'_> {
    pub(super) fn record_fields(
        &mut self,
        id: ExprId,
        fields: &[crate::ast::FieldValue],
        ty: &Type,
        span: Span,
    ) {
        let Type::Named { name, .. } = super::meaning(ty) else {
            return;
        };
        let label = match self.resolved.expr(id) {
            Some(Target::Constructor { name, .. }) => name.clone(),
            _ => name.name.clone(),
        };
        let callee =
            self.constructor_kind(&name.origin, &name.name, &label, true);
        self.unify(&callee.ty, ty, span, span);
        let Some(slots) = &callee.spots else { return };
        let mut bound = crate::typed::Bound {
            spots: slots
                .iter()
                .map(|slot| (slot.site, slot.ty.clone()))
                .collect(),
            ..Default::default()
        };
        for field in fields {
            if let Some(index) = slots.iter().position(|slot| {
                slot.name.as_deref() == Some(super::last(&field.name).as_str())
            }) {
                bound.supplies.push((
                    index,
                    crate::typed::Supply::Expression(field.value),
                ));
            }
        }
        for (index, slot) in slots.iter().enumerate() {
            if bound.supplies.iter().any(|(spot, _)| *spot == index)
                || !slot.default
            {
                continue;
            }
            let value = match slot.value {
                Some(value) => {
                    self.declared_default(&name.origin, slots, index, span);
                    crate::typed::DefaultValue::Declared {
                        origin: name.origin.clone(),
                        value,
                    }
                }
                None => self.default_value(&slot.ty, span),
            };
            bound.defaults.push((index, value));
        }
        self.record_callee(&callee, span);
        self.state.bound.insert((span.start, span.end), bound);
    }
}
