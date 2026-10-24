//! Hidden meanings follow actual runtime type reads and references.
use super::Infer;
use crate::{
    ast::{ExprKind, PatternKind},
    resolve::Origin,
    token::Span,
    typed::{DeclaredExpression, DefaultValue, Selected, Typed},
    types::{Scheme, Type},
};
use alloc::{collections::BTreeMap, vec::Vec};

type Site = (usize, usize);
type ReadKey = (usize, Origin, Origin);

/// The same lexical distinction as lowering's contextual type input: a base's
/// formal belongs to its declaration, while a bound actual keeps its caller.
#[derive(Clone, Copy)]
struct PredicateInput<'a> {
    ty: &'a Type,
    home: &'a Origin,
    parameters_home: &'a Origin,
    bindings: &'a [(crate::types::Var, Self)],
}
impl<'a> PredicateInput<'a> {
    const fn child(self, ty: &'a Type) -> Self {
        Self { ty, ..self }
    }
    fn bound(self) -> Option<Self> {
        let Type::Var(var) = self.ty else { return None };
        self.bindings
            .iter()
            .find_map(|(formal, actual)| (formal == var).then_some(*actual))
    }
}
#[derive(Clone, Copy)]
enum PredicateMode {
    Entry,
    Arrows,
    Run,
}
struct PredicateRead {
    site: Span,
    home: Origin,
    parameter: Type,
}
#[derive(Default)]
struct PredicateCatalog {
    shapes: BTreeMap<crate::types::TypeName, (Origin, crate::typed::Shape)>,
    tests: alloc::collections::BTreeSet<crate::types::TypeName>,
}
struct SourceReads<'a> {
    ordinary: Vec<(Span, &'a Type)>,
    predicates: Vec<PredicateRead>,
}
fn effective_name(
    home: &Origin,
    name: &crate::types::TypeName,
) -> crate::types::TypeName {
    crate::types::TypeName {
        origin: if name.origin == Origin::This {
            home.clone()
        } else {
            name.origin.clone()
        },
        name: name.name.clone(),
    }
}

/// Project only Parameter syntax that a lowered Test argument materializes.
/// Limited arguments lower their meaning, whereas a predicate follows its name.
fn test_parameters(
    input: PredicateInput<'_>,
    site: Span,
    out: &mut Vec<PredicateRead>,
) {
    if let Some(actual) = input.bound() {
        test_parameters(actual, site, out);
        return;
    }
    match input.ty {
        Type::Var(_) | Type::Rigid(_) => out.push(PredicateRead {
            site,
            home: input.parameters_home.clone(),
            parameter: input.ty.clone(),
        }),
        Type::Named { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Either(arguments)
        | Type::Both(arguments)
        | Type::Sum(arguments) => {
            for part in arguments {
                test_parameters(input.child(part), site, out);
            }
        }
        Type::Function { parameter, result } => {
            test_parameters(input.child(parameter), site, out);
            test_parameters(input.child(result), site, out);
        }
        Type::Array { element, length } => {
            test_parameters(input.child(element), site, out);
            test_parameters(input.child(length), site, out);
        }
        Type::Limited { meaning, .. } => {
            test_parameters(input.child(meaning), site, out);
        }
        Type::Not(inner) => test_parameters(input.child(inner), site, out),
        Type::Unit | Type::Number(_) => (),
    }
}
fn predicate_parameters<'a>(
    input: PredicateInput<'a>,
    mode: PredicateMode,
    site: Span,
    catalog: &'a PredicateCatalog,
    visiting: &mut Vec<crate::types::TypeName>,
    out: &mut Vec<PredicateRead>,
) {
    if let Some(actual) = input.bound() {
        predicate_parameters(actual, mode, site, catalog, visiting, out);
        return;
    }
    if let Type::Limited { name, .. } = input.ty {
        predicate_parameters(
            input.child(name),
            mode,
            site,
            catalog,
            visiting,
            out,
        );
        return;
    }
    if matches!(mode, PredicateMode::Entry | PredicateMode::Arrows) {
        if let Type::Function { parameter, result } = input.ty {
            predicate_parameters(
                input.child(parameter),
                PredicateMode::Entry,
                site,
                catalog,
                visiting,
                out,
            );
            predicate_parameters(
                input.child(result),
                PredicateMode::Arrows,
                site,
                catalog,
                visiting,
                out,
            );
            return;
        }
        if matches!(mode, PredicateMode::Arrows) {
            return;
        }
    }
    match input.ty {
        Type::Both(parts) | Type::Either(parts) => {
            for part in parts {
                predicate_parameters(
                    input.child(part),
                    PredicateMode::Run,
                    site,
                    catalog,
                    visiting,
                    out,
                );
            }
        }
        Type::Not(inner) => predicate_parameters(
            input.child(inner),
            PredicateMode::Run,
            site,
            catalog,
            visiting,
            out,
        ),
        Type::Named { .. } => {
            named_predicate_parameters(input, site, catalog, visiting, out);
        }
        // Run treats a Function as true; record fields are not predicates.
        _ => (),
    }
}

fn named_predicate_parameters<'a>(
    input: PredicateInput<'a>,
    site: Span,
    catalog: &'a PredicateCatalog,
    visiting: &mut Vec<crate::types::TypeName>,
    out: &mut Vec<PredicateRead>,
) {
    let Type::Named { name, arguments } = input.ty else {
        unreachable!("named predicate input")
    };
    let name = effective_name(input.home, name);
    if visiting.contains(&name) {
        return;
    }
    visiting.push(name.clone());
    if let Some((home, shape)) = catalog.shapes.get(&name) {
        let Type::Named {
            arguments: formals, ..
        } = &shape.ty
        else {
            unreachable!("named shape")
        };
        let bindings: Vec<_> = formals
            .iter()
            .zip(arguments)
            .filter_map(|(formal, actual)| {
                let Type::Var(var) = formal else { return None };
                Some((*var, input.child(actual)))
            })
            .collect();
        for base in &shape.bases {
            predicate_parameters(
                PredicateInput {
                    ty: base,
                    home,
                    parameters_home: home,
                    bindings: &bindings,
                },
                PredicateMode::Run,
                site,
                catalog,
                visiting,
                out,
            );
        }
    }
    if catalog.tests.contains(&name) {
        for argument in arguments {
            test_parameters(input.child(argument), site, out);
        }
    }
    visiting.pop();
}

/// Executable snapshots, with the receiving site in their containing source.
fn default_edges(typed: &Typed) -> Vec<(Site, &DeclaredExpression)> {
    let mut edges: Vec<_> = typed
        .default_instances
        .iter()
        .map(|((start, end, _), instance)| ((*start, *end), instance))
        .collect();
    let mut pending: Vec<_> = typed
        .derived_defaults
        .iter()
        .map(|(site, value)| (*site, value))
        .collect();
    for (site, bound) in typed.bound.iter().chain(
        typed.accesses.iter().flat_map(|(site, steps)| {
            steps.iter().filter_map(move |step| {
                step.bound.as_ref().map(|bound| (site, bound))
            })
        }),
    ) {
        pending.extend(bound.defaults.iter().map(|(_, value)| (*site, value)));
    }
    while let Some((site, value)) = pending.pop() {
        match value {
            DefaultValue::Expression(instance) => edges.push((site, instance)),
            DefaultValue::Record { fields, .. } => {
                pending
                    .extend(fields.iter().map(|(_, _, value)| (site, value)));
            }
            DefaultValue::Array { element, .. } => {
                pending.push((site, element));
            }
            _ => (),
        }
    }
    edges
}

fn default_root_key(
    instance: &DeclaredExpression,
    home: &Origin,
) -> (usize, Origin, usize) {
    (
        alloc::rc::Rc::as_ptr(&instance.typed) as usize,
        home.clone(),
        instance.value.0,
    )
}

impl Infer<'_> {
    pub(super) fn record_hidden(
        &self,
        schemes: &BTreeMap<Site, Scheme>,
        decisions: &Typed,
    ) -> BTreeMap<Site, Vec<usize>> {
        self.record_hidden_seed(schemes, decisions, &BTreeMap::new())
    }
    pub(super) fn record_hidden_seed(
        &self,
        schemes: &BTreeMap<Site, Scheme>,
        decisions: &Typed,
        seed: &BTreeMap<Site, Vec<usize>>,
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
        for (site, indices) in seed {
            if let Some(known) = hidden.get_mut(site) {
                for index in indices {
                    if !known.contains(index) {
                        known.push(*index);
                    }
                }
                known.sort_unstable();
            }
        }
        // Each body is found once, and a pass reads the indices this pass
        // already added, so a chain of references settles in one pass
        // in source order; the loop ends when a pass adds nothing.
        let bodies = self.binding_bodies();
        let reads = self.source_reads(decisions);
        loop {
            let mut changed = false;
            for (site, scheme) in schemes {
                let Some(span) = bodies.get(site).copied() else {
                    continue;
                };
                let parameters = decisions
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
                            && self.hidden_in(
                                span, parameter, &reads, &hidden, decisions,
                            )
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

    /// Types read by casts, with their expression's span, by start.
    fn read_casts_at<'a>(
        module: &crate::ast::Module,
        decisions: &'a Typed,
    ) -> Vec<(Span, &'a Type)> {
        let mut reads: Vec<_> = decisions
            .casts
            .iter()
            .enumerate()
            .flat_map(|(index, cast)| {
                let Some(cast) = cast.as_ref() else {
                    return [None, None];
                };
                let span = module
                    .expressions
                    .get(index)
                    .expect("cast belongs to its declared source")
                    .span;
                match cast.row {
                    crate::typed::CastRow::Read => {
                        [Some((span, &cast.to)), None]
                    }
                    crate::typed::CastRow::Pack => {
                        [Some((span, &cast.from)), Some((span, &cast.to))]
                    }
                    _ => [None, None],
                }
            })
            .flatten()
            .collect();
        reads.sort_by_key(|(span, _)| span.start);
        reads
    }

    /// Checked layouts and actual registered test owners, not source predicates
    /// guessed from a type name. Each snapshot retains its effective home.
    fn predicate_catalog(&self, decisions: &Typed) -> PredicateCatalog {
        let home = self.declared_home.clone().unwrap_or(Origin::This);
        let files = self.file_cache.files.borrow();
        let mut pending = alloc::vec![(decisions, home)];
        pending.extend(files.iter().map(|(stem, file)| {
            (file.typed.as_ref(), Origin::File(stem.clone()))
        }));
        if let Some(frame) = self.frame {
            pending.extend(
                frame
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| (&line.typed, Origin::Line(index))),
            );
        }
        let mut seen = alloc::collections::BTreeSet::new();
        let mut catalog = PredicateCatalog::default();
        while let Some((typed, home)) = pending.pop() {
            if !seen.insert((core::ptr::from_ref(typed) as usize, home.clone()))
            {
                continue;
            }
            for shape in &typed.shapes {
                if let Type::Named { name, .. } = &shape.ty {
                    catalog
                        .shapes
                        .entry(effective_name(&home, name))
                        .or_insert_with(|| (home.clone(), shape.clone()));
                }
            }
            for test in &typed.tests {
                if let Type::Named { name, .. } = &test.owner {
                    catalog.tests.insert(effective_name(&home, name));
                }
            }
            pending.extend(default_edges(typed).into_iter().map(
                |(_, instance)| {
                    let child = if instance.origin == Origin::This {
                        home.clone()
                    } else {
                        instance.origin.clone()
                    };
                    (instance.typed.as_ref(), child)
                },
            ));
            pending.extend(typed.declared.iter().map(|instance| {
                let child = if instance.origin == Origin::This {
                    home.clone()
                } else {
                    instance.origin.clone()
                };
                (instance.typed.as_ref(), child)
            }));
            pending.extend(typed.constructors.iter().map(|source| {
                let child = if source.home == Origin::This {
                    home.clone()
                } else {
                    source.home.clone()
                };
                (source.typed.as_ref(), child)
            }));
        }
        catalog
    }

    fn predicate_reads_at(
        module: &crate::ast::Module,
        resolved: &crate::resolve::Resolved,
        typed: &Typed,
        home: &Origin,
        parameters_home: &Origin,
        catalog: &PredicateCatalog,
    ) -> Vec<PredicateRead> {
        let mut reads = Vec::new();
        let mut collect = |ty: &Type, site, mode| {
            predicate_parameters(
                PredicateInput {
                    ty,
                    home,
                    parameters_home,
                    bindings: &[],
                },
                mode,
                site,
                catalog,
                &mut Vec::new(),
                &mut reads,
            );
        };
        for check in &typed.checks {
            if check.role == crate::limits::CheckRole::CallerReport
                || check.gate
                || check.place_origin != Origin::This
            {
                continue;
            }
            let Some(target) = &check.target else {
                continue;
            };
            // Match gates::entries' producing expression, not a diagnostic span
            // or an imported check's foreign expression index.
            for (index, expression) in module.expressions.iter().enumerate() {
                let id = crate::ast::ExprId(index);
                let Some(actual) = typed.expr(id) else {
                    continue;
                };
                if !check
                    .value
                    .map_or(expression.span == check.place, |value| value == id)
                {
                    continue;
                }
                let mode = if check.path.is_empty()
                    || matches!(
                        super::meaning(actual),
                        Type::Function { .. } | Type::Var(_)
                    ) {
                    PredicateMode::Entry
                } else {
                    PredicateMode::Run
                };
                collect(target, expression.span, mode);
            }
        }
        for (site, conversion) in &typed.conversions {
            let target = match conversion.mode {
                crate::ir::ConversionMode::Check
                | crate::ir::ConversionMode::TryCheck => &conversion.to,
                _ => &conversion.parameter,
            };
            collect(
                target,
                Span {
                    start: site.0,
                    end: site.1,
                },
                PredicateMode::Run,
            );
        }
        for (index, pattern) in module.patterns.iter().enumerate() {
            let id = crate::ast::PatternId(index);
            let Some(input) = typed.pattern(id) else {
                continue;
            };
            if matches!(super::meaning(input), Type::Either(_)) {
                continue;
            }
            let Some(crate::resolve::Target::Type { origin, name }) =
                resolved.pattern(id)
            else {
                continue;
            };
            // named_pattern deliberately lowers this gate with no actual args.
            let owner = Type::named(origin.clone(), name, Vec::new());
            collect(&owner, pattern.span, PredicateMode::Run);
        }
        reads
    }

    /// Actual read associations once per source owner in this hidden pass.
    fn source_reads<'b>(
        &'b self,
        decisions: &'b Typed,
    ) -> BTreeMap<ReadKey, SourceReads<'b>> {
        let current_home = self.declared_home.clone().unwrap_or(Origin::This);
        let catalog = self.predicate_catalog(decisions);
        let mut pending = alloc::vec![(
            decisions,
            current_home.clone(),
            current_home.clone(),
            self.resolved
        )];
        let mut result = BTreeMap::new();
        while let Some((typed, home, parameters_home, resolved)) = pending.pop()
        {
            let key = (
                core::ptr::from_ref(typed) as usize,
                home.clone(),
                parameters_home.clone(),
            );
            if result.contains_key(&key) {
                continue;
            }
            let module = if home == current_home {
                self.module
            } else {
                self.owner(&home)
                    .expect("default source owner is available")
            };
            let mut reads = Self::read_casts_at(module, typed);
            reads.extend(Self::member_reads(typed));
            reads.sort_by_key(|(site, _)| site.start);
            let mut predicates = Self::predicate_reads_at(
                module,
                resolved,
                typed,
                &home,
                &parameters_home,
                &catalog,
            );
            predicates.sort_by_key(|read| read.site.start);
            result.insert(
                key,
                SourceReads {
                    ordinary: reads,
                    predicates,
                },
            );
            pending.extend(default_edges(typed).into_iter().map(
                |(_, instance)| {
                    let child_home = if instance.origin == Origin::This {
                        home.clone()
                    } else {
                        instance.origin.clone()
                    };
                    (
                        instance.typed.as_ref(),
                        child_home,
                        parameters_home.clone(),
                        instance.resolved.as_ref(),
                    )
                },
            ));
        }
        result
    }

    /// Actual selected member-node and dispatch-result types, indexed once.
    fn member_reads(decisions: &Typed) -> Vec<(Span, &Type)> {
        let mut reads = Vec::new();
        for (site, steps) in &decisions.accesses {
            let span = Span {
                start: site.0,
                end: site.1,
            };
            for (index, step) in steps.iter().enumerate() {
                if !matches!(step.selected, Selected::Member { .. }) {
                    continue;
                }
                let ty = if index + 1 == steps.len() {
                    decisions.callee_types.get(site).unwrap_or(&step.callee)
                } else {
                    &step.callee
                };
                // The compiler can read this callable before applying supplies.
                reads.push((span, ty));
                if let Some(bound) = &step.bound {
                    reads.push((span, &step.result));
                    reads
                        .extend(bound.applied.iter().map(|(ty, _)| (span, ty)));
                }
            }
        }
        // A direct dynamic operation passes its actual checked callable to
        // dispatch. A selected row may consume only a prefix of these inputs,
        // leaving the remaining arrows in its protocol result.
        for (site, selected) in &decisions.selected {
            if matches!(selected, Selected::Member { .. }) {
                if let Some(ty) = decisions.callee_types.get(site) {
                    reads.push((
                        Span {
                            start: site.0,
                            end: site.1,
                        },
                        ty,
                    ));
                }
            }
        }
        reads
    }

    /// Whether `parameter` is hidden in the body at `span`: read by a
    /// cast or selected by an operation inside it.
    fn hidden_in(
        &self,
        span: Span,
        parameter: &Type,
        reads: &BTreeMap<ReadKey, SourceReads<'_>>,
        hidden: &BTreeMap<Site, Vec<usize>>,
        decisions: &Typed,
    ) -> bool {
        let current_home = self.declared_home.clone().unwrap_or(Origin::This);
        let parameter = self.apply(parameter).rehome(&current_home);
        if self.direct_hidden_in(
            span,
            &parameter,
            &reads[&(
                core::ptr::from_ref(decisions) as usize,
                current_home.clone(),
                current_home.clone(),
            )],
            hidden,
            decisions,
            &current_home,
        ) {
            return true;
        }
        let mut pending: Vec<_> = default_edges(decisions)
            .into_iter()
            .filter(|(at, _)| span.start <= at.0 && at.1 <= span.end)
            .map(|(_, instance)| {
                (instance, current_home.clone(), current_home.clone())
            })
            .collect();
        let mut seen = alloc::collections::BTreeSet::new();
        while let Some((instance, parent_home, parameters_home)) = pending.pop()
        {
            let home = if instance.origin == Origin::This {
                parent_home
            } else {
                instance.origin.clone()
            };
            if !seen.insert((
                default_root_key(instance, &home),
                parameters_home.clone(),
            )) {
                continue;
            }
            let module = if home == current_home {
                self.module
            } else {
                self.owner(&home)
                    .expect("default source owner is available")
            };
            let nested = module
                .expressions
                .get(instance.value.0)
                .expect("default expression belongs to its owner")
                .span;
            let nested_reads = &reads[&(
                alloc::rc::Rc::as_ptr(&instance.typed) as usize,
                home.clone(),
                parameters_home.clone(),
            )];
            let masks = if home == current_home {
                hidden
            } else {
                &instance.typed.hidden
            };
            if self.direct_hidden_in(
                nested,
                &parameter,
                nested_reads,
                masks,
                &instance.typed,
                &home,
            ) {
                return true;
            }
            pending.extend(
                default_edges(&instance.typed)
                    .into_iter()
                    .filter(|(at, _)| {
                        nested.start <= at.0 && at.1 <= nested.end
                    })
                    .map(|(_, instance)| {
                        (instance, home.clone(), parameters_home.clone())
                    }),
            );
        }
        false
    }

    fn direct_hidden_in(
        &self,
        span: Span,
        parameter: &Type,
        reads: &SourceReads<'_>,
        hidden: &BTreeMap<Site, Vec<usize>>,
        decisions: &Typed,
        home: &Origin,
    ) -> bool {
        let contains = |ty: &Type| {
            super::dot::contains_type(&self.apply(ty).rehome(home), parameter)
        };
        let first = reads
            .ordinary
            .partition_point(|(site, _)| site.start < span.start);
        let predicate_start = reads
            .predicates
            .partition_point(|read| read.site.start < span.start);
        reads.ordinary[first..]
            .iter()
            .take_while(|(site, _)| site.start <= span.end)
            .any(|(site, to)| site.end <= span.end && contains(to))
            || reads.predicates[predicate_start..]
                .iter()
                .take_while(|read| read.site.start <= span.end)
                .any(|read| {
                    let current_home =
                        self.declared_home.clone().unwrap_or(Origin::This);
                    read.home == current_home
                        && read.site.end <= span.end
                        && contains(&read.parameter)
                })
            || decisions
                .selected
                .range((span.start, 0)..=(span.end, usize::MAX))
                .any(|(site, selected)| {
                    if site.1 > span.end {
                        return false;
                    }
                    match selected {
                        Selected::Static { head, .. }
                        | Selected::Member { head, .. } => contains(head),
                        Selected::Derived {
                            ty,
                            kind: crate::ir::Derived::Default,
                        } => contains(ty),
                        Selected::Declared { .. } => {
                            let Some(ty) = decisions.callee_types.get(site)
                            else {
                                return false;
                            };
                            let mut result = ty;
                            while let Type::Function {
                                result: output, ..
                            } = result
                            {
                                result = output;
                            }
                            contains(result)
                        }
                        Selected::Value { .. } => decisions
                            .meaning_arguments
                            .get(site)
                            .is_some_and(|arguments| {
                                self.reference_uses(
                                    selected, arguments, parameter, hidden,
                                    home,
                                )
                            }),
                        _ => false,
                    }
                })
            || decisions
                .roots
                .range((span.start, 0)..=(span.end, usize::MAX))
                .any(|(site, root)| {
                    site.1 <= span.end
                        && self.reference_uses(
                            &root.selected,
                            &root.meanings,
                            parameter,
                            hidden,
                            home,
                        )
                })
            || decisions
                .accesses
                .range((span.start, 0)..=(span.end, usize::MAX))
                .any(|(site, steps)| {
                    site.1 <= span.end
                        && steps.iter().any(|step| {
                            self.reference_uses(
                                &step.selected,
                                &step.meanings,
                                parameter,
                                hidden,
                                home,
                            )
                        })
                })
    }

    fn reference_uses(
        &self,
        selected: &Selected,
        arguments: &[Type],
        parameter: &Type,
        hidden: &BTreeMap<Site, Vec<usize>>,
        home: &Origin,
    ) -> bool {
        if let Selected::Member { head, .. } = selected {
            return super::dot::contains_type(
                &self.apply(head).rehome(home),
                parameter,
            );
        }
        let Selected::Value { origin, site } = selected else {
            return false;
        };
        let origin_home = if *origin == Origin::This {
            home
        } else {
            origin
        };
        let indices =
            if *origin == Origin::This {
                hidden.get(&(site.start, site.end)).cloned().unwrap_or_else(
                    || self.reference_hidden(origin_home, *site, hidden),
                )
            } else {
                self.reference_hidden(origin_home, *site, hidden)
            };
        indices.iter().any(|index| {
            arguments.get(*index).is_some_and(|ty| {
                super::dot::contains_type(
                    &self.apply(ty).rehome(home),
                    parameter,
                )
            })
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
                .constructor(&(origin.clone(), site))
                .and_then(|entry| match entry {
                    super::files::Constructor::Checked(_, source) => {
                        source.typed.hidden.get(&key).cloned()
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    self.file_cache
                        .expressions
                        .borrow()
                        .values()
                        .find_map(|expression| {
                            (expression.origin == *origin)
                                .then(|| {
                                    expression.typed.hidden.get(&key).cloned()
                                })
                                .flatten()
                        })
                        .unwrap_or_default()
                }),
        }
    }
}

#[cfg(test)]
mod retained_default_tests {
    use super::*;
    use alloc::{boxed::Box, rc::Rc, vec};

    #[test]
    fn retained_default_edges_keep_receiving_sites_and_distinct_roots() {
        let module = crate::parse::parse(crate::lex::lex("1\n2")).unwrap();
        let declarations = crate::decl::Declarations::new();
        let resolved = Rc::new(
            crate::resolve::resolve(&module, &declarations, &[]).unwrap(),
        );
        let shared = Rc::new(Typed::default());
        let first = DeclaredExpression {
            origin: Origin::This,
            value: crate::ast::ExprId(0),
            resolved: Rc::clone(&resolved),
            typed: Rc::clone(&shared),
        };
        let second = DeclaredExpression {
            value: crate::ast::ExprId(1),
            ..first.clone()
        };
        let expression = |value: DeclaredExpression| {
            DefaultValue::Expression(Box::new(value))
        };
        let mut typed = Typed::default();
        typed.default_instances.insert((1, 2, 0), first.clone());
        typed.derived_defaults.insert(
            (3, 4),
            DefaultValue::Record {
                ty: Type::Unit,
                fields: vec![(
                    Span { start: 0, end: 1 },
                    Type::Unit,
                    DefaultValue::Array {
                        ty: Type::Unit,
                        element: Box::new(expression(first.clone())),
                    },
                )],
            },
        );
        typed.bound.insert(
            (5, 6),
            crate::typed::Bound {
                defaults: vec![(0, expression(first))],
                ..crate::typed::Bound::default()
            },
        );
        typed.accesses.insert(
            (7, 8),
            vec![crate::typed::Access {
                head: Type::Unit,
                callee: Type::Unit,
                result: Type::Unit,
                selected: Selected::Derived {
                    ty: Type::Unit,
                    kind: crate::ir::Derived::Default,
                },
                meanings: vec![],
                bound: Some(crate::typed::Bound {
                    defaults: vec![(0, expression(second))],
                    ..crate::typed::Bound::default()
                }),
            }],
        );
        let mut sites: Vec<_> = default_edges(&typed)
            .iter()
            .map(|(site, _)| *site)
            .collect();
        sites.sort_unstable();
        assert_eq!(sites, [(1, 2), (3, 4), (5, 6), (7, 8)]);
        // Listing restores only the lookup; embedded executable plans remain.
        typed.default_instances.clear();
        let edges = default_edges(&typed);
        assert_eq!(edges.len(), 3);
        let caches: alloc::collections::BTreeSet<_> = edges
            .iter()
            .map(|(_, instance)| Rc::as_ptr(&instance.typed) as usize)
            .collect();
        let roots: alloc::collections::BTreeSet<_> = edges
            .iter()
            .map(|(_, instance)| default_root_key(instance, &Origin::This))
            .collect();
        assert_eq!(caches.len(), 1);
        assert_eq!(roots.len(), 2);
    }
}
