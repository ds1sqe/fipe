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

use alloc::{rc::Rc, string::String, vec, vec::Vec};

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
    /// A spot left to its declared default, in the callee's home: what
    /// the default's expression says, read with the spots before it.
    Declared(Origin, ExprId),
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
                callee.owner = callee
                    .owner
                    .as_ref()
                    .map(|owner| crate::types::replace(owner, &written));
                for meaning in &mut callee.meanings {
                    *meaning = crate::types::replace(meaning, &written);
                }
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
                self.members(head, &field.segments, span)
            }
            _ => return Callee::plain(self.expr(id, None)),
        };
        self.record_callee(&callee, span);
        self.set_type(id.0, callee.whole());
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
                    self.insert(
                        |s| &mut s.roots,
                        (span.start, span.end),
                        super::SavedRoot {
                            root: crate::typed::Root {
                                selected: selected.clone(),
                                meanings: head.meanings.clone(),
                            },
                            callee: head.whole(),
                            owner: head.owner.clone(),
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
                self.insert(
                    |s| &mut s.conversions,
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

    pub(super) fn overlay_declared_signature(
        &mut self,
        callee: &mut Callee,
        module: &crate::ast::Module,
        annotation: crate::ast::TypeId,
        home: &Origin,
        definition: (&str, Span),
        span: Span,
    ) {
        use crate::ast::TypeKind;
        let mut current = annotation;
        if let TypeKind::Qualified { ty, .. } = &module.types[current.0].kind {
            current = *ty;
        }
        let mut parameters = Vec::new();
        while let TypeKind::Function { parameter, result } =
            &module.types[current.0].kind
        {
            parameters.push(*parameter);
            current = *result;
        }
        if parameters.is_empty() {
            return;
        }
        // Checked written heads own the raw body boundary. Remaining arrows
        // describe its returned value, not additional arguments to this body.
        if let Some(spots) = &callee.spots {
            parameters.truncate(spots.len());
        }
        let prior = callee.spots.as_deref().unwrap_or(&[]);
        let mut actual = callee.whole();
        let mut slots = Vec::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let Type::Function {
                parameter: ty,
                result,
            } = self.apply(&actual)
            else {
                self.error(
                    span,
                    Reason::DeclarationValue {
                        name: definition.0.into(),
                        origin: home.clone(),
                        declaration: definition.1,
                    },
                );
                return;
            };
            let mut slot = prior.get(index).cloned().unwrap_or_else(|| Slot {
                place_kind: crate::typed::Place::Parameter(None),
                name: None,
                ty: (*ty).clone(),
                default: false,
                value: None,
                site: module.types[parameter.0].span,
            });
            slot.ty = *ty;
            if let TypeKind::Labeled { name, default, .. } =
                &module.types[parameter.0].kind
            {
                slot.name = Some(super::last(name));
                slot.place_kind =
                    crate::typed::Place::Parameter(slot.name.clone());
                slot.default = default.is_some();
                slot.value = *default;
                slot.site = name.span;
            }
            slots.push(slot);
            actual = *result;
        }
        callee.self_slot = callee.self_slot.or_else(|| {
            slots
                .iter()
                .position(|slot| slot.name.as_deref() == Some("self"))
        });
        callee.ty = actual;
        callee.spots = Some(slots);
        callee.home = Some(home.clone());
    }

    pub(super) fn declared_item_annotation(
        &self,
        site: Span,
    ) -> Option<crate::ast::TypeId> {
        self.module.entries.iter().find_map(|entry| match entry {
            crate::ast::Entry::Line(id) => {
                let crate::ast::ExprKind::Bind {
                    pattern,
                    annotation,
                    ..
                } = &self.module.expressions[id.0].kind
                else {
                    return None;
                };
                (self.module.patterns[pattern.0].span == site)
                    .then_some(*annotation)
                    .flatten()
            }
            crate::ast::Entry::Item(item) => {
                let crate::ast::ItemKind::Impl { members, .. } = &item.kind
                else {
                    return None;
                };
                members.iter().find_map(|member| match member {
                    crate::ast::ImplMember::Named {
                        name, annotation, ..
                    } if name.span == site => *annotation,
                    _ => None,
                })
            }
        })
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
        let key = (stage.start, stage.end);
        self.set_contract_stage(key, callee.contract_binding.clone());
        self.insert(
            |s| &mut s.callee_types,
            (stage.start, stage.end),
            callee.whole(),
        );
        self.set_meaning_arguments(
            (stage.start, stage.end),
            callee.meanings.clone(),
        );
        if let Some(selected) = &callee.selected {
            self.select((stage.start, stage.end), selected.clone());
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
        let contract_binding = callee.contract_binding.clone();
        let mut slots = callee.spots.unwrap_or_default();
        let result = callee.ty;
        let mut filled: Vec<Option<Supply>> = vec![None; slots.len()];
        let head_slot = callee.supplied.as_ref().map(|(index, _, _)| *index);
        if let Some((index, ty, span)) = callee.supplied {
            filled[index] = Some(Supply::Typed(ty, span));
        }
        let consumed = self.flags(&slots, arguments, &mut filled);
        let extra = self.positional(&slots, arguments, &consumed, &mut filled);
        if let Some((ty, span)) = pipe {
            match filled.iter().rposition(Option::is_none) {
                Some(index) => filled[index] = Some(Supply::Typed(ty, span)),
                None => self.error(stage, Reason::FilledSpot),
            }
        }
        // The call's type is the open spots' function only when no operand
        // is left for the result to take; past the spot list, the result
        // is applied first and the caller's check meets what it returns.
        if let Some(expected) = expected.filter(|_| extra.is_empty()) {
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
            let source_slot = usize::from(
                !conversion_word(kind) && callee.self_slot == Some(0),
            );
            self.convert_supply(
                (target, kind),
                &slots[source_slot].ty,
                filled.get(source_slot).and_then(Option::as_ref),
                &result,
                mark,
                stage,
            );
        }
        let home = callee.home.as_ref().unwrap_or(&Origin::This);
        self.supply_declared_defaults(home, &slots, &filled, stage);
        self.dots();
        self.refine_member_arguments(
            contract_binding.as_ref(),
            &slots,
            &filled,
            stage,
        );
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
                let name = slot.name.clone()?;
                let supply = match value {
                    Some(value) => value.clone(),
                    None => Supply::Declared(home.clone(), slot.value?),
                };
                Some((name, supply))
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
        let open = slots
            .iter()
            .zip(&filled)
            .any(|(slot, supply)| supply.is_none() && !slot.default);
        let open_bool = slots.iter().zip(&filled).any(|(slot, supply)| {
            supply.is_none() && !slot.default && self.is_bool(&slot.ty)
        });
        let result = slots
            .iter()
            .zip(&filled)
            .rev()
            .filter(|(slot, supply)| supply.is_none() && !slot.default)
            .fold(result, |ty, (slot, _)| Type::function(slot.ty.clone(), ty));
        self.apply_extra(result, extra, (open, open_bool), stage)
    }

    fn supply_declared_defaults(
        &mut self,
        home: &Origin,
        slots: &[super::Slot],
        filled: &[Option<Supply>],
        stage: Span,
    ) {
        for (index, supply) in filled.iter().enumerate() {
            if supply.is_none() && slots[index].value.is_some() {
                self.declared_default(home, slots, index, stage);
            }
        }
    }

    /// A conversion's source spot: a supplied value is converted to the
    /// target, with a literal read at the meaning it reaches; an unsupplied
    /// member conversion owes the member on the target.
    fn convert_supply(
        &mut self,
        (target, kind): (&Type, &str),
        slot: &Type,
        supply: Option<&Supply>,
        result: &Type,
        mark: usize,
        stage: Span,
    ) {
        if let Some(supply) = supply {
            self.read_literal_at_meaning(target, kind, supply, slot);
            self.default_literals(mark, &[], &[]);
            let source = self.apply(slot);
            let converted = self.convert(target, kind, &source, stage);
            self.unify(result, &converted, stage, stage);
        } else if !conversion_word(kind) {
            self.push_owed(super::Owed::new(
                crate::types::Obligation::Member {
                    head: target.clone(),
                    name: kind.into(),
                    ty: Type::function(slot.clone(), result.clone()),
                    dot: stage,
                },
                stage,
            ));
        }
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
            let listing = self.snapshot();
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
            self.keep_except(listing, super::trial::Field::DefaultInstances);
            bound.defaults.sort_by_key(|(spot, _)| fields[*spot]);
            self.insert(|s| &mut s.bound, (start, end), bound);
            self.insert(|s| &mut s.constructor_fields, (start, end), fields);
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
            self.insert(|s| &mut s.derived_defaults, (start, end), value);
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
                let listing = self.snapshot();
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
                self.keep_except(
                    listing,
                    super::trial::Field::DefaultInstances,
                );
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
                None | Some(Supply::Default(_) | Supply::Declared(..)) => None,
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
        self.insert(|s| &mut s.bound, (stage.start, stage.end), bound);
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
                            self.duplicate_supply(&slots[slot], argument.span);
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
                let reason = if slot.is_some() {
                    Reason::FlagOperand { name }
                } else {
                    Reason::UnknownSpot { name }
                };
                self.error(argument.span, reason);
                if let Some(value) = value {
                    self.expr(*value, None);
                }
                continue;
            };
            if filled[slot].is_some() {
                self.duplicate_supply(&slots[slot], argument.span);
            } else {
                filled[slot] = Some(supply);
            }
        }
        consumed
    }

    fn duplicate_supply(&mut self, slot: &Slot, span: Span) {
        let name = slot.name.clone().unwrap_or_default();
        self.error(span, Reason::DuplicateSupply { name });
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

    /// Step 2: positional operands fill unfilled non-Bool spots in order;
    /// those past the spot list are returned for the result to take.
    fn positional(
        &self,
        slots: &[Slot],
        arguments: &[Argument],
        consumed: &[bool],
        filled: &mut [Option<Supply>],
    ) -> Vec<ExprId> {
        let mut extra = Vec::new();
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
                extra.push(*value);
            }
        }
        extra
    }

    /// Step 4's tail: the bound result, a function, takes the operands
    /// written past the spot list, as 08-evaluation's APP applies them.
    /// An operand left over while a Bool spot is open was refused by
    /// that spot, which only a flag or the pipe fills.
    fn apply_extra(
        &mut self,
        result: Type,
        extra: Vec<ExprId>,
        (open, open_bool): (bool, bool),
        stage: Span,
    ) -> Type {
        if extra.is_empty() {
            return result;
        }
        // A result still a variable may be a function, unless a claim on
        // it says otherwise: `inc 1 2` has one argument too many.
        let applied = self.apply(&result);
        let function = match &applied {
            Type::Function { .. } => true,
            Type::Var(_) => !self.state.obligations.iter().any(|owed| {
                self.state.subst.apply(owed.obligation.subject()) == applied
            }),
            _ => false,
        };
        let peeled = if open || !function {
            None
        } else {
            self.peel(&result, extra.len())
        };
        let Some((parameters, rest)) = peeled else {
            let reason = if open_bool {
                Reason::PositionalBool
            } else {
                Reason::ExtraArgument
            };
            for operand in extra {
                let span = self.span(operand);
                self.error(span, reason.clone());
                self.expr(operand, None);
            }
            return result;
        };
        for (operand, parameter) in extra.iter().zip(&parameters) {
            let expected = Expected {
                place_kind: crate::typed::Place::Parameter(None),
                ty: parameter.clone(),
                span: self.span(*operand),
                spot: true,
                cast: true,
            };
            self.expr(*operand, Some(&expected));
        }
        self.journal_key(|s| &mut s.bound, (stage.start, stage.end));
        if let Some(bound) = self.state.bound.get_mut(&(stage.start, stage.end))
        {
            bound.applied = parameters.into_iter().zip(extra).collect();
        }
        rest
    }

    fn enter(
        &mut self,
        target: &Type,
        actual: &Type,
        span: Span,
        other: Span,
        place_kind: crate::typed::Place,
    ) {
        let variable = matches!(target, Type::Var(_));
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
                    variable,
                    Some(other),
                ));
            }
        }
        let numeric = self.numeric_var(actual);
        let errors = self.state.errors.len();
        let first = self.snapshot();
        self.unify(&target, actual, span, other);
        if numeric {
            self.dots();
        }
        if self.state.errors.len() == errors
            || self.state.errors[errors..]
                .iter()
                .any(|e| super::bind::decided_literal(&e.reason))
        {
            self.keep(first);
            return;
        }
        self.restore(first);
        let second = self.snapshot();
        self.default_entering(actual);
        let source = self.apply(actual);
        if let Some(row) = self.cast_pair(&target, &source, span, other) {
            if let Some(index) = self.expression_ids.get(&span).copied() {
                self.record_cast(ExprId(index), row, source, target);
            }
            self.keep(second);
        } else if self.state.errors[errors..]
            .iter()
            .any(|e| matches!(e.reason, Reason::OutcomeText { .. }))
        {
            self.keep(second);
        } else {
            // Neither fits: the unification against the place reports.
            self.restore(second);
            self.unify(&target, actual, span, other);
            if numeric {
                self.dots();
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
                            let errors = self.state.errors.len();
                            let whole = self.snapshot();
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
                            if self.state.errors.len() == errors {
                                self.keep(whole);
                                *supply = Some(Supply::Expr(id));
                                continue;
                            }
                            self.restore(whole);
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
        self.insert(|s| &mut s.bound, (stage.start, stage.end), bound);
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
        let kind = &module.expressions[id.0].kind;
        if !kind.supplies_piped_value() {
            let result = self.fresh();
            let function = Type::function(input, result.clone());
            self.expr(id, Some(&Expected::new(function, input_span)));
            return result;
        }
        let result = if let ExprKind::Call {
            callee,
            arguments,
            style,
        } = kind
        {
            let callee = self.callee(*callee);
            self.written_call(callee, arguments, *style, pipe, span, expected)
        } else {
            let callee = self.callee(id);
            self.apply_callee(callee, &[], pipe, span, expected)
        };
        // The stage's call lists the piped value among its supplies, so
        // its node is the stage's result, not a function of the input.
        self.set_type(id.0, result.clone());
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
        if self.declared_home.as_ref() == Some(home) {
            match self.source_default(
                home,
                &slots[index],
                &slots[..index],
                stage,
            ) {
                Ok(Some(instance)) => self.set_default_instance(
                    (stage.start, stage.end, index),
                    instance,
                ),
                Ok(None) => {}
                Err(cause) => self.error(
                    stage,
                    Reason::Default {
                        name: slots[index].name.clone().unwrap_or_default(),
                        cause,
                    },
                ),
            }
            return;
        }
        let (_, valid) = self.declared_expression(
            home,
            &slots[index],
            &slots[..index],
            stage,
            true,
        );
        if valid.is_ok() {
            if let Some(value) = slots[index].value {
                if let Some(recorded) = self
                    .file_cache
                    .expressions
                    .borrow()
                    .get(&(home.clone(), value.0))
                    .cloned()
                {
                    self.set_default_instance(
                        (stage.start, stage.end, index),
                        recorded,
                    );
                }
            }
        }
        if let Err(cause) = valid {
            self.error(
                stage,
                Reason::Default {
                    name: slots[index].name.clone().unwrap_or_default(),
                    cause,
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
    ) -> (Scheme, Result<(), Option<alloc::boxed::Box<Reason>>>) {
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

    /// A declared expression typed in its own module against its slot.
    /// `Err` carries the first error of that typing, none when a name
    /// of the expression did not resolve.
    pub(super) fn declared_context(
        &mut self,
        home: &Origin,
        slot: &Slot,
        earlier: &[Slot],
        stage: Span,
        is_default: bool,
        owner: Option<&Type>,
    ) -> (Scheme, Result<(), Option<alloc::boxed::Box<Reason>>>) {
        // Recursive defaults use their signatures. Evaluation decides termination.
        let key = (home.clone(), slot.site);
        if self.state.defaults.contains(&key) {
            return (Scheme::mono(slot.ty.clone()), Ok(()));
        }
        let (Some(module), Some(value)) = (self.owner(home), slot.value) else {
            return (Scheme::mono(slot.ty.clone()), Err(None));
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
            return (Scheme::mono(slot.ty.clone()), Ok(()));
        }
        // A file's default is typed as the file types it, naming the
        // file's own declarations as the file does; what the outside
        // supplies is renamed in, what it solves and owes renamed out.
        let same_file = matches!(home, Origin::File(stem) if self.checked_file_scope.as_ref() == Some(stem));
        let foreign =
            (owner.is_none() && matches!(home, Origin::File(_)) && !same_file)
                .then(|| self.state.subst.solved());
        let supplied = |ty: &Type| {
            if foreign.is_some() {
                ty.unhome(home)
            } else {
                ty.clone()
            }
        };
        let earlier: Vec<_> = earlier
            .iter()
            .filter_map(|slot| {
                Some((slot.name.clone()?, slot.site, supplied(&slot.ty)))
            })
            .collect();
        let names: Vec<_> = earlier
            .iter()
            .map(|(name, site, _)| (name.clone(), *site))
            .collect();
        let (resolved, resolve_errors) =
            self.default_resolution(home, module, value, &names, owner);
        let slot = &Slot {
            ty: supplied(&slot.ty),
            ..slot.clone()
        };
        let mut inner = self.inner(module, &resolved, home, key, earlier);
        if let Some(owner) = owner {
            inner.assume_test_owner(owner, slot.site);
        }
        let (scheme, mut typed, errors) =
            inner.check_declared_body(value, slot, stage, is_default);
        if !is_default {
            typed
                .schemes
                .insert((slot.site.start, slot.site.end), scheme.clone());
            if resolve_errors.is_empty()
                && inner.state.errors.is_empty()
                && errors.is_empty()
            {
                inner.complete_owned_references(&mut typed);
                typed.complete_references(&inner.state.subst);
            }
            typed.hidden = inner.record_hidden(&typed.schemes, &typed);
        }
        let valid =
            if resolve_errors.is_empty() {
                inner.state.errors.iter().chain(&errors).next().map_or(
                    Ok(()),
                    |error| {
                        Err(Some(alloc::boxed::Box::new(error.reason.clone())))
                    },
                )
            } else {
                Err(None)
            };
        let recorded = valid.is_ok().then(|| typed.clone());
        if owner.is_none() && is_default {
            self.adopt_checks(typed.checks, home, stage, !same_file);
        }
        let owed = inner
            .state
            .obligations
            .into_iter()
            .filter(|_| owner.is_none())
            .collect();
        self.adopt_solutions(
            inner.state.subst,
            owed,
            foreign.as_deref(),
            home,
            stage,
        );
        self.record_declared(home, value, resolved, recorded);
        let scheme = if foreign.is_some() {
            scheme.rehome(home)
        } else {
            scheme
        };
        (scheme, valid)
    }

    /// The inner typing's solutions and a default's obligations become
    /// this unit's; a file's are renamed to the file's home.
    fn adopt_solutions(
        &mut self,
        subst: crate::types::Subst,
        owed: Vec<super::Owed>,
        foreign: Option<&[bool]>,
        home: &Origin,
        stage: Span,
    ) {
        self.state.subst = subst;
        if let Some(solved) = foreign {
            self.state.subst.rehome_since(solved, home);
        }
        for mut owed in owed {
            owed.span = stage;
            if foreign.is_some() {
                owed.obligation = owed.obligation.map(|ty| ty.rehome(home));
            }
            self.push_owed(owed);
        }
    }

    /// What an expression of `home` sees besides its own module: this
    /// unit's locals and items, a file's bindings typed with the file;
    /// a line's bindings are the frame's, found by the lookup.
    fn scope_inner(&self, inner: &mut Infer<'_>, home: &Origin) {
        match home {
            Origin::This => {
                inner.state.locals.clone_from(&self.state.locals);
                inner.state.status.clone_from(&self.state.status);
                inner.items.clone_from(&self.items);
                inner.groups.clone_from(&self.groups);
                inner.self_type.clone_from(&self.self_type);
                inner
                    .checked_file_scope
                    .clone_from(&self.checked_file_scope);
            }
            Origin::File(stem) => {
                inner.state.locals.extend(self.file_locals(stem));
                if self.checked_file(stem).is_some() {
                    inner.checked_file_scope = Some(stem.clone());
                }
            }
            _ => {}
        }
    }
}

impl<'a> Infer<'a> {
    /// An inference over `module` for one declared expression of `home`,
    /// continuing this unit's substitution, with the earlier spots as
    /// locals and `key` among the defaults being typed.
    fn inner<'r>(
        &mut self,
        module: &'r crate::ast::Module,
        resolved: &'r crate::resolve::Resolved,
        home: &Origin,
        key: (Origin, Span),
        earlier: Vec<(String, Span, Type)>,
    ) -> Infer<'r>
    where
        'a: 'r,
    {
        let subst = core::mem::take(&mut self.state.subst);
        let mut inner = Infer {
            module,
            resolved,
            declarations: self.declarations,
            files: self.files,
            frame: self.frame,
            visiting: self.visiting.clone(),
            file_cache: self.file_cache,
            indexes: self.indexes.clone(),
            expression_ids: super::expression_ids(module),
            operators: super::operators(module),
            weak: self.weak,
            state: State::new(subst, module.expressions.len()),
            items: Vec::new(),
            groups: Vec::new(),
            self_type: None,
            declared_home: None,
            checked_file_scope: None,
        };
        self.scope_inner(&mut inner, home);
        inner.state.defaults.clone_from(&self.state.defaults);
        inner.state.defaults.push(key);
        for (_, site, ty) in earlier {
            let local = Local {
                owner: None,
                scheme: Scheme::mono(ty),
                heads: None,
            };
            inner.state.locals.insert((site.start, site.end), local);
        }
        inner
    }

    /// A default's limit checks, listed at this call with their limits
    /// kept in the declaring module.
    fn adopt_checks(
        &mut self,
        checks: Vec<crate::limits::Check>,
        home: &Origin,
        stage: Span,
        rehome_types: bool,
    ) {
        for mut check in checks {
            check.role = crate::limits::CheckRole::CallerReport;
            check.place = stage;
            check.receiving_call = None;
            check.frequency = crate::limits::frequency(self.module, stage);
            check.place_origin = Origin::This;
            if check.limit_origin == Origin::This {
                check.limit_origin = home.clone();
            }
            if rehome_types {
                check.target = check.target.map(|ty| ty.rehome(home));
                check.known.rehome(home);
            }
            self.state.checks.push(check);
        }
    }
    /// The resolution a declared expression is typed under: its own
    /// module's for this unit, an accepted line or a file, which saw the
    /// frame and the module's bindings; a declaration's is built here.
    fn default_resolution(
        &self,
        home: &Origin,
        module: &crate::ast::Module,
        value: ExprId,
        names: &[(String, Span)],
        owner: Option<&Type>,
    ) -> (Rc<crate::resolve::Resolved>, Vec<crate::resolve::Error>) {
        let owner_name = owner.and_then(|ty| {
            if let Type::Named { name, .. } = ty {
                Some(name.name.as_str())
            } else {
                None
            }
        });
        let own = match home {
            Origin::This => Some(Rc::new(self.resolved.clone())),
            Origin::Line(line) => self
                .frame
                .and_then(|frame| frame.lines.get(*line))
                .map(|line| Rc::new(line.resolved.clone())),
            Origin::File(stem) => {
                self.checked_file(stem).map(|file| file.resolved)
            }
            _ => None,
        };
        if let Some(resolved) = own {
            return (resolved, Vec::new());
        }
        let (resolved, errors) = crate::resolve::resolve_declared_expression(
            module,
            self.declarations,
            home,
            value,
            names,
            owner_name,
        );
        (Rc::new(resolved), errors)
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
            // A literal default at a spot still open chooses with the
            // call's other literals, not alone.
            if !matches!(self.apply(&slot.ty), Type::Var(_)) {
                self.default_literals(0, &[], &[]);
            }
            Scheme::mono(self.apply(&slot.ty))
        } else {
            let annotated = self.module.entries.iter().any(|entry| {
                let crate::ast::Entry::Line(id) = entry else { return false };
                let crate::ast::ExprKind::Bind {
                    pattern, annotation: Some(_), value: bound, ..
                } = &self.module.expressions[id.0].kind else { return false };
                let pattern = &self.module.patterns[pattern.0];
                if *bound != value { return false; }
                pattern.span == slot.site
                    && matches!(&pattern.kind, crate::ast::PatternKind::Name(name)
                        if name.segments.last().map(String::as_str) == slot.name.as_deref())
            });
            self.generalize(
                &[super::bind::Part {
                    owner: None,
                    ty: slot.ty.clone(),
                    site: slot.site,
                    annotated,
                }],
                mark,
                &[],
            )
            .remove(0)
            .scheme
        };
        self.finish_dots();
        self.settle();
        self.record_supplied_tests();
        let (typed, errors) = self.default_checks(Some(value));
        (scheme, typed, errors)
    }

    pub(super) fn constructor_error(
        &mut self,
        home: &Origin,
        name: &str,
        site: Span,
    ) -> Option<Local> {
        self.error(
            self.module.span,
            Reason::DeclarationValue {
                name: name.into(),
                origin: home.clone(),
                declaration: site,
            },
        );
        None
    }

    pub(super) fn supplied_constructor(
        &mut self,
        home: &Origin,
        name: &str,
        module: &crate::ast::Module,
        site: Span,
    ) -> Option<Local> {
        use super::files::Constructor;
        let key = (home.clone(), site);
        if let Some(cached) = self.file_cache.constructor(&key) {
            return match cached {
                Constructor::Checking(local)
                | Constructor::Checked(local, _) => Some(local),
                Constructor::Failed => self.constructor_error(home, name, site),
            };
        }
        self.file_cache
            .constructor_scopes
            .borrow_mut()
            .push((key.clone(), alloc::collections::BTreeSet::new()));
        let mut state = self.check_constructor_group(home, name, module, site);
        let (_, dependencies) = self
            .file_cache
            .constructor_scopes
            .borrow_mut()
            .pop()
            .expect("constructor check scope");
        if let Constructor::Checked(_, source) = &mut state {
            source.dependencies = dependencies.into_iter().collect();
            for cached in self.file_cache.constructors.borrow_mut().values_mut()
            {
                if let Constructor::Checked(_, sibling) = cached {
                    if Rc::ptr_eq(&sibling.typed, &source.typed) {
                        sibling.dependencies.clone_from(&source.dependencies);
                    }
                }
            }
        }
        self.file_cache
            .constructors
            .borrow_mut()
            .insert(key.clone(), state);
        if let Some(Constructor::Checked(local, _)) =
            self.file_cache.constructor(&key)
        {
            Some(local)
        } else {
            self.constructor_error(home, name, site)
        }
    }

    fn pending_constructor(&mut self, index: usize, site: Span) -> Local {
        // A recursive caller already knows a constructor returns its owning Self.
        let mut result = self.items[index].ty.clone();
        while let Type::Function { result: next, .. } = result {
            result = *next;
        }
        let owner = self.groups[self.items[index].group].self_type.clone();
        if matches!(self.items[index].body, Some(super::Body::Arms(_))) {
            if let Some(owner) = &owner {
                self.unify(owner, &result, site, site);
            }
        }
        Local {
            owner: matches!(
                self.items[index].body,
                Some(super::Body::Value(_))
            )
            .then_some(owner)
            .flatten(),
            scheme: Scheme::mono(self.apply(&self.items[index].ty)),
            heads: self.items[index].heads.clone(),
        }
    }

    fn check_constructor_group(
        &mut self,
        home: &Origin,
        name: &str,
        module: &crate::ast::Module,
        site: Span,
    ) -> super::files::Constructor {
        use super::files::Constructor;
        let key = (home.clone(), site);
        let (resolved, resolver_groups) =
            crate::resolve::resolve_declared_source(
                module,
                self.declarations,
                home,
            );
        let resolved = Rc::new(resolved);
        let mut inner =
            self.inner(module, &resolved, home, key.clone(), Vec::new());
        inner.declared_home = Some(home.clone());
        inner.declare();
        inner.declare_source_bindings();
        let Some(index) = inner.items.iter().position(|item| item.site == site).or_else(|| {
            module.entries.iter().any(|entry| matches!(entry, crate::ast::Entry::Item(crate::ast::Item { kind: crate::ast::ItemKind::Impl { members, .. }, .. }) if members.iter().any(|m| matches!(m, crate::ast::ImplMember::Constructor { arm, .. } if arm.span == site))))
                .then(|| inner.item_index(&Key::Constructor(name.into()))).flatten()
        }) else { self.state.subst = inner.state.subst; return Constructor::Failed; };
        let pending = inner.pending_constructor(index, site);
        inner
            .file_cache
            .constructors
            .borrow_mut()
            .insert(key, Constructor::Checking(pending));
        let _callee = inner.item_callee(index, site);
        inner.finish_dots();
        inner.settle();
        let reached: Vec<_> = inner
            .items
            .iter()
            .zip(&inner.state.status)
            .filter(|(_, status)| matches!(status, super::Status::Done(_)))
            .map(|(item, _)| item.site)
            .collect();
        let resolve_valid = resolver_groups.iter().all(|(span, errors)| {
            errors.is_empty()
                || !reached.iter().any(|site| {
                    span.start <= site.start && site.end <= span.end
                })
        });
        if resolve_valid && inner.state.errors.is_empty() {
            inner.finish_source_defaults();
            inner.record_supplied_tests();
        }
        let (frozen, _, _) = inner.frozen();
        let local = frozen
            .iter()
            .find(|(at, _)| *at == (site.start, site.end))
            .map(|(_, local)| local.clone());
        let (mut typed, limits) =
            if resolve_valid && inner.state.errors.is_empty() {
                inner.default_checks(None)
            } else {
                (crate::typed::Typed::default(), Vec::new())
            };
        typed.frozen.clone_from(&frozen);
        let valid =
            resolve_valid && inner.state.errors.is_empty() && limits.is_empty();
        if valid {
            inner.finish_source_default_masks(&mut typed);
        }
        let roots = inner.reached_source_roots();
        let subst = inner.state.subst;
        let owed = inner.state.obligations;
        self.adopt_solutions(subst, owed, None, home, self.module.span);
        if !valid {
            return Constructor::Failed;
        }
        for plan in &inner.state.pending_defaults {
            self.file_cache.record(
                (home.clone(), plan.instance.value.0),
                plan.instance.clone(),
            );
        }
        let source = crate::typed::DeclaredConstructor {
            home: home.clone(),
            dependencies: Vec::new(),
            roots,
            resolved,
            typed: Rc::new(typed),
        };
        // Publish only finalized reached locals; every entry shares the same source transaction.
        for (at, local) in frozen {
            if reached.iter().any(|site| at == (site.start, site.end)) {
                self.file_cache.constructors.borrow_mut().insert(
                    (
                        home.clone(),
                        Span {
                            start: at.0,
                            end: at.1,
                        },
                    ),
                    Constructor::Checked(local, source.clone()),
                );
            }
        }
        local.map_or(Constructor::Failed, |local| {
            Constructor::Checked(local, source)
        })
    }

    fn reached_source_roots(&self) -> Vec<ExprId> {
        self.items
            .iter()
            .zip(&self.state.status)
            .filter(|(_, status)| matches!(status, super::Status::Done(_)))
            .flat_map(|(item, _)| match &item.body {
                Some(super::Body::Value(value)) => vec![*value],
                Some(super::Body::Arms(arms)) => arms
                    .iter()
                    .flat_map(|arm| {
                        arm.guard.into_iter().chain(core::iter::once(arm.body))
                    })
                    .collect(),
                None => Vec::new(),
            })
            .collect()
    }

    fn scope_source_default(
        &self,
        typed: &mut crate::typed::Typed,
        value: ExprId,
        earlier: &[Slot],
    ) -> SourceNodes {
        let roots = core::iter::once(value).chain(
            typed
                .tests
                .iter()
                .filter(|test| test.expression.is_none())
                .map(|test| test.value),
        );
        let nodes = SourceNodes::collect(self.module, roots);
        for (index, ty) in typed.expressions.iter_mut().enumerate() {
            if !nodes.expressions.contains(&index) {
                *ty = None;
            }
        }
        for (index, cast) in typed.casts.iter_mut().enumerate() {
            if !nodes.expressions.contains(&index) {
                *cast = None;
            }
        }
        for (index, scheme) in typed.bindings.iter_mut().enumerate() {
            if !nodes.expressions.contains(&index) {
                *scheme = None;
            }
        }
        for (index, known) in typed.known.iter_mut().enumerate() {
            if !nodes.expressions.contains(&index) {
                *known = crate::limits::known::Known::default();
            }
        }
        typed
            .pattern_types
            .retain(|id, _| nodes.patterns.contains(id));
        typed
            .pattern_equals
            .retain(|id, _| nodes.patterns.contains(id));
        macro_rules! scoped { ($($field:ident),*) => { $(typed.$field.retain(|site,_|nodes.sites.contains(site));)* }; }
        scoped!(
            selected,
            meaning_arguments,
            callee_types,
            accesses,
            roots,
            bound,
            conversions,
            rebuilds,
            rebuild_types,
            modes,
            supplied,
            constructor_fields,
            derived_defaults
        );
        typed
            .default_instances
            .retain(|(start, end, _), _| nodes.sites.contains(&(*start, *end)));
        typed.checks.retain(|check| match check.value {
            Some(id) => nodes.expressions.contains(&id.0),
            None => {
                nodes.sites.contains(&(check.place.start, check.place.end))
                    || check.receiving_call.is_some_and(|call| {
                        nodes.sites.contains(&(call.start, call.end))
                    })
            }
        });
        typed.facts.retain(|(site, _)| nodes.sites.contains(site));
        typed
            .warnings
            .retain(|w| nodes.sites.contains(&(w.span.start, w.span.end)));
        let mut locals = nodes.sites.clone();
        locals.extend(
            earlier.iter().map(|slot| (slot.site.start, slot.site.end)),
        );
        for id in &nodes.expressions {
            if let Some(crate::resolve::Target::Local { site }) =
                self.resolved.expr(ExprId(*id))
            {
                locals.insert((site.start, site.end));
            }
        }
        locals.extend(
            self.items
                .iter()
                .zip(&self.state.status)
                .filter(|(_, status)| matches!(status, super::Status::Done(_)))
                .map(|(item, _)| (item.site.start, item.site.end)),
        );
        locals.extend(
            typed
                .tests
                .iter()
                .map(|test| (test.self_site.start, test.self_site.end)),
        );
        typed.schemes.retain(|site, _| locals.contains(site));
        typed.frozen.retain(|(site, _)| locals.contains(site));
        typed
            .body_parameters
            .retain(|site, _| locals.contains(site));
        typed.hidden.retain(|site, _| locals.contains(site));
        nodes
    }

    fn source_default_context(
        &self,
        owner: &super::LexicalOwner,
        home: &Origin,
        resolved: &Rc<crate::resolve::Resolved>,
        earlier: &[Slot],
    ) -> Rc<super::LexicalContext> {
        let mut lexical = State::new(
            crate::types::Subst::default(),
            self.module.expressions.len(),
        );
        lexical.env.clone_from(&self.state.env);
        lexical.claims.clone_from(&self.state.claims);
        let globals = if self.state.lexical_owner.is_none() {
            &self.state.locals
        } else {
            &self
                .state
                .lexical_frames
                .iter()
                .rev()
                .find(|frame| frame.owner.is_none())
                .expect("unit lexical frame")
                .facts
                .locals
        };
        for (item, status) in self.items.iter().zip(&self.state.status) {
            if matches!(status, super::Status::Done(_)) {
                let site = (item.site.start, item.site.end);
                if let Some(local) = globals.get(&site) {
                    lexical.locals.insert(site, local.clone());
                }
            }
        }
        for prior in earlier.iter().filter(|s| s.name.is_some()) {
            lexical.locals.insert(
                (prior.site.start, prior.site.end),
                Local {
                    owner: None,
                    scheme: Scheme::mono(prior.ty.clone()),
                    heads: None,
                },
            );
        }
        Rc::new(super::LexicalContext {
            owner: owner.clone(),
            resolved: Rc::clone(resolved),
            self_type: self.self_type.clone(),
            home: Some(home.clone()),
            facts: super::LexicalFacts::capture(&lexical),
        })
    }

    fn resolve_source_default(
        &self,
        home: &Origin,
        value: ExprId,
        earlier: &[Slot],
    ) -> Option<Rc<crate::resolve::Resolved>> {
        let names: Vec<_> = earlier
            .iter()
            .filter_map(|s| Some((s.name.clone()?, s.site)))
            .collect();
        let (extra, errors) = crate::resolve::resolve_declared_expression(
            self.module,
            self.declarations,
            home,
            value,
            &names,
            None,
        );
        if !errors.is_empty() {
            return None;
        }
        Some(Rc::new(crate::resolve::declared_overlay(
            self.resolved,
            &extra,
        )))
    }

    fn source_default(
        &mut self,
        home: &Origin,
        slot: &Slot,
        earlier: &[Slot],
        stage: Span,
    ) -> Result<
        Option<crate::typed::DeclaredExpression>,
        Option<alloc::boxed::Box<Reason>>,
    > {
        let key = (home.clone(), slot.site);
        if self.state.defaults.contains(&key) {
            return Ok(None);
        }
        let Some(value) = slot.value else {
            return Err(None);
        };
        let Some(resolved) = self.resolve_source_default(home, value, earlier)
        else {
            return Err(None);
        };
        let token = Rc::new(super::DefaultOwner { key });
        let owner = Some(Rc::clone(&token));
        let context =
            self.source_default_context(&owner, home, &resolved, earlier);
        let provisional = crate::typed::DeclaredExpression {
            origin: home.clone(),
            value,
            resolved: Rc::clone(&resolved),
            typed: Rc::new(crate::typed::Typed::default()),
        };
        let use_start = self.state.uses.len();
        self.journal_list(|state| &mut state.pending_defaults);
        self.state.pending_defaults.push(super::PendingDefault {
            context: Rc::clone(&context),
            earlier: earlier.to_vec(),
            instance: provisional,
            uses: Vec::new(),
            invocation: use_start..use_start,
            roots: alloc::collections::BTreeMap::new(),
        });
        // Selective rollback preserves the token but clears its temporary lookup maps.
        self.state.journal(super::trial::Undo::Capsule(context));
        let (instance, failure, uses, end, roots) =
            self.with_lexical_owner(&owner, |inner| {
                let error_start = inner.state.errors.len();
                let expected = Expected {
                    place_kind: slot.place_kind.clone(),
                    ty: slot.ty.clone(),
                    span: stage,
                    spot: true,
                    cast: true,
                };
                inner.expr(value, Some(&expected));
                inner.dots();
                // The common group's ordinary marks own literal/rebuild finishing.
                // Never drain its obligations at a lexical default return.
                let (mut typed, limits) = inner.default_checks(Some(value));
                let nodes =
                    inner.scope_source_default(&mut typed, value, earlier);
                let failure = inner.state.errors[error_start..]
                    .iter()
                    .chain(&limits)
                    .next()
                    .map(|error| alloc::boxed::Box::new(error.reason.clone()));
                inner.state.errors.truncate(error_start);
                let uses = inner
                    .state
                    .uses
                    .iter()
                    .cloned()
                    .enumerate()
                    .skip(use_start)
                    .filter(|(_, used)| {
                        super::same_owner(&used.lexical_owner, &owner)
                    })
                    .collect();
                let roots = typed
                    .roots
                    .keys()
                    .map(|site| (*site, inner.state.roots[site].clone()))
                    .collect();
                let _ = nodes;
                (
                    crate::typed::DeclaredExpression {
                        origin: home.clone(),
                        value,
                        resolved: Rc::clone(&resolved),
                        typed: Rc::new(typed),
                    },
                    failure,
                    uses,
                    inner.state.uses.len(),
                    roots,
                )
            });
        self.journal_list(|state| &mut state.pending_defaults);
        let pending = self
            .state
            .pending_defaults
            .iter_mut()
            .find(|pending| super::same_owner(&pending.context.owner, &owner))
            .expect("registered default invocation");
        pending.instance = instance.clone();
        pending.uses = uses;
        pending.invocation = use_start..end;
        pending.roots = roots;
        failure.map_or_else(|| Ok(Some(instance)), |failure| Err(Some(failure)))
    }

    fn finish_source_defaults(&mut self) {
        let plans = self.state.pending_defaults.clone();
        let mut replacements = Vec::new();
        for plan in plans.iter().rev() {
            let (mut refreshed, limits, saved_roots) =
                self.with_lexical_owner(&plan.context.owner, |inner| {
                    let (mut typed, limits) =
                        inner.default_checks(Some(plan.instance.value));
                    inner.scope_source_default(
                        &mut typed,
                        plan.instance.value,
                        &plan.earlier,
                    );
                    let roots = typed
                        .roots
                        .keys()
                        .map(|site| (*site, inner.state.roots[site].clone()))
                        .collect::<alloc::collections::BTreeMap<_, _>>();
                    (typed, limits, roots)
                });
            if !limits.is_empty() {
                self.state.errors.extend(limits);
                return;
            }
            // Keep provisional Rc identities until the exact child replacements below.
            let mut instance = plan.instance.clone();
            instance.typed = Rc::new(core::mem::take(&mut refreshed));
            instance =
                super::record::replace_recorded(&instance, &replacements);
            let typed = Rc::make_mut(&mut instance.typed);
            typed.schemes.extend(
                self.items.iter().zip(&self.state.status).filter_map(
                    |(item, status)| {
                        let super::Status::Done(member) = status else {
                            return None;
                        };
                        Some((
                            (item.site.start, item.site.end),
                            super::bind::applied(
                                &member.scheme,
                                &self.state.subst,
                            ),
                        ))
                    },
                ),
            );
            let Some(uses) = self.finalized_default_uses(
                plan,
                &instance.origin,
                instance.value,
            ) else {
                return;
            };
            self.complete_saved_references(typed, &uses, &saved_roots);
            typed.complete_references(&self.state.subst);
            let pending: Vec<_> = replacements
                .iter()
                .map(
                    |(_, instance): &(
                        Rc<crate::typed::Typed>,
                        crate::typed::DeclaredExpression,
                    )| Rc::clone(&instance.typed),
                )
                .collect();
            instance = super::record::apply_recorded_preserving(
                &instance,
                &self.state.subst,
                &pending,
            );
            replacements.push((Rc::clone(&plan.instance.typed), instance));
        }
        self.replace_source_defaults(&replacements);
        self.journal_list(|s| &mut s.pending_defaults);
        for plan in &mut self.state.pending_defaults {
            if let Some((_, instance)) = replacements
                .iter()
                .find(|(old, _)| Rc::ptr_eq(old, &plan.instance.typed))
            {
                plan.instance = instance.clone();
            }
        }
    }

    fn finalized_default_uses(
        &mut self,
        plan: &super::PendingDefault,
        origin: &Origin,
        value: ExprId,
    ) -> Option<Vec<super::Use>> {
        let mut uses = Vec::new();
        for (index, captured) in &plan.uses {
            let Some(current) = self.state.uses.get(*index).filter(|current| {
                current.item == captured.item
                    && super::same_owner(
                        &current.lexical_owner,
                        &captured.lexical_owner,
                    )
                    && current.span == captured.span
                    && current.ty == captured.ty
                    && current.owner == captured.owner
            }) else {
                self.error(
                    self.module.span,
                    Reason::DeclarationValue {
                        name: "default".into(),
                        origin: origin.clone(),
                        declaration: self.module.expressions[value.0].span,
                    },
                );
                return None;
            };
            let _ = current;
        }
        let owned_uses: Vec<_> = self
            .state
            .uses
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, used)| {
                super::same_owner(&used.lexical_owner, &plan.context.owner)
            })
            .collect();
        uses.extend(owned_uses.iter().map(|(_, used)| used.clone()));
        self.journal_list(|state| &mut state.pending_defaults);
        let current_plan = self
            .state
            .pending_defaults
            .iter_mut()
            .find(|pending| {
                super::same_owner(&pending.context.owner, &plan.context.owner)
            })
            .expect("final default owner");
        current_plan.uses = owned_uses;
        current_plan.invocation.end = self.state.uses.len();
        debug_assert!(plan
            .uses
            .iter()
            .all(|(index, _)| plan.invocation.contains(index)));
        Some(uses)
    }

    /// Seed retained lexical snapshots from the completed owner's actual checks.
    fn finish_source_default_masks(&mut self, typed: &mut crate::typed::Typed) {
        // The complete source, not a pruned default body, owns helper masks.
        let authoritative = &typed.hidden;
        let seed: alloc::collections::BTreeMap<_, _> = self
            .items
            .iter()
            .zip(&self.state.status)
            .filter(|(_, status)| matches!(status, super::Status::Done(_)))
            .filter_map(|(item, _)| {
                let site = (item.site.start, item.site.end);
                authoritative.get(&site).map(|mask| (site, mask.clone()))
            })
            .collect();
        let phase1 = self.state.pending_defaults.clone();
        let mut finished = Vec::new();
        for plan in phase1.iter().rev() {
            let mut instance =
                super::record::replace_recorded(&plan.instance, &finished);
            let typed = Rc::make_mut(&mut instance.typed);
            typed.hidden =
                self.record_hidden_seed(&typed.schemes, typed, &seed);
            finished.push((Rc::clone(&plan.instance.typed), instance));
        }
        self.replace_source_defaults(&finished);
        self.journal_list(|s| &mut s.pending_defaults);
        for plan in &mut self.state.pending_defaults {
            if let Some((_, instance)) = finished
                .iter()
                .find(|(old, _)| Rc::ptr_eq(old, &plan.instance.typed))
            {
                plan.instance = instance.clone();
            }
        }
        super::record::replace_typed(typed, &finished);
    }

    fn replace_source_defaults(
        &mut self,
        replacements: &[(
            Rc<crate::typed::Typed>,
            crate::typed::DeclaredExpression,
        )],
    ) {
        let defaults: Vec<_> = self
            .state
            .default_instances
            .iter()
            .map(|(at, instance)| {
                (*at, super::record::replace_recorded(instance, replacements))
            })
            .collect();
        for (at, instance) in defaults {
            self.set_default_instance(at, instance);
        }
        let bounds: Vec<_> = self
            .state
            .bound
            .iter()
            .map(|(at, bound)| {
                let mut bound = bound.clone();
                super::record::replace_bound(&mut bound, replacements);
                (*at, bound)
            })
            .collect();
        for (at, bound) in bounds {
            self.insert(|s| &mut s.bound, at, bound);
        }
        let defaults: Vec<_> = self
            .state
            .derived_defaults
            .iter()
            .map(|(at, default)| {
                let mut default = default.clone();
                super::record::replace_default(&mut default, replacements);
                (*at, default)
            })
            .collect();
        for (at, default) in defaults {
            self.insert(|s| &mut s.derived_defaults, at, default);
        }
        let accesses: Vec<_> = self
            .state
            .accesses
            .iter()
            .map(|(at, steps)| {
                let mut steps = steps.clone();
                for step in &mut steps {
                    if let Some(bound) = &mut step.bound {
                        super::record::replace_bound(bound, replacements);
                    }
                }
                (*at, steps)
            })
            .collect();
        for (at, steps) in accesses {
            self.insert(|s| &mut s.accesses, at, steps);
        }
    }

    fn record_declared(
        &self,
        home: &Origin,
        value: ExprId,
        resolved: Rc<crate::resolve::Resolved>,
        recorded: Option<crate::typed::Typed>,
    ) {
        if let Some(typed) = recorded {
            self.file_cache.record(
                (home.clone(), value.0),
                crate::typed::DeclaredExpression {
                    origin: home.clone(),
                    value,
                    resolved,
                    typed: Rc::new(typed),
                },
            );
        }
    }
    fn default_checks(
        &mut self,
        value: Option<ExprId>,
    ) -> (crate::typed::Typed, Vec<super::Error>) {
        self.record_operations();
        let shapes = self.record_shapes();
        let dispatch = self.record_dispatch(&shapes);
        let mut typed = crate::typed::Typed {
            dispatch,
            shapes,
            rebuilds: self.state.rebuilds.clone(),
            modes: self.state.modes.clone(),
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
            .map(|(id, ty, span, kind, variable, receiving_call)| {
                (
                    *id,
                    self.apply(ty),
                    *span,
                    kind.clone(),
                    *variable,
                    *receiving_call,
                )
            })
            .collect();
        let errors = crate::limits::check(
            &self.limit_inputs(),
            &entries,
            value,
            &mut typed,
        );
        typed.hidden = self.record_hidden(&typed.schemes, &typed);
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
        self.insert(|s| &mut s.bound, (span.start, span.end), bound);
    }
}

/// Whether a conversion is one of the words rather than a member.
fn conversion_word(kind: &str) -> bool {
    matches!(kind, "from" | "try_from" | "to" | "try")
}

#[derive(Default)]
pub struct SourceNodes {
    pub(crate) expressions: alloc::collections::BTreeSet<usize>,
    pub(crate) patterns: alloc::collections::BTreeSet<usize>,
    pub(crate) types: alloc::collections::BTreeSet<usize>,
    pub(crate) sites: alloc::collections::BTreeSet<(usize, usize)>,
}
enum DefaultNode {
    Expr(ExprId),
    Pattern(crate::ast::PatternId),
    Type(crate::ast::TypeId),
}
impl SourceNodes {
    fn site(&mut self, span: Span) {
        self.sites.insert((span.start, span.end));
    }
    fn collect(
        module: &crate::ast::Module,
        roots: impl IntoIterator<Item = ExprId>,
    ) -> Self {
        Self::from_roots(module, roots, core::iter::empty())
    }

    pub(crate) fn from_roots(
        module: &crate::ast::Module,
        expressions: impl IntoIterator<Item = ExprId>,
        patterns: impl IntoIterator<Item = crate::ast::PatternId>,
    ) -> Self {
        let mut found = Self::default();
        let mut pending: Vec<_> = expressions
            .into_iter()
            .map(DefaultNode::Expr)
            .chain(patterns.into_iter().map(DefaultNode::Pattern))
            .collect();
        while let Some(next) = pending.pop() {
            match next {
                DefaultNode::Expr(id) => {
                    if !found.expressions.insert(id.0) {
                        continue;
                    }
                    found.collect_expression(module, id, &mut pending);
                }
                DefaultNode::Pattern(id) => {
                    if !found.patterns.insert(id.0) {
                        continue;
                    }
                    found.collect_pattern(module, id, &mut pending);
                }
                DefaultNode::Type(id) => {
                    if !found.types.insert(id.0) {
                        continue;
                    }
                    found.collect_type(module, id, &mut pending);
                }
            }
        }
        found
    }

    fn collect_expression(
        &mut self,
        module: &crate::ast::Module,
        id: ExprId,
        pending: &mut Vec<DefaultNode>,
    ) {
        use crate::ast::ExprKind;
        let expr = &module.expressions[id.0];
        self.site(expr.span);
        match &expr.kind {
            ExprKind::Group(parts)
            | ExprKind::Tuple(parts)
            | ExprKind::List(parts)
            | ExprKind::Block(parts) => {
                pending.extend(parts.iter().copied().map(DefaultNode::Expr));
            }
            ExprKind::Outcome { payload, .. } => {
                pending.extend(payload.iter().copied().map(DefaultNode::Expr));
            }
            ExprKind::Instance { name, arguments } => {
                self.site(name.span);
                pending
                    .extend(arguments.iter().copied().map(DefaultNode::Type));
            }
            ExprKind::Record { name, fields } => {
                self.site(name.span);
                for field in fields {
                    self.site(field.span);
                    self.site(field.name.span);
                    pending.push(DefaultNode::Expr(field.value));
                }
            }
            ExprKind::Call {
                callee, arguments, ..
            } => self.collect_call(*callee, arguments, pending),
            ExprKind::Field { value, field } => {
                self.site(field.span);
                pending.push(DefaultNode::Expr(*value));
            }
            ExprKind::Not { operator, value } => {
                self.site(*operator);
                pending.push(DefaultNode::Expr(*value));
            }
            ExprKind::Binary {
                operator,
                left,
                right,
                ..
            }
            | ExprKind::Pipe {
                operator,
                left,
                right,
                ..
            } => {
                self.site(*operator);
                pending.push(DefaultNode::Expr(*left));
                pending.push(DefaultNode::Expr(*right));
            }
            ExprKind::Bind {
                pattern,
                annotation,
                value,
                ..
            } => {
                pending.push(DefaultNode::Pattern(*pattern));
                pending
                    .extend(annotation.iter().copied().map(DefaultNode::Type));
                pending.push(DefaultNode::Expr(*value));
            }
            ExprKind::With { key, value, body } => {
                self.site(key.span);
                pending.push(DefaultNode::Expr(*value));
                pending.push(DefaultNode::Expr(*body));
            }
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                pending.push(DefaultNode::Expr(*condition));
                pending.push(DefaultNode::Expr(*then_body));
                pending
                    .extend(else_body.iter().copied().map(DefaultNode::Expr));
            }
            ExprKind::Arms(arms) => self.collect_arms(arms, pending),
            ExprKind::Name(name) | ExprKind::Selector(name) => {
                self.site(name.span);
            }
            ExprKind::Literal(_) | ExprKind::Word(_) | ExprKind::Path(_) => {}
        }
    }

    fn collect_pattern(
        &mut self,
        module: &crate::ast::Module,
        id: crate::ast::PatternId,
        pending: &mut Vec<DefaultNode>,
    ) {
        use crate::ast::PatternKind;
        let pattern = &module.patterns[id.0];
        self.site(pattern.span);
        match &pattern.kind {
            PatternKind::Constructor { name, payload } => {
                self.site(name.span);
                pending.push(DefaultNode::Pattern(*payload));
            }
            PatternKind::Group(part) => {
                pending.push(DefaultNode::Pattern(*part));
            }
            PatternKind::Tuple(parts) | PatternKind::List(parts) => {
                pending.extend(parts.iter().copied().map(DefaultNode::Pattern));
            }
            PatternKind::Outcome { payload, .. } => pending
                .extend(payload.iter().copied().map(DefaultNode::Pattern)),
            PatternKind::Default { name, value } => {
                self.site(name.span);
                pending.push(DefaultNode::Expr(*value));
            }
            PatternKind::Record { name, fields, rest } => {
                self.site(name.span);
                if let Some(span) = rest {
                    self.site(*span);
                }
                for field in fields {
                    self.site(field.span);
                    self.site(field.name.span);
                    pending.extend(
                        field.pattern.iter().copied().map(DefaultNode::Pattern),
                    );
                }
            }
            PatternKind::Name(name) => self.site(name.span),
            PatternKind::Wildcard | PatternKind::Literal(_) => {}
        }
    }

    fn collect_type(
        &mut self,
        module: &crate::ast::Module,
        id: crate::ast::TypeId,
        pending: &mut Vec<DefaultNode>,
    ) {
        use crate::ast::{ConstraintKind, TypeKind};
        let ty = &module.types[id.0];
        self.site(ty.span);
        match &ty.kind {
            TypeKind::Named { name, arguments } => {
                self.site(name.span);
                pending
                    .extend(arguments.iter().copied().map(DefaultNode::Type));
            }
            TypeKind::Array { element, length } => {
                pending.push(DefaultNode::Type(*element));
                pending.push(DefaultNode::Type(*length));
            }
            TypeKind::Sum(parts)
            | TypeKind::Tuple(parts)
            | TypeKind::Both(parts)
            | TypeKind::Either(parts) => {
                pending.extend(parts.iter().copied().map(DefaultNode::Type));
            }
            TypeKind::Not(part) => {
                pending.push(DefaultNode::Type(*part));
            }
            TypeKind::Function { parameter, result } => {
                pending.push(DefaultNode::Type(*parameter));
                pending.push(DefaultNode::Type(*result));
            }
            TypeKind::Qualified { constraints, ty } => {
                pending.push(DefaultNode::Type(*ty));
                for constraint in constraints {
                    self.site(constraint.span);
                    match &constraint.kind {
                        ConstraintKind::Named { class, variable } => {
                            self.site(variable.span);
                            pending.push(DefaultNode::Type(*class));
                        }
                        ConstraintKind::Member { member, ty } => {
                            self.site(member.span);
                            pending.push(DefaultNode::Type(*ty));
                        }
                    }
                }
            }
            TypeKind::Labeled { name, ty, default } => {
                self.site(name.span);
                pending.push(DefaultNode::Type(*ty));
                pending.extend(default.iter().copied().map(DefaultNode::Expr));
            }
            TypeKind::Unit | TypeKind::Number(_) => {}
        }
    }

    fn collect_call(
        &mut self,
        callee: ExprId,
        arguments: &[Argument],
        pending: &mut Vec<DefaultNode>,
    ) {
        pending.push(DefaultNode::Expr(callee));
        for argument in arguments {
            self.site(argument.span);
            match &argument.kind {
                ArgumentKind::Positional(value) => {
                    pending.push(DefaultNode::Expr(*value));
                }
                ArgumentKind::Flag { value, .. } => {
                    pending
                        .extend(value.iter().copied().map(DefaultNode::Expr));
                }
            }
        }
    }

    fn collect_arms(
        &mut self,
        arms: &[crate::ast::Arm],
        pending: &mut Vec<DefaultNode>,
    ) {
        for arm in arms {
            self.site(arm.span);
            pending
                .extend(arm.patterns.iter().copied().map(DefaultNode::Pattern));
            pending.extend(arm.guard.iter().copied().map(DefaultNode::Expr));
            pending.push(DefaultNode::Expr(arm.body));
        }
    }
}

impl Infer<'_> {
    pub(super) fn attach_contract_binding(
        &mut self,
        mark: usize,
        binding: &super::ContractBindingUse,
    ) {
        let reads = match &binding.origin {
            Origin::This => self
                .state
                .member_contract_reads
                .get(&(binding.site.start, binding.site.end))
                .cloned(),
            Origin::Line(index) => self
                .frame
                .and_then(|frame| frame.lines.get(*index))
                .and_then(|line| {
                    line.typed
                        .member_contract_reads
                        .get(&(binding.site.start, binding.site.end))
                })
                .cloned(),
            _ => None,
        };
        let Some(reads) = reads else {
            return;
        };
        self.journal_list(|s| &mut s.obligations);
        for owed in &mut self.state.obligations[mark..] {
            let Some(ordinal) = owed.contract_ordinal else {
                continue;
            };
            if !matches!(
                owed.obligation,
                crate::types::Obligation::Member { .. }
            ) {
                continue;
            }
            owed.contract_use = Some(crate::typed::MemberContractUse {
                origin: binding.origin.clone(),
                binding: binding.site,
                at: binding.at,
                obligation: ordinal,
            });
            owed.contract_reads =
                reads.get(ordinal).cloned().unwrap_or_default();
        }
    }

    pub(super) fn local_contract_view(&self, site: Span, callee: &mut Callee) {
        if callee.spots.is_some() || crate::limits::has_limits(&callee.whole())
        {
            return;
        }
        let Some(value) = crate::limits::Bindings::of(self.module).site(site)
        else {
            return;
        };
        if let Some(ty) = self.contract_of_value(value, &mut Vec::new()) {
            callee.ty = ty;
        }
    }

    fn contract_of_value(
        &self,
        value: ExprId,
        visited: &mut Vec<ExprId>,
    ) -> Option<Type> {
        if visited.contains(&value) {
            // A recursive source supplies no declaration-contract evidence.
            return None;
        }
        visited.push(value);
        let result = match &self.module.expressions[value.0].kind {
            ExprKind::Name(_) => match self.resolved.expr(value) {
                Some(Target::Local { site }) => {
                    crate::limits::Bindings::of(self.module)
                        .site(*site)
                        .and_then(|value| {
                            self.contract_of_value(value, visited)
                        })
                }
                _ => None,
            },
            ExprKind::Group(parts) => parts
                .last()
                .and_then(|value| self.contract_of_value(*value, visited)),
            ExprKind::Block(lines) => lines
                .last()
                .and_then(|value| self.contract_of_value(*value, visited)),
            ExprKind::Call { .. } => {
                let span = self.span(value);
                self.state
                    .contract_stages
                    .get(&(span.start, span.end))
                    .and_then(|binding| {
                        self.contract_of_result(binding, span, visited)
                    })
            }
            _ => None,
        };
        visited.pop();
        result
    }

    fn contract_of_result(
        &self,
        binding: &super::ContractBindingUse,
        stage: Span,
        visited: &mut Vec<ExprId>,
    ) -> Option<Type> {
        if binding.origin != Origin::This {
            return None;
        }
        let bound = self.state.bound.get(&(stage.start, stage.end))?;
        if !bound.open.is_empty() || !bound.applied.is_empty() {
            return None;
        }
        let source =
            crate::limits::Bindings::of(self.module).site(binding.site)?;
        let ExprKind::Arms(arms) = &self.module.expressions[source.0].kind
        else {
            return None;
        };
        let [arm] = arms.as_slice() else {
            return None;
        };
        if bound.spots.len() != arm.patterns.len() {
            return None;
        }
        self.contract_of_returned_read(binding, arm.body, visited)
    }

    fn contract_of_returned_read(
        &self,
        binding: &super::ContractBindingUse,
        value: ExprId,
        visited: &mut Vec<ExprId>,
    ) -> Option<Type> {
        if visited.contains(&value) {
            return None;
        }
        visited.push(value);
        let result = match &self.module.expressions[value.0].kind {
            ExprKind::Group(parts) => parts.last().and_then(|value| {
                self.contract_of_returned_read(binding, *value, visited)
            }),
            ExprKind::Block(lines) => lines.last().and_then(|value| {
                self.contract_of_returned_read(binding, *value, visited)
            }),
            ExprKind::Name(_) => match self.resolved.expr(value) {
                Some(Target::Local { site }) => {
                    crate::limits::Bindings::of(self.module)
                        .site(*site)
                        .and_then(|value| {
                            self.contract_of_returned_read(
                                binding, value, visited,
                            )
                        })
                }
                Some(Target::Members { .. }) => self
                    .final_member_read(value, &mut Vec::new())
                    .and_then(|read| self.contract_at_read(binding, &read)),
                _ => None,
            },
            ExprKind::Field { .. } => self
                .final_member_read(value, &mut Vec::new())
                .and_then(|read| self.contract_at_read(binding, &read)),
            _ => None,
        };
        visited.pop();
        result
    }

    fn contract_at_read(
        &self,
        binding: &super::ContractBindingUse,
        read: &crate::typed::MemberContractRead,
    ) -> Option<Type> {
        let mut contracts = self
            .state
            .member_contracts
            .iter()
            .filter(|(key, contract)| {
                (&key.origin, key.binding, key.at)
                    == (&binding.origin, binding.site, binding.at)
                    && contract.reads.contains(read)
            })
            .map(|(_, contract)| self.apply(&contract.callable));
        let first = contracts.next();
        first.filter(|first| {
            matches!(first, Type::Function { .. })
                && contracts.all(|other| other == *first)
        })
    }

    fn final_member_read(
        &self,
        value: ExprId,
        visited: &mut Vec<ExprId>,
    ) -> Option<crate::typed::MemberContractRead> {
        if visited.contains(&value) {
            return None;
        }
        visited.push(value);
        let result = (|| {
            let step = match &self.module.expressions[value.0].kind {
                ExprKind::Group(parts) | ExprKind::Block(parts) => {
                    return parts.last().and_then(|value| {
                        self.final_member_read(*value, visited)
                    });
                }
                ExprKind::Field { field, .. } => {
                    field.segments.len().checked_sub(1)?
                }
                ExprKind::Name(_) => match self.resolved.expr(value) {
                    Some(Target::Members { members, .. }) => {
                        members.len().checked_sub(1)?
                    }
                    Some(Target::Local { site }) => {
                        let value = crate::limits::Bindings::of(self.module)
                            .site(*site)?;
                        return self.final_member_read(value, visited);
                    }
                    _ => return None,
                },
                _ => return None,
            };
            Some(crate::typed::MemberContractRead {
                span: self.span(value),
                step,
            })
        })();
        visited.pop();
        result
    }

    fn contract_formal(
        &self,
        binding: Span,
        argument: ExprId,
        visited: &mut Vec<ExprId>,
    ) -> Option<usize> {
        if visited.contains(&argument) {
            return None;
        }
        visited.push(argument);
        let result = (|| {
            let value =
                crate::limits::Bindings::of(self.module).site(binding)?;
            let ExprKind::Arms(arms) = &self.module.expressions[value.0].kind
            else {
                return None;
            };
            let [arm] = arms.as_slice() else {
                return None;
            };
            match &self.module.expressions[argument.0].kind {
                ExprKind::Group(parts) => parts.last().and_then(|value| self.contract_formal(binding, *value, visited)),
                ExprKind::Name(_) => match self.resolved.expr(argument) {
                    Some(Target::Local { site }) => {
                        arm.patterns.iter().position(|pattern| {
                            matches!(&self.module.patterns[pattern.0].kind,
                                crate::ast::PatternKind::Name(name) if name.span == *site)
                        }).or_else(|| {
                            crate::limits::Bindings::of(self.module).site(*site)
                                .and_then(|value| self.contract_formal(binding, value, visited))
                        })
                    }
                    _ => None,
                },
                _ => None,
            }
        })();
        visited.pop();
        result
    }

    fn refine_member_arguments(
        &mut self,
        binding: Option<&super::ContractBindingUse>,
        slots: &[super::Slot],
        filled: &[Option<Supply>],
        stage: Span,
    ) {
        if self.state.solving_dots {
            return;
        }
        let Some(binding) =
            binding.filter(|binding| binding.origin == Origin::This)
        else {
            return;
        };
        let contracts: Vec<_> = self
            .state
            .member_contracts
            .iter()
            .filter(|(key, _)| {
                (&key.origin, key.binding, key.at)
                    == (&binding.origin, binding.site, binding.at)
            })
            .map(|(key, contract)| (key.clone(), contract.clone()))
            .collect();
        for (key, contract) in contracts {
            self.refine_contract_arguments(
                binding, &key, &contract, slots, filled, stage,
            );
        }
    }
    fn refine_contract_arguments(
        &mut self,
        binding: &super::ContractBindingUse,
        key: &crate::typed::MemberContractUse,
        contract: &crate::typed::MemberContract,
        slots: &[super::Slot],
        filled: &[Option<Supply>],
        stage: Span,
    ) {
        for read in &contract.reads {
            let calls: Vec<_> = self
                .module
                .expressions
                .iter()
                .enumerate()
                .filter_map(|(index, expression)| {
                    let ExprKind::Call { callee, .. } = &expression.kind else {
                        return None;
                    };
                    (self.final_member_read(*callee, &mut Vec::new()).as_ref()
                        == Some(read))
                    .then_some(ExprId(index))
                })
                .collect();
            for call in calls {
                let body_stage = self.span(call);
                let Some(bound) = self
                    .state
                    .bound
                    .get(&(body_stage.start, body_stage.end))
                    .cloned()
                else {
                    continue;
                };
                for (spot, supply) in &bound.supplies {
                    let crate::typed::Supply::Expression(argument) = supply
                    else {
                        continue;
                    };
                    let Some(formal) = self.contract_formal(
                        binding.site,
                        *argument,
                        &mut Vec::new(),
                    ) else {
                        continue;
                    };
                    let mut expected = self.apply(&contract.callable);
                    for _ in 0..*spot {
                        let Type::Function { result, .. } = expected else {
                            break;
                        };
                        expected = *result;
                    }
                    let Type::Function { parameter, .. } = expected else {
                        continue;
                    };
                    let target = *parameter;
                    if !crate::limits::has_limits(&target) {
                        continue;
                    }
                    let Some(Some(Supply::Expr(value))) = filled.get(formal)
                    else {
                        continue;
                    };
                    let value = *value;
                    let refinement =
                        (key.clone(), stage, body_stage, read.step, *spot);
                    if self
                        .state
                        .contract_refinements
                        .get(&refinement)
                        .is_some_and(|saved| {
                            self.apply(saved) == self.apply(&target)
                        })
                    {
                        continue;
                    }
                    // Mark before existing expected-type retyping can revisit a source stage.
                    self.set_contract_refinement(refinement, target.clone());
                    let actual = self.state.types[value.0]
                        .as_ref()
                        .map(|ty| self.apply(ty));
                    if let (Some(actual), Some(slot)) =
                        (actual, slots.get(formal))
                    {
                        // Each use adds its own entry; the meaning Bound never chooses one contract over another.
                        self.enter(
                            &target,
                            &actual,
                            self.span(value),
                            stage,
                            slot.place_kind.clone(),
                        );
                    }
                }
            }
        }
    }
}
