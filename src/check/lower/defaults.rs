use super::{home_origin, ir, owner, Index, Origin, Rc, Source, Type, Vec};
use crate::{
    token::Span,
    typed::{Bound, DeclaredExpression, DefaultValue},
};

impl Index {
    pub(super) fn defaults(
        &mut self,
        source: &Source<'_>,
        bound: &Bound,
        slots: &[usize],
        span: Span,
    ) -> Vec<(usize, usize)> {
        bound
            .defaults
            .iter()
            .map(|(spot, value)| {
                let earlier: Vec<_> = bound
                    .spots
                    .iter()
                    .zip(slots)
                    .enumerate()
                    .filter(|(index, _)| {
                        source
                            .typed
                            .constructor_fields
                            .get(&(span.start, span.end))
                            .map_or(*index < *spot, |fields| {
                                fields[*index] < fields[*spot]
                            })
                    })
                    .map(|(_, ((site, _), slot))| (*site, *slot))
                    .collect();
                let node =
                    self.default_node(source, span, value, &earlier, *spot);
                (*spot, self.constant_default(node))
            })
            .collect()
    }

    pub(super) fn default_node(
        &mut self,
        source: &Source<'_>,
        span: Span,
        value: &DefaultValue,
        earlier: &[(Span, usize)],
        spot: usize,
    ) -> usize {
        match value {
            DefaultValue::Prepared {
                selected,
                meanings,
                ty,
            } => self.selected(source, span, selected, meanings, ty, None),
            DefaultValue::Declared { .. } => {
                let recorded = &source.typed.default_instances
                    [&(span.start, span.end, spot)];
                self.default_expression(source, recorded, earlier)
            }
            DefaultValue::Expression(recorded) => {
                self.default_expression(source, recorded, earlier)
            }
            DefaultValue::Record { ty, fields } => {
                self.default_record(source, span, ty, fields)
            }
            DefaultValue::Array { ty, element } => {
                let Type::Array { length, .. } = ty else {
                    unreachable!("array default")
                };
                let length = self.ty(&source.home, length);
                let value = self.default_node(source, span, element, &[], 0);
                self.node(
                    &source.home,
                    span,
                    ty,
                    ir::Kind::Repeat { value, length },
                )
            }
            DefaultValue::Type(ty) => {
                let meaning = self.ty(&source.home, ty);
                self.node(
                    &source.home,
                    span,
                    ty,
                    ir::Kind::Derived {
                        kind: ir::Derived::Default,
                        ty: meaning,
                    },
                )
            }
        }
    }

    fn default_record(
        &mut self,
        source: &Source<'_>,
        span: Span,
        ty: &Type,
        fields: &[(Span, Type, DefaultValue)],
    ) -> usize {
        let mut slots = Vec::new();
        for (_, ty, _) in fields {
            let ty = self.ty(&source.home, ty);
            let slot = self.store.push_slot(ir::Slot {
                ty,
                parameters: Vec::new(),
                hidden: Vec::new(),
                definition: None,
            });
            slots.push(slot);
        }
        let ir::Type::Meaning { index: meaning, .. } =
            self.ty(&source.home, ty)
        else {
            unreachable!("record default")
        };
        let mut defaults = Vec::new();
        for (spot, (_, _, value)) in fields.iter().enumerate() {
            let earlier: Vec<_> = fields
                .iter()
                .zip(&slots)
                .take(spot)
                .map(|((site, _, _), slot)| (*site, *slot))
                .collect();
            let node = self.default_node(source, span, value, &earlier, spot);
            defaults.push((spot, self.constant_default(node)));
        }
        let signature = fields
            .iter()
            .rev()
            .fold(ty.clone(), |result, (_, field, _)| {
                Type::function(field.clone(), result)
            });
        let callee = self.node(
            &source.home,
            span,
            &signature,
            ir::Kind::Constructor {
                meaning,
                constructor: 0,
                fields: (0..fields.len()).collect(),
            },
        );
        self.node(
            &source.home,
            span,
            ty,
            ir::Kind::Call {
                callee,
                slots,
                supplies: Vec::new(),
                defaults,
                open: Vec::new(),
            },
        )
    }

    fn default_expression(
        &mut self,
        source: &Source<'_>,
        recorded: &DeclaredExpression,
        earlier: &[(Span, usize)],
    ) -> usize {
        let origin = &recorded.origin;
        let module = if *origin == Origin::This {
            source.module.clone()
        } else {
            self.store
                .source_modules()
                .get(origin)
                .cloned()
                .unwrap_or_else(|| {
                    Rc::new(
                        owner(source.declarations, source.files, origin)
                            .clone(),
                    )
                })
        };
        let instance = Source {
            module,
            declarations: source.declarations,
            files: source.files,
            home: home_origin(&source.home, origin),
            resolved: recorded.resolved.clone(),
            typed: recorded.typed.clone(),
            roots: alloc::vec![recorded.value],
        };
        let mut scope = self.reserve(&instance);
        scope.slots.extend(earlier.iter().copied());
        let bodies = self.definitions(&instance, &scope);
        self.fill(&instance, &scope, &bodies);
        self.captures(&scope);
        self.entries(&instance, &scope, &mut Vec::new());
        scope.expression(recorded.value)
    }
}
