use super::{Callee, Infer, Key, Reason, Slot};
use crate::{resolve::Origin, token::Span, types::Type};
use alloc::{string::String, vec, vec::Vec};

impl Infer<'_> {
    pub(super) fn check_conversions(&mut self) {
        let mut seen: Vec<(String, Type)> = Vec::new();
        for index in 0..self.items.len() {
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
        let source = self.apply(&source);
        let target = self.apply(target);
        let Type::Named { name, .. } = super::meaning(&target) else {
            self.error(
                span,
                Reason::Conversion {
                    target: target.clone(),
                    candidates: Vec::new(),
                },
            );
            return target;
        };
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
                sources.push((*parameter).clone());
                if !matches!(source, Type::Var(_) | Type::Rigid(_))
                    && super::meaning(&parameter) == super::meaning(&source)
                {
                    selected.push((member, *result));
                }
            }
        }
        let [(member, result)] = selected.as_slice() else {
            self.error(
                span,
                Reason::Conversion {
                    target: target.clone(),
                    candidates: sources,
                },
            );
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
