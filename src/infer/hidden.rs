//! Hidden meanings follow actual receiverless operations and references.
use super::Infer;
use crate::{
    ast::{ExprKind, PatternKind},
    resolve::Origin,
    token::Span,
    typed::Selected,
    types::{Scheme, Type},
};
use alloc::{collections::BTreeMap, vec::Vec};

type Site = (usize, usize);

impl Infer<'_> {
    pub(super) fn record_hidden(
        &self,
        schemes: &BTreeMap<Site, Scheme>,
    ) -> BTreeMap<Site, Vec<usize>> {
        let mut hidden: BTreeMap<_, _> = schemes
            .iter()
            .map(|(site, scheme)| {
                let mut variables = Vec::new();
                for (ty, _, _) in &scheme.literals {
                    ty.variables(&mut variables);
                }
                (
                    *site,
                    scheme
                        .variables
                        .iter()
                        .enumerate()
                        .filter_map(|(index, var)| {
                            variables.contains(var).then_some(index)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        // Each body is found once, and a pass reads the indices this pass
        // already added, so a chain of references settles in one pass
        // in source order; the loop ends when a pass adds nothing.
        let bodies = self.binding_bodies();
        let reads = self.read_casts();
        loop {
            let mut changed = false;
            for (site, scheme) in schemes {
                let Some(span) = bodies.get(site).copied() else {
                    continue;
                };
                let parameters = self
                    .state
                    .body_parameters
                    .get(site)
                    .cloned()
                    .unwrap_or_else(|| {
                        scheme
                            .variables
                            .iter()
                            .map(|var| Type::Var(*var))
                            .collect()
                    });
                let added: Vec<usize> = parameters
                    .iter()
                    .enumerate()
                    .filter(|(index, parameter)| {
                        !hidden[site].contains(index)
                            && self.hidden_in(span, parameter, &reads, &hidden)
                    })
                    .map(|(index, _)| index)
                    .collect();
                if !added.is_empty() {
                    changed = true;
                    let indices =
                        hidden.get_mut(site).expect("binding indexed");
                    indices.extend(added);
                    indices.sort_unstable();
                }
            }
            if !changed {
                return hidden;
            }
        }
    }

    /// The body of each name binding and item, by its site; a binding
    /// written in the module before an item of the same site.
    fn binding_bodies(&self) -> BTreeMap<Site, Span> {
        let mut bodies = BTreeMap::new();
        for expression in &self.module.expressions {
            if let ExprKind::Bind { pattern, value, .. } = expression.kind {
                let pattern = &self.module.patterns[pattern.0];
                if matches!(pattern.kind, PatternKind::Name(_)) {
                    bodies
                        .entry((pattern.span.start, pattern.span.end))
                        .or_insert(self.module.expressions[value.0].span);
                }
            }
        }
        for item in &self.items {
            let span = match item.body.as_ref() {
                Some(super::Body::Value(value)) => {
                    self.module.expressions[value.0].span
                }
                Some(super::Body::Arms(arms)) => {
                    let (Some(first), Some(last)) = (arms.first(), arms.last())
                    else {
                        continue;
                    };
                    Span {
                        start: first.span.start,
                        end: last.span.end,
                    }
                }
                None => continue,
            };
            bodies
                .entry((item.site.start, item.site.end))
                .or_insert(span);
        }
        bodies
    }

    /// The read casts with their expression's span, by start.
    fn read_casts(&self) -> Vec<(Span, &Type)> {
        let mut reads: Vec<_> = self
            .state
            .casts
            .iter()
            .enumerate()
            .filter_map(|(index, cast)| {
                let cast = cast.as_ref()?;
                (cast.row == crate::typed::CastRow::Read)
                    .then(|| (self.module.expressions[index].span, &cast.to))
            })
            .collect();
        reads.sort_by_key(|(span, _)| span.start);
        reads
    }

    /// Whether `parameter` is hidden in the body at `span`: read by a
    /// cast or selected by an operation inside it.
    fn hidden_in(
        &self,
        span: Span,
        parameter: &Type,
        reads: &[(Span, &Type)],
        hidden: &BTreeMap<Site, Vec<usize>>,
    ) -> bool {
        let contains =
            |ty: &Type| super::dot::contains_type(&self.apply(ty), parameter);
        let first = reads.partition_point(|(site, _)| site.start < span.start);
        reads[first..]
            .iter()
            .take_while(|(site, _)| site.start <= span.end)
            .any(|(site, to)| site.end <= span.end && contains(to))
            || self
                .state
                .selected
                .range((span.start, 0)..=(span.end, usize::MAX))
                .any(|(site, selected)| {
                    if site.1 > span.end {
                        return false;
                    }
                    match selected {
                        Selected::Static { head, .. } => contains(head),
                        Selected::Derived {
                            ty,
                            kind: crate::ir::Derived::Default,
                        } => contains(ty),
                        Selected::Declared { .. } => {
                            let Some(ty) = self.state.callee_types.get(site)
                            else {
                                return false;
                            };
                            let mut result = ty;
                            while let Type::Function {
                                parameter: input,
                                result: output,
                            } = result
                            {
                                if contains(input) {
                                    return false;
                                }
                                result = output;
                            }
                            contains(result)
                        }
                        Selected::Value {
                            origin,
                            site: definition,
                        } => {
                            let positions = self.reference_hidden(
                                origin,
                                *definition,
                                hidden,
                            );
                            self.state.meaning_arguments.get(site).is_some_and(
                                |arguments| {
                                    positions.iter().any(|index| {
                                        arguments
                                            .get(*index)
                                            .is_some_and(contains)
                                    })
                                },
                            )
                        }
                        _ => false,
                    }
                })
    }

    fn reference_hidden(
        &self,
        origin: &Origin,
        site: Span,
        local: &BTreeMap<Site, Vec<usize>>,
    ) -> Vec<usize> {
        let key = (site.start, site.end);
        match origin {
            Origin::This => local.get(&key).cloned().unwrap_or_default(),
            Origin::Line(index) => self
                .frame
                .and_then(|frame| frame.lines.get(*index))
                .and_then(|line| line.typed.hidden.get(&key))
                .cloned()
                .unwrap_or_default(),
            Origin::File(stem) => self
                .file_cache
                .files
                .borrow()
                .get(stem)
                .and_then(|file| file.typed.hidden.get(&key))
                .cloned()
                .unwrap_or_default(),
            _ => self
                .file_cache
                .expressions
                .borrow()
                .values()
                .find_map(|expression| {
                    (expression.origin == *origin)
                        .then(|| expression.typed.hidden.get(&key).cloned())
                        .flatten()
                })
                .unwrap_or_default(),
        }
    }
}
