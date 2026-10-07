use super::{Callee, Infer, Key, Reason, Slot};
use crate::{resolve::Origin, token::Span, types::Type};
use alloc::{string::String, vec, vec::Vec};

impl Infer<'_> {
    pub(super) fn check_conversions(&mut self) {
        let mut seen: Vec<(String, Type)> = Vec::new();
        for index in 0..self.items.len() {
            let site = self.items[index].site;
            if !self.module.entries.iter().any(|e| matches!(e, crate::ast::Entry::Item(item) if matches!(item.kind,crate::ast::ItemKind::Impl { claim: Some(_), .. }) && item.span.start <= site.start && site.end <= item.span.end)) { continue; }
            let Key::Member(owner, member) = &self.items[index].key else {
                continue;
            };
            if !matches!(member.as_str(), "from" | "try_from") {
                continue;
            }
            let Type::Function { parameter, .. } =
                self.apply(&self.items[index].ty)
            else {
                continue;
            };
            let pair = (owner.clone(), super::meaning(&parameter).clone());
            let target = self.groups[self.items[index].group].self_type.clone();
            let mut prior = false;
            if let Some(target @ Type::Named { .. }) = target {
                let Type::Named { name, .. } = &target else {
                    unreachable!()
                };
                if matches!(name.origin, Origin::Line(_)) {
                    for member in ["from", "try_from"] {
                        for callee in self.conversion_members(
                            &name.origin,
                            &name.name,
                            &target,
                            member,
                            self.items[index].site,
                        ) {
                            if let Type::Function { parameter, .. } =
                                callee.whole()
                            {
                                prior |= super::meaning(&parameter) == &pair.1;
                            }
                        }
                    }
                }
            }
            if prior || seen.contains(&pair) {
                self.error(
                    self.items[index].site,
                    Reason::ConflictingConversion,
                );
            } else {
                seen.push(pair);
            }
        }
    }

    pub(super) fn conversion_callee(
        &mut self,
        target: Type,
        kind: &str,
    ) -> Callee {
        let result = if kind == "try_from" {
            super::prelude_of("Result", vec![target.clone(), self.fresh()])
        } else {
            target.clone()
        };
        let mut callee = Callee::plain(result);
        callee.spots = Some(vec![Slot {
            place_kind: crate::typed::Place::Parameter(None),
            name: None,
            ty: self.fresh(),
            default: false,
            value: None,
            site: Span::default(),
        }]);
        callee.conversion = Some((target, String::from(kind)));
        callee
    }

    pub(super) fn convert(
        &mut self,
        target: &Type,
        kind: &str,
        source: &Type,
        span: Span,
    ) -> Type {
        let source = self.conversion_source(source);
        let target = self.apply(target);
        let Type::Named { name, .. } = super::meaning(&target) else {
            self.error(
                span,
                Reason::Conversion {
                    source: Some(source),
                    target: target.clone(),
                    candidates: Vec::new(),
                },
            );
            return target;
        };
        if !matches!(kind, "from" | "try_from" | "to" | "try") {
            return match self.select_member(&target, kind, &source, span) {
                Type::Function { result, .. } => *result,
                other => other,
            };
        }
        if let Some(result) = limited_conversion(&target, &source, kind) {
            return result;
        }
        let mut candidates = Vec::new();
        for member in ["from", "try_from"] {
            if (kind == "from" || kind == "try_from") && member != kind {
                continue;
            }
            if matches!(name.origin, Origin::This | Origin::Line(_)) {
                let key = Key::Member(name.name.clone(), String::from(member));
                let indices: Vec<_> = self
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, item)| (item.key == key).then_some(i))
                    .collect();
                for index in indices {
                    let callee = self.item_callee(index, span);
                    candidates.push((member, callee.whole()));
                }
            }
            for callee in self.conversion_members(
                &name.origin,
                &name.name,
                &target,
                member,
                span,
            ) {
                candidates.push((member, callee.whole()));
            }
        }
        let mut selected = Vec::new();
        let mut sources = Vec::new();
        for (member, ty) in candidates {
            if let Type::Function { parameter, result } = self.apply(&ty) {
                let claimed = self.claims(&target, true).iter().any(|class| matches!(class, Type::Named { name, arguments } if name.origin == Origin::Prelude && name.name == (if member == "from" { "From" } else { "TryFrom" }) && arguments.first().is_some_and(|a| super::meaning(a) == super::meaning(&parameter))));
                if !claimed {
                    continue;
                }
                sources.push((*parameter).clone());
                if !matches!(source, Type::Var(_) | Type::Rigid(_))
                    && super::meaning(&parameter) == super::meaning(&source)
                {
                    selected.push((member, *result));
                }
            }
        }
        if self
            .choose_meaning(&source, &sources, &target, span)
            .is_none()
        {
            return self.fresh();
        }
        let [(member, result)] = selected.as_slice() else {
            return self.fresh();
        };
        if *member == "try_from" {
            if let Type::Named { name, arguments } = result {
                if name.origin == Origin::Prelude
                    && name.name == "Result"
                    && arguments.len() == 2
                {
                    self.unify(&target, &arguments[0], span, span);
                    return if kind == "to" { target } else { result.clone() };
                }
            }
            self.error(span, Reason::NotAFunction);
            self.fresh()
        } else {
            self.unify(&target, result, span, span);
            if kind == "try" {
                super::prelude_of(
                    "Result",
                    vec![target, super::prelude("LimitError")],
                )
            } else {
                target
            }
        }
    }
}

impl Infer<'_> {
    fn conversion_source(&mut self, source: &Type) -> Type {
        let source = self.apply(source);
        if matches!(source, Type::Var(_)) {
            let mut env = Vec::new();
            for owed in &self.state.obligations {
                let ty = self.apply(owed.obligation.subject());
                if ty != source {
                    ty.variables(&mut env);
                }
            }
            self.default_literals(0, &env, &[]);
        }
        self.apply(&source)
    }

    /// Overloads are selected by a known argument meaning in every spelling.
    pub(super) fn choose_meaning(
        &mut self,
        source: &Type,
        candidates: &[Type],
        target: &Type,
        span: Span,
    ) -> Option<usize> {
        let source = self.apply(source);
        let matching: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                (!matches!(source, Type::Var(_) | Type::Rigid(_))
                    && super::meaning(c) == super::meaning(&source))
                .then_some(i)
            })
            .collect();
        if let [index] = matching.as_slice() {
            Some(*index)
        } else {
            self.error(
                span,
                Reason::Conversion {
                    source: Some(source),
                    target: target.clone(),
                    candidates: candidates.to_vec(),
                },
            );
            None
        }
    }

    pub(super) fn member_choices(
        &mut self,
        head: &Type,
        member: &str,
    ) -> Vec<Type> {
        let mut choices = Vec::new();
        for class in self.claims(head, true) {
            for required in self.member_limits(
                &class,
                head,
                Span::default(),
                &mut Vec::new(),
            ) {
                if let crate::types::Obligation::Member { name, ty, .. } =
                    required
                {
                    if name == member && !choices.contains(&ty) {
                        choices.push(ty);
                    }
                }
            }
        }
        choices
    }

    pub(super) fn overloaded_member(
        &mut self,
        head: &Type,
        member: &str,
    ) -> Option<Callee> {
        if self.member_choices(head, member).len() < 2 {
            return None;
        }
        let mut self_slot = 1;
        if let Type::Named { name: owner, .. } = head {
            'blocks: for (module, _) in
                self.impl_scopes(&owner.origin, &owner.name)
            {
                for members in super::lower::impls(module, &owner.name) {
                    for entry in members {
                        if let crate::ast::ImplMember::Named {
                            name,
                            value: Some(value),
                            ..
                        } = entry
                        {
                            if super::last(name) == member {
                                if let crate::ast::ExprKind::Arms(arms) =
                                    &module.expressions[value.0].kind
                                {
                                    self_slot =
                                        super::arms::heads_in(module, arms)
                                            .iter()
                                            .position(|h| {
                                                h.name.as_deref()
                                                    == Some("self")
                                            })
                                            .unwrap_or(1);
                                    break 'blocks;
                                }
                            }
                        }
                    }
                }
            }
        }
        let argument = self.fresh();
        let mut callee = Callee::plain(self.fresh());
        callee.spots = Some(
            (0..2)
                .map(|index| Slot {
                    place_kind: crate::typed::Place::Parameter(
                        (index == self_slot).then(|| String::from("self")),
                    ),
                    name: (index == self_slot).then(|| String::from("self")),
                    ty: if index == self_slot {
                        head.clone()
                    } else {
                        argument.clone()
                    },
                    default: false,
                    value: None,
                    site: Span::default(),
                })
                .collect(),
        );
        callee.self_slot = Some(self_slot);
        callee.conversion = Some((head.clone(), String::from(member)));
        Some(callee)
    }

    pub(super) fn select_member(
        &mut self,
        head: &Type,
        member: &str,
        source: &Type,
        span: Span,
    ) -> Type {
        let choices = self.member_choices(head, member);
        let sources: Vec<_> = choices
            .iter()
            .filter_map(|ty| match ty {
                Type::Function { parameter, .. } => Some((**parameter).clone()),
                _ => None,
            })
            .collect();
        self.choose_meaning(source, &sources, head, span)
            .map_or_else(|| self.fresh(), |i| choices[i].clone())
    }
}

fn limited_conversion(
    target: &Type,
    source: &Type,
    kind: &str,
) -> Option<Type> {
    if matches!(kind, "to" | "try") && matches!(target, Type::Limited { .. }) {
        let target_meaning = super::meaning(target);
        let source_meaning = super::meaning(source);
        let numeric = |ty: &Type| matches!(ty, Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "Integer" | "F32" | "F64"));
        if target_meaning == source_meaning
            || (numeric(target_meaning) && numeric(source_meaning))
        {
            return Some(if kind == "try" {
                super::prelude_of(
                    "Result",
                    vec![target.clone(), super::prelude("LimitError")],
                )
            } else {
                target.clone()
            });
        }
    }
    None
}
