use super::{form, home_origin, ir, Index, Scope, Source, Type, TypeName, Vec};

impl Index {
    pub(super) fn pattern(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::PatternId,
    ) -> ir::Pattern {
        use crate::ast::PatternKind as P;
        let node = &source.module.patterns[id.0];
        let kind = match &node.kind {
            P::Wildcard => ir::PatternKind::Any,
            P::Name(name) if !name.is_uppercase() => {
                ir::PatternKind::Slot(scope.slots[&name.span])
            }
            P::Default { name, .. } => {
                ir::PatternKind::Slot(scope.slots[&name.span])
            }
            P::Group(inner) => return self.pattern(source, scope, *inner),
            P::Literal(literal) => {
                let ty =
                    source.typed.pattern(id).expect("checked literal pattern");
                let site = (node.span.start, node.span.end);
                let reader = source.typed.selected.get(&site).map(|selected| {
                    self.selected(
                        source,
                        node.span,
                        selected,
                        source
                            .typed
                            .meaning_arguments
                            .get(&site)
                            .map_or(&[], Vec::as_slice),
                        source.typed.callee_types.get(&site).unwrap_or(ty),
                        None,
                    )
                });
                let value = self.node(
                    &source.home,
                    node.span,
                    ty,
                    ir::Kind::Literal {
                        form: form(literal),
                        reader,
                    },
                );
                let comparison = &source.typed.pattern_equals[&id.0];
                let equality = self.selected(
                    source,
                    node.span,
                    &comparison.selected,
                    &comparison.meanings,
                    &comparison.ty,
                    Some(value),
                );
                ir::PatternKind::Literal {
                    value,
                    equality,
                    self_spot: comparison.self_slot,
                }
            }
            P::Tuple(parts) => ir::PatternKind::Tuple(
                parts
                    .iter()
                    .map(|part| self.pattern(source, scope, *part))
                    .collect(),
            ),
            P::List(parts) => ir::PatternKind::List(
                parts
                    .iter()
                    .map(|part| self.pattern(source, scope, *part))
                    .collect(),
            ),
            P::Outcome { good, payload } => ir::PatternKind::Side {
                good: *good,
                payload: payload.map(|part| {
                    alloc::boxed::Box::new(self.pattern(source, scope, part))
                }),
            },
            P::Name(_) | P::Constructor { .. } => {
                self.named_pattern(source, scope, id)
            }
            P::Record { .. } => self.record_pattern(source, scope, id),
        };
        ir::Pattern {
            span: node.span,
            kind,
        }
    }

    pub(super) fn named_pattern(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::PatternId,
    ) -> ir::PatternKind {
        use crate::{ast::PatternKind, resolve::Target};
        let pattern = &source.module.patterns[id.0];
        let payload = match pattern.kind {
            PatternKind::Constructor { payload, .. } => Some(payload),
            _ => None,
        };
        let target =
            source.resolved.pattern(id).expect("checked pattern target");
        let (origin, owner, label) = match target {
            Target::Type { origin, name } => (origin, name, name),
            Target::Constructor { origin, ty, name } => (origin, ty, name),
            _ => unreachable!("named pattern target"),
        };
        let input = source.typed.pattern(id).expect("checked pattern input");
        if matches!(crate::infer::meaning(input), Type::Either(_)) {
            let meaning = self.meaning(
                &source.home,
                &TypeName {
                    origin: origin.clone(),
                    name: owner.clone(),
                },
            );
            let payload = payload.map_or_else(
                || ir::Pattern {
                    span: pattern.span,
                    kind: ir::PatternKind::Any,
                },
                |part| self.pattern(source, scope, part),
            );
            return ir::PatternKind::Tag {
                meaning,
                payload: alloc::boxed::Box::new(payload),
            };
        }
        if matches!(target, Target::Type { .. }) {
            let owner = TypeName {
                origin: home_origin(&source.home, origin),
                name: owner.clone(),
            };
            let predicate = self.predicate(
                &source.home,
                &Type::Named {
                    name: owner,
                    arguments: Vec::new(),
                },
                &mut Vec::new(),
            );
            let payload = payload.map_or_else(
                || ir::Pattern {
                    span: pattern.span,
                    kind: ir::PatternKind::Any,
                },
                |part| self.pattern(source, scope, part),
            );
            return ir::PatternKind::Gate {
                predicate,
                payload: alloc::boxed::Box::new(payload),
            };
        }
        let meaning = self.meaning(
            &source.home,
            &TypeName {
                origin: origin.clone(),
                name: owner.clone(),
            },
        );
        let constructor = self.constructor_index(meaning, label);
        let count = self.store.tables().meanings[meaning].constructors
            [constructor]
            .fields
            .len();
        let mut parts = alloc::vec![ir::Pattern { span: pattern.span, kind: ir::PatternKind::Any }; count];
        if let (Some(payload), Some(first)) = (payload, parts.first_mut()) {
            *first = self.pattern(source, scope, payload);
        }
        ir::PatternKind::Constructor {
            meaning,
            constructor,
            parts,
        }
    }

    pub(super) fn constructor_index(
        &self,
        meaning: usize,
        label: &str,
    ) -> usize {
        self.store.tables().meanings[meaning]
            .constructors
            .iter()
            .position(|constructor| constructor.label == label)
            .expect("checked constructor layout")
    }
    fn record_pattern(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::PatternId,
    ) -> ir::PatternKind {
        use crate::ast::PatternKind as P;
        match &source.module.patterns[id.0].kind {
            P::Record { fields, .. } => {
                let ty =
                    source.typed.pattern(id).expect("record pattern input");
                let ir::Type::Meaning { index: meaning, .. } =
                    self.ty(&source.home, ty)
                else {
                    unreachable!("record pattern has a nominal meaning")
                };
                let label = match source
                    .resolved
                    .pattern(id)
                    .expect("record pattern target")
                {
                    crate::resolve::Target::Constructor { name, .. } => {
                        name.clone()
                    }
                    _ => self.store.tables().meanings[meaning].label.clone(),
                };
                let constructor = self.constructor_index(meaning, &label);
                let layout = self.store.tables().meanings[meaning].constructors
                    [constructor]
                    .fields
                    .clone();
                let fields = fields
                    .iter()
                    .map(|field| {
                        let index = layout
                            .iter()
                            .position(|part| {
                                Some(&part.label) == field.name.segments.last()
                            })
                            .expect("checked record field");
                        let pattern = field.pattern.map_or_else(
                            || ir::Pattern {
                                span: field.name.span,
                                kind: ir::PatternKind::Slot(
                                    scope.slots[&field.name.span],
                                ),
                            },
                            |part| self.pattern(source, scope, part),
                        );
                        (index, pattern)
                    })
                    .collect();
                ir::PatternKind::Record {
                    meaning,
                    constructor,
                    fields,
                }
            }
            _ => unreachable!("record pattern"),
        }
    }
}
