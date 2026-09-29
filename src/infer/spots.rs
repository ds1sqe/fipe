//! Application: spots bind first, then APP ([evaluation.md] §Parameter
//! binding, steps 1 to 4). A callee with a spot list is a declared
//! function, a constructor, or a binding whose value is an arm list;
//! anything else applies positionally and curried.
//!
//! Chained Bool flags (`-la`) and a trailing `Vec` collecting operands
//! are 07's; they are reported as an unknown spot and an extra argument.
//!
//! [evaluation.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/evaluation.md

use super::{
    is_prelude,
    lower::{fill_family, has_placeholder, is_placeholder},
    prelude, Callee, Expected, Head, Infer, Key, Local, Reason, Slot, State,
};
use crate::{
    ast::{Argument, ArgumentKind, ExprId, ExprKind},
    resolve::{resolve_default, Origin, Target},
    token::Span,
    types::{Scheme, Type},
};

use alloc::{string::String, vec, vec::Vec};

/// What fills a spot.
#[derive(Clone)]
enum Supply {
    Expr(ExprId),
    /// A bare Bool flag.
    True,
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
                let callee = match self.resolved.expr(id) {
                    Some(target) => self.target(target, span),
                    None => Callee::plain(self.fresh()),
                };
                for (parameter, argument) in
                    callee.parameters.iter().zip(arguments)
                {
                    let ty = self.lower_here(*argument);
                    self.unify(parameter, &ty, span, span);
                }
                callee
            }
            ExprKind::Field { value, field } => {
                let head = self.expr(*value, None);
                self.members(head, &[super::last(field)], span)
            }
            _ => return Callee::plain(self.expr(id, None)),
        };
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
                self.constructor(origin, ty, name)
            }
            Target::SelfType => self.self_constructor(),
            Target::Members { head, members } => {
                let head = self.target(head, span);
                let value = self.apply_callee(head, &[], None, span, None);
                self.members(value, members, span)
            }
            Target::TypeMember { ty, member } => {
                self.type_member(ty, member, span)
            }
            Target::Word => Callee::plain(prelude("String")),
            Target::Type { .. }
            | Target::Trait { .. }
            | Target::Module(_)
            | Target::Variable(_) => {
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
                name: head.name.clone(),
                ty,
                default: head.default,
                value: None,
                site: Span::default(),
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

    /// Apply a callee to written arguments and an optional piped input.
    pub(super) fn apply_callee(
        &mut self,
        callee: Callee,
        arguments: &[Argument],
        pipe: Option<(Type, Span)>,
        stage: Span,
        expected: Option<&Type>,
    ) -> Type {
        if callee.spots.is_none() {
            let ty = self.curried(&callee.ty, arguments, pipe, stage);
            self.dots();
            return ty;
        }
        self.bind_spots(callee, arguments, pipe, stage, expected)
    }

    fn bind_spots(
        &mut self,
        callee: Callee,
        arguments: &[Argument],
        pipe: Option<(Type, Span)>,
        stage: Span,
        expected: Option<&Type>,
    ) -> Type {
        let mut slots = callee.spots.unwrap_or_default();
        let mut result = callee.ty;
        let mut filled: Vec<Option<Supply>> = vec![None; slots.len()];
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
        if slots.iter().any(|slot| has_placeholder(&slot.ty))
            || has_placeholder(&result)
        {
            self.family(&mut slots, &mut result, &mut filled, stage, expected);
        }
        self.supply(&slots, &filled, stage);
        if let Some(home) = &callee.home {
            for (index, supply) in filled.iter().enumerate() {
                if supply.is_none() && slots[index].value.is_some() {
                    self.declared_default(home, &slots, index, stage);
                }
            }
        }
        self.dots();
        // Step 4: a defaulted spot closes; the rest stay open in order.
        slots
            .iter()
            .zip(&filled)
            .rev()
            .filter(|(slot, supply)| supply.is_none() && !slot.default)
            .fold(result, |ty, (slot, _)| Type::function(slot.ty.clone(), ty))
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
            let supply = match (slot, value) {
                (None, _) => None,
                (Some(_), Some(value)) => Some(Supply::Expr(*value)),
                (Some(slot), None) if self.is_bool(&slots[slot].ty) => {
                    Some(Supply::True)
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
        let mut next = 0;
        for (index, argument) in arguments.iter().enumerate() {
            let ArgumentKind::Positional(value) = &argument.kind else {
                continue;
            };
            if consumed[index] {
                continue;
            }
            while next < slots.len()
                && (filled[next].is_some() || self.is_bool(&slots[next].ty))
            {
                next += 1;
            }
            if next < slots.len() {
                filled[next] = Some(Supply::Expr(*value));
            } else {
                self.error(argument.span, Reason::ExtraArgument);
                self.expr(*value, None);
            }
        }
    }

    /// Type every supply against its spot: typed values first, then
    /// operands, arm lists and selectors last so that their parameters
    /// meet known types.
    fn supply(
        &mut self,
        slots: &[Slot],
        filled: &[Option<Supply>],
        stage: Span,
    ) {
        for (slot, supply) in slots.iter().zip(filled) {
            match supply {
                Some(Supply::Typed(ty, span)) => {
                    self.unify(&slot.ty, ty, *span, stage);
                }
                Some(Supply::True) => {
                    self.unify(&slot.ty, &prelude("Bool"), stage, stage);
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
            for (slot, supply) in slots.iter().zip(filled) {
                if let Some(Supply::Expr(id)) = supply {
                    if late(id) == pass {
                        let expected = Expected {
                            ty: slot.ty.clone(),
                            span: stage,
                            spot: true,
                        };
                        self.expr(*id, Some(&expected));
                    }
                }
            }
        }
    }

    /// `Functor.map` and the other higher-kinded methods: the family is
    /// the outer name of the first receiver whose type is known, its
    /// last argument the element; else the expected result's.
    fn family(
        &mut self,
        slots: &mut [Slot],
        result: &mut Type,
        filled: &mut [Option<Supply>],
        stage: Span,
        expected: Option<&Type>,
    ) {
        let mut family = None;
        for (slot, supply) in slots.iter().zip(filled.iter_mut()) {
            if !is_placeholder(&slot.ty) || family.is_some() {
                continue;
            }
            let ty = match supply {
                Some(Supply::Typed(ty, _)) => ty.clone(),
                Some(Supply::Expr(id)) => {
                    let id = *id;
                    let ty = self.expr(id, None);
                    *supply = Some(Supply::Typed(ty.clone(), self.span(id)));
                    ty
                }
                _ => continue,
            };
            family = outer(&self.apply(&ty));
        }
        if family.is_none() && is_placeholder(result) {
            family = expected.and_then(|ty| outer(&self.apply(ty)));
        }
        let family = family.unwrap_or_else(|| {
            self.error(stage, Reason::UnknownFamily);
            (String::from("?"), Origin::Prelude, Vec::new())
        });
        for slot in slots.iter_mut() {
            slot.ty = fill_family(&slot.ty, &family);
        }
        *result = fill_family(result, &family);
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
        let count = operands.len() + usize::from(pipe.is_some());
        let Some((parameters, result)) = self.peel(ty, count) else {
            self.error(stage, Reason::NotAFunction);
            for operand in operands {
                self.expr(operand, None);
            }
            return self.fresh();
        };
        if let Some((ty, span)) = pipe {
            self.unify(&parameters[count - 1], &ty, span, stage);
        }
        for (operand, parameter) in operands.into_iter().zip(parameters) {
            let expected = Expected {
                ty: parameter,
                span: stage,
                spot: true,
            };
            self.expr(operand, Some(&expected));
        }
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
                callee, arguments, ..
            } => {
                let callee = self.callee(*callee);
                self.apply_callee(callee, arguments, pipe, span, expected)
            }
            ExprKind::Name(_)
            | ExprKind::Instance { .. }
            | ExprKind::Field { .. } => {
                let callee = self.callee(id);
                return self.apply_callee(callee, &[], pipe, span, expected);
            }
            _ => {
                let result = self.fresh();
                let function = Type::function(input, result.clone());
                self.expr(id, Some(&Expected::new(function, input_span)));
                return result;
            }
        };
        self.state.types[id.0] = Some(Type::function(input, result.clone()));
        result
    }

    pub(super) fn is_bool(&self, ty: &Type) -> bool {
        is_prelude(&self.apply(ty), "Bool")
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
        let (Some(module), Some(value)) =
            (self.owner(home), slots[index].value)
        else {
            return;
        };
        let earlier: Vec<_> = slots[..index]
            .iter()
            .filter_map(|slot| Some((slot.name.clone()?, slot.site, &slot.ty)))
            .collect();
        let names: Vec<_> = earlier
            .iter()
            .map(|(name, site, _)| (name.clone(), *site))
            .collect();
        let resolved =
            resolve_default(module, self.declarations, value, &names);
        let subst = core::mem::take(&mut self.state.subst);
        let mut inner = Infer {
            module,
            resolved: &resolved,
            declarations: self.declarations,
            files: self.files,
            frame: self.frame,
            visiting: self.visiting.clone(),
            weak: self.weak,
            state: State::new(subst, module.expressions.len()),
            items: Vec::new(),
            groups: Vec::new(),
            self_type: None,
        };
        for (_, site, ty) in earlier {
            let local = Local {
                scheme: Scheme::mono(ty.clone()),
                heads: None,
            };
            inner.state.locals.insert((site.start, site.end), local);
        }
        let expected = Expected {
            ty: slots[index].ty.clone(),
            span: stage,
            spot: true,
        };
        inner.expr(value, Some(&expected));
        self.state.subst = inner.state.subst;
        for mut owed in inner.state.obligations {
            owed.span = stage;
            self.state.obligations.push(owed);
        }
    }
}

/// A type's family and the arguments before its element.
fn outer(ty: &Type) -> Option<(String, Origin, Vec<Type>)> {
    match ty {
        Type::Named { name, arguments } if !arguments.is_empty() => Some((
            name.name.clone(),
            name.origin.clone(),
            arguments[..arguments.len() - 1].to_vec(),
        )),
        _ => None,
    }
}
