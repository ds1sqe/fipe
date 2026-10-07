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
        loop {
            let previous = hidden.clone();
            for (site, scheme) in schemes {
                let Some(span) = self.binding_body(*site) else {
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
                for (index, parameter) in parameters.iter().enumerate() {
                    if self.hidden_in(span, parameter, &previous)
                        && !hidden[site].contains(&index)
                    {
                        hidden
                            .get_mut(site)
                            .expect("binding indexed")
                            .push(index);
                    }
                }
                hidden
                    .get_mut(site)
                    .expect("binding indexed")
                    .sort_unstable();
            }
            if hidden == previous {
                return hidden;
            }
        }
    }

    fn binding_body(&self, site: Site) -> Option<Span> {
        for expression in &self.module.expressions {
            if let ExprKind::Bind { pattern, value, .. } = expression.kind {
                let pattern = &self.module.patterns[pattern.0];
                if matches!(pattern.kind, PatternKind::Name(_))
                    && (pattern.span.start, pattern.span.end) == site
                {
                    return Some(self.module.expressions[value.0].span);
                }
            }
        }
        self.items.iter().find_map(|item| {
            if (item.site.start, item.site.end) != site {
                return None;
            }
            match item.body.as_ref()? {
                super::Body::Value(value) => {
                    Some(self.module.expressions[value.0].span)
                }
                super::Body::Arms(arms) => Some(Span {
                    start: arms.first()?.span.start,
                    end: arms.last()?.span.end,
                }),
            }
        })
    }

    fn hidden_in(
        &self,
        span: Span,
        parameter: &Type,
        hidden: &BTreeMap<Site, Vec<usize>>,
    ) -> bool {
        let contains =
            |ty: &Type| super::dot::contains_type(&self.apply(ty), parameter);
        self.state.casts.iter().enumerate().any(|(index, cast)| {
            let site = self.module.expressions[index].span;
            span.start <= site.start
                && site.end <= span.end
                && cast.as_ref().is_some_and(|cast| {
                    cast.row == crate::typed::CastRow::Read
                        && contains(&cast.to)
                })
        }) || self.state.selected.iter().any(|(site, selected)| {
            if site.0 < span.start || site.1 > span.end {
                return false;
            }
            match selected {
                Selected::Static { head, .. } => contains(head),
                Selected::Derived {
                    ty,
                    kind: crate::ir::Derived::Default,
                } => contains(ty),
                Selected::Declared { .. } => {
                    let Some(ty) = self.state.callee_types.get(site) else {
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
                    let positions =
                        self.reference_hidden(origin, *definition, hidden);
                    self.state.meaning_arguments.get(site).is_some_and(
                        |arguments| {
                            positions.iter().any(|index| {
                                arguments.get(*index).is_some_and(contains)
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
