//! Static facts and entry checks over the typed tree; never unification.
#[path = "../coverage.rs"]
mod coverage;
mod entry;
mod holding;
pub mod known;
mod phase;
mod plan;
pub use holding::Holding;
pub use plan::{Check, Frequency, Phase, Place, Reason};

use crate::{
    ast::{
        BinaryOp, Entry, ExprId, ExprKind, ItemKind, Limit, Literal, Module,
        PatternKind, PipeOp, TypeId, TypeKind,
    },
    decl::Declarations,
    frame::Frame,
    infer::{Error, Reason as TypeReason},
    resolve::{Origin, Resolved, Target},
    token::Span,
    typed::Typed,
    types::{Type, TypeName},
};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use known::{End, Integer, Known, Range};

type Site = (usize, usize);

/// What every static pass reads: the unit, its names, the declarations
/// and files it may reach, and the session frame.
pub(crate) struct Inputs<'a> {
    pub module: &'a Module,
    pub resolved: &'a Resolved,
    pub declarations: &'a Declarations,
    pub files: &'a [(String, Module)],
    pub cache: &'a crate::infer::files::FileCache,
    pub frame: Option<&'a Frame>,
}

pub(crate) fn check(
    inputs: &Inputs<'_>,
    entries: &[(ExprId, Type, Span, Place)],
    instance: Option<ExprId>,
    typed: &mut Typed,
) -> Vec<Error> {
    let module = inputs.module;
    let mut pass = Pass::new(inputs, typed, Place::Result);
    pass.instance = instance;
    pass.typed.known = vec![Known::default(); module.expressions.len()];
    pass.seed_places();
    pass.check_purity();
    let mut entry_types = vec![None; module.expressions.len()];
    for (id, ty, _, _) in entries {
        entry_types[id.0].get_or_insert(ty);
    }
    for index in 0..module.expressions.len() {
        let id = ExprId(index);
        if pass.typed.expr(id).is_none() {
            continue;
        }
        let mut known = pass.fact(id);
        pass.construction_entry(id, &mut known);
        pass.settle_default_divisors(id);
        pass.expression_holding(id, entry_types[index]);
        pass.typed.known[index] = known.clone();
        pass.binding_facts(id);
    }
    pass.timing_places();
    pass.holdings();
    pass.demands();
    pass.finish_checks();
    pass.entries(entries);
    pass.keep_facts();
    pass.coverage();
    pass.typed.facts = pass
        .locals
        .into_iter()
        .filter(|(site, _)| site.0 >= module.span.start)
        .collect();
    pass.errors
}

/// Ask the same static entry pass whether a type default is proved.
pub(crate) fn default_proved(
    inputs: &Inputs<'_>,
    ty: &Type,
    known: &Known,
) -> bool {
    let mut typed = Typed::default();
    let pass = Pass::new(inputs, &mut typed, Place::Result);
    pass.evaluate_type(ty, known, 0).boolean == Some(true)
}

pub(crate) fn flag_entry(
    inputs: &Inputs<'_>,
    ty: &Type,
    (span, place_kind): (Span, Place),
) -> (Vec<Error>, Vec<Check>) {
    let mut typed = Typed::default();
    let mut pass = Pass::new(inputs, &mut typed, place_kind);
    pass.enter(
        ty,
        &Known {
            boolean: Some(true),
            ..Known::default()
        },
        span,
        pass.frequency(span),
    );
    (pass.errors, typed.checks)
}

pub(crate) fn default_fact(module: &Module, value: ExprId) -> Known {
    test(module, value, &Known::default())
}

fn declaration_index(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: Option<&Frame>,
) -> BTreeMap<TypeName, usize> {
    let modules = core::iter::once((module, Origin::This))
        .chain(core::iter::once((
            &declarations.prelude.module,
            Origin::Prelude,
        )))
        .chain(
            declarations
                .commands
                .iter()
                .map(|n| (&n.module, Origin::Commands)),
        )
        .chain(
            declarations
                .modules
                .iter()
                .map(|n| (&n.module, Origin::Host(n.name.clone()))),
        )
        .chain(files.iter().map(|(n, m)| (m, Origin::File(n.clone()))))
        .chain(frame.into_iter().flat_map(|f| {
            f.lines
                .iter()
                .enumerate()
                .map(|(i, l)| (&l.module, Origin::Line(i)))
        }));
    let mut index = BTreeMap::new();
    for (module, origin) in modules {
        for (i, entry) in module.entries.iter().enumerate() {
            if let Entry::Item(item) = entry {
                if let ItemKind::Type { name, .. } = &item.kind {
                    index.insert(
                        TypeName {
                            origin: origin.clone(),
                            name: name
                                .segments
                                .last()
                                .cloned()
                                .unwrap_or_default(),
                        },
                        i,
                    );
                }
            }
        }
    }
    index
}

fn contexts(
    module: &Module,
) -> BTreeMap<usize, (Phase, Frequency, Option<String>)> {
    let mut contexts: BTreeMap<_, _> = module
        .expressions
        .iter()
        .map(|e| e.span.start)
        .chain(module.types.iter().map(|t| t.span.start))
        .map(|start| (start, (Phase::Eval, Frequency::Once, None)))
        .collect();
    for entry in &module.entries {
        let Entry::Item(item) = entry else {
            continue;
        };
        let ItemKind::Impl {
            target, members, ..
        } = &item.kind
        else {
            continue;
        };
        let TypeKind::Named { name: ty, .. } = &module.types[target.0].kind
        else {
            continue;
        };
        for member in members {
            if let crate::ast::ImplMember::Named { name, span, .. } = member {
                let owner = format!(
                    "{}.{}",
                    ty.segments.join("."),
                    name.segments.join(".")
                );
                contexts.entry(name.span.start).or_insert((
                    Phase::Eval,
                    Frequency::PerCall,
                    None,
                ));
                for (_, context) in contexts.range_mut(span.start..span.end) {
                    context.2 = Some(owner.clone());
                }
            }
        }
    }
    for expression in &module.expressions {
        if let ExprKind::Bind { pattern, .. } = expression.kind {
            let owner = match &module.patterns[pattern.0].kind {
                PatternKind::Name(name) => Some(name.segments.join(".")),
                _ => None,
            };
            for (_, context) in
                contexts.range_mut(expression.span.start..expression.span.end)
            {
                context.0 = Phase::Init;
                if context.2.is_none() {
                    context.2.clone_from(&owner);
                }
            }
        }
    }
    for expression in &module.expressions {
        if matches!(expression.kind, ExprKind::Arms(_)) {
            for (_, context) in
                contexts.range_mut(expression.span.start..expression.span.end)
            {
                context.0 = Phase::Eval;
                context.1 = Frequency::PerCall;
            }
        }
    }
    contexts
}

struct Pass<'a> {
    module: &'a Module,
    resolved: &'a Resolved,
    declarations: &'a Declarations,
    files: &'a [(String, Module)],
    cache: &'a crate::infer::files::FileCache,
    frame: Option<&'a Frame>,
    typed: &'a mut Typed,
    locals: BTreeMap<Site, Known>,
    errors: Vec<Error>,
    active: Vec<TypeName>,
    place_kind: Place,
    path: Vec<crate::ir::Projection>,
    entry_phase: Option<Phase>,
    purity: core::cell::RefCell<BTreeMap<Target, bool>>,
    purity_cycles: core::cell::Cell<usize>,
    declarations_by_name: BTreeMap<TypeName, usize>,
    contexts: BTreeMap<usize, (Phase, Frequency, Option<String>)>,
    checks_seen:
        BTreeSet<(Span, Place, Span, Origin, Vec<crate::ir::Projection>)>,
    /// The one declared expression this pass types for a use site, a
    /// default for a call or construction or a limited type's test.
    instance: Option<ExprId>,
}
impl<'a> Pass<'a> {
    fn new(
        inputs: &Inputs<'a>,
        typed: &'a mut Typed,
        place_kind: Place,
    ) -> Self {
        Self {
            module: inputs.module,
            resolved: inputs.resolved,
            declarations: inputs.declarations,
            files: inputs.files,
            cache: inputs.cache,
            frame: inputs.frame,
            typed,
            locals: BTreeMap::new(),
            errors: Vec::new(),
            active: Vec::new(),
            place_kind,
            path: Vec::new(),
            entry_phase: None,
            purity: core::cell::RefCell::new(BTreeMap::new()),
            purity_cycles: core::cell::Cell::new(0),
            declarations_by_name: declaration_index(
                inputs.module,
                inputs.declarations,
                inputs.files,
                inputs.frame,
            ),
            contexts: contexts(inputs.module),
            checks_seen: BTreeSet::new(),
            instance: None,
        }
    }
}
impl Pass<'_> {
    /// What is known of an expression's value before it enters a place.
    fn fact(&mut self, id: ExprId) -> Known {
        // A cast value is known by its target's meaning alone: the
        // facts of the source, its length or exact value, say nothing
        // of the converted value. The Bytes row keeps a text literal's
        // byte length, which is the bytes' length.
        let cast = self
            .typed
            .casts
            .get(id.0)
            .and_then(Option::as_ref)
            .filter(|cast| cast.row != crate::typed::CastRow::Pack)
            .cloned();
        let mut known = match &cast {
            Some(cast)
                if cast.row == crate::typed::CastRow::Bytes
                    && matches!(
                        self.module.expressions[id.0].kind,
                        ExprKind::Literal(Literal::Str { .. })
                    ) =>
            {
                Known {
                    parts: self.value(id).parts,
                    ..Known::default()
                }
            }
            Some(_) => Known::default(),
            None => self.value(id),
        };
        known.pure = self.pure_expression(id);
        known.phase = self.expression_phase(id);
        known.demanded |= call_head(self.module, id).and_then(|callee| {
            crate::resolve::timing(self.resolved.expr(callee))
        }) == Some(Phase::Compile);
        if let Some(ty) = self.typed.expr(id) {
            let ty = cast.as_ref().map_or(ty, |cast| &cast.to);
            if let Type::Named { name, .. } = crate::infer::meaning(ty) {
                known.meaning = Some(name.clone());
            }
            if cast.is_none()
                && (matches!(ty, Type::Array { .. })
                    || matches!(
                        self.module.expressions[id.0].kind,
                        ExprKind::Field { .. }
                    )
                    || matches!(
                        self.resolved.expr(id),
                        Some(Target::Members { .. })
                    ))
                && known.failure != Some(known::Failure::Dependency)
            {
                self.place_facts(ty, &mut known);
            }
            known.holding = self.holding(ty, &mut Vec::new()).or(known.holding);
        }
        known
    }

    fn finish_checks(&mut self) {
        let demanded: BTreeSet<_> = self
            .module
            .expressions
            .iter()
            .zip(&self.typed.known)
            .filter_map(|(expression, known)| {
                known.demanded.then_some(expression.span)
            })
            .collect();
        for check in &mut self.typed.checks {
            if check.owner.is_none() {
                check.owner = self
                    .contexts
                    .get(&check.place.start)
                    .and_then(|c| c.2.clone());
            }
            if demanded.contains(&check.place) {
                check.phase = Phase::Compile;
                check.reason = Reason::CompileTimeEvaluation;
            }
        }
    }

    /// A division outside a type item is entered where it stands. In
    /// a type's test it is the test's own fault, never an entry. In a
    /// field default it is entered by the pass that types the default
    /// for a construction, not at the declaration.
    fn divisor_entered(&self, span: Span) -> bool {
        let within =
            |outer: Span| outer.start <= span.start && span.end <= outer.end;
        let Some(item) = self.module.entries.iter().find_map(|entry| {
            let Entry::Item(item) = entry else {
                return None;
            };
            (matches!(item.kind, ItemKind::Type { .. }) && within(item.span))
                .then_some(item)
        }) else {
            return true;
        };
        let ItemKind::Type { members, .. } = &item.kind else {
            return true;
        };
        members.iter().flatten().any(|member| {
            let crate::ast::Member::Field(field) = member else {
                return false;
            };
            field.default.is_some_and(|default| {
                self.instance == Some(default)
                    && within(self.module.expressions[default.0].span)
            })
        })
    }

    /// The facts a call or construction supplies to each spot by name.
    fn supplied_facts(&self, stage: Span) -> Vec<(String, Known)> {
        self.typed
            .supplied
            .get(&(stage.start, stage.end))
            .map(|values| {
                values
                    .iter()
                    .map(|(name, supply)| {
                        (name.clone(), self.supply_fact(supply))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn supply_fact(&self, supply: &crate::infer::Supply) -> Known {
        let get = |id: ExprId| self.typed.known[id.0].clone();
        match supply {
            crate::infer::Supply::Expr(id) => get(*id),
            crate::infer::Supply::Collected(ids) => Known {
                parts: core::iter::once((
                    String::from("len"),
                    exact(Integer::from(ids.len() as u64)),
                ))
                .chain(
                    ids.iter()
                        .enumerate()
                        .map(|(i, id)| (i.to_string(), get(*id))),
                )
                .collect(),
                ..Known::default()
            },
            crate::infer::Supply::Default(known) => known.clone(),
            crate::infer::Supply::True(_) => Known {
                boolean: Some(true),
                ..Known::default()
            },
            crate::infer::Supply::Typed(_, span) => self
                .module
                .expressions
                .iter()
                .position(|e| e.span == *span)
                .map_or_else(Known::default, |i| get(ExprId(i))),
        }
    }

    /// The facts of a record written by its fields: each written field
    /// by its value, each other field by its default read with the
    /// fields before it.
    fn record_facts(&self, id: ExprId) -> Vec<(String, Known)> {
        let ExprKind::Record { fields, .. } =
            &self.module.expressions[id.0].kind
        else {
            return Vec::new();
        };
        let written = |name: &str| {
            fields
                .iter()
                .find(|f| f.name.segments.last().is_some_and(|n| n == name))
                .map(|f| self.typed.known[f.value.0].clone())
        };
        let declared = self.typed.expr(id).and_then(|ty| {
            let Type::Named { name, .. } = ty else {
                return None;
            };
            let module = self.owner(&name.origin)?;
            Some(
                construction_fields(module, &name.name, &name.name)
                    .into_iter()
                    .map(|field| (field, module))
                    .collect::<Vec<_>>(),
            )
        });
        let Some(declared) = declared.filter(|fields| !fields.is_empty())
        else {
            return fields
                .iter()
                .map(|f| {
                    (
                        f.name.segments.last().cloned().unwrap_or_default(),
                        self.typed.known[f.value.0].clone(),
                    )
                })
                .collect();
        };
        let mut parts: Vec<(String, Known)> = Vec::new();
        for (field, module) in declared {
            let name = field.name.segments.last().cloned().unwrap_or_default();
            let value = written(&name)
                .or_else(|| {
                    field.default.map(|id| spot_test(module, id, &parts))
                })
                .unwrap_or_default();
            parts.push((name, value));
        }
        parts
    }

    /// A default's divisor check, planned where the default was typed,
    /// is settled with what this call or construction supplies: exact
    /// zero refutes it, a range without zero proves it, the rest stays
    /// open with the supplied facts.
    fn settle_default_divisors(&mut self, id: ExprId) {
        let expression = &self.module.expressions[id.0];
        let stage = match expression.kind {
            ExprKind::Pipe {
                op: PipeOp::Plain,
                right,
                ..
            } => self.module.expressions[right.0].span,
            ExprKind::Call { .. } | ExprKind::Record { .. } => expression.span,
            _ => return,
        };
        let spots = if matches!(expression.kind, ExprKind::Record { .. }) {
            self.record_facts(id)
        } else {
            self.supplied_facts(stage)
        };
        let mut keep = Vec::new();
        let mut errors = Vec::new();
        for (index, check) in self.typed.checks.iter().enumerate() {
            if check.place != stage || check.target.is_some() {
                continue;
            }
            let Some(value) = check.value else { continue };
            let Some(module) = self.owner(&check.limit_origin) else {
                continue;
            };
            let fact = spot_test(module, value, &spots);
            let range = fact.range.clone().unwrap_or_else(Range::unknown);
            if range == Range::exact(0.into()) {
                errors.push(Error {
                    span: stage,
                    reason: TypeReason::Limit {
                        message: String::from(
                            "division by zero in a default: \
                             the divisor is exact 0",
                        ),
                    },
                });
                keep.push((index, None));
            } else if range.contains_zero() {
                keep.push((index, Some(fact)));
            } else {
                keep.push((index, None));
            }
        }
        self.errors.extend(errors);
        for (index, fact) in keep.into_iter().rev() {
            match fact {
                Some(fact) => self.typed.checks[index].known = fact,
                None => {
                    self.typed.checks.remove(index);
                }
            }
        }
    }

    fn construction_entry(&mut self, id: ExprId, known: &mut Known) {
        if matches!(
            self.module.expressions[id.0].kind,
            ExprKind::Record { .. }
                | ExprKind::Call { .. }
                | ExprKind::Pipe {
                    op: PipeOp::Plain,
                    ..
                }
        ) {
            if let Some(Type::Named { name, .. }) = self.typed.expr(id).cloned()
            {
                if matches!(
                    self.module.expressions[id.0].kind,
                    ExprKind::Record { .. }
                ) || matches!(
                    call_head(self.module, id)
                        .and_then(|callee| self.resolved.expr(callee)),
                    Some(Target::Constructor { .. } | Target::SelfType)
                ) {
                    self.named_entry(
                        &name,
                        known,
                        self.module.expressions[id.0].span,
                        self.frequency(self.module.expressions[id.0].span),
                        &mut Vec::new(),
                    );
                    if self.declaration(&name).is_some_and(|(_, _, limits)| {
                        limits.iter().any(|l| matches!(l, Limit::Test(_)))
                    }) {
                        self.named_facts(&name, known, &mut Vec::new());
                    }
                }
            }
        }
    }

    fn check_purity(&mut self) {
        for entry in &self.module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Type { limits, .. } = &item.kind {
                    for limit in limits {
                        if let Limit::Test(id) = limit {
                            if self.typed.expr(*id).is_some()
                                && !self.pure_test(*id)
                            {
                                self.errors.push(Error {
                                    span: self.module.expressions[id.0].span,
                                    reason: TypeReason::Limit {
                                        message: String::from(
                                            "a limit test must be pure",
                                        ),
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn frequency(&self, span: Span) -> Frequency {
        self.contexts
            .get(&span.start)
            .map_or(Frequency::Once, |context| context.1)
    }
    fn entry_owner(&self, span: Span) -> Option<String> {
        self.contexts
            .get(&span.start)
            .and_then(|context| context.2.clone())
    }
    fn expression_holding(&mut self, id: ExprId, entry: Option<&Type>) {
        if let Some(ty) = entry.or_else(|| self.typed.expr(id)).cloned() {
            self.instantiated_holding(&ty, self.module.expressions[id.0].span);
        }
    }

    fn seed_places(&mut self) {
        for (site, local) in &self.typed.frozen {
            let mut known = Known {
                pure: true,
                ..Known::default()
            };
            self.place_facts(&local.scheme.ty, &mut known);
            self.locals.insert(*site, known);
        }
        // Parameters are places: seed their written limits before reading bodies.
        for entry in &self.module.entries {
            if let Entry::Line(id) = entry {
                self.seed(*id);
            }
        }
    }

    fn binding_facts(&mut self, id: ExprId) {
        if let ExprKind::Bind {
            pattern,
            annotation,
            value,
            ..
        } = self.module.expressions[id.0].kind
        {
            let mut fact = self.typed.known[value.0].clone();
            if annotation.is_some()
                && fact.failure != Some(known::Failure::Dependency)
            {
                if let Some(scheme) = self.typed.binding(id).cloned() {
                    fact.demanded |= annotation.is_some_and(|annotation| {
                        self.type_timings(
                            self.module,
                            &Origin::This,
                            annotation,
                            &mut Vec::new(),
                        )
                        .iter()
                        .any(|(phase, _)| *phase == Phase::Compile)
                    });
                    self.typed.known[value.0].demanded = fact.demanded;
                    self.place_facts(&scheme.ty, &mut fact);
                    fact.holding = self.holding(&scheme.ty, &mut Vec::new());
                    self.typed.known[value.0].holding.clone_from(&fact.holding);
                }
            }
            self.pattern_facts(pattern, &fact);
        }
    }

    fn pattern_facts(&mut self, pattern: crate::ast::PatternId, fact: &Known) {
        match &self.module.patterns[pattern.0].kind {
            PatternKind::Name(name) => {
                self.locals
                    .insert((name.span.start, name.span.end), fact.clone());
            }
            PatternKind::Tuple(parts) | PatternKind::List(parts) => {
                for (index, part) in parts.iter().enumerate() {
                    let mut known = fact
                        .parts
                        .iter()
                        .find(|(name, _)| *name == index.to_string())
                        .map_or_else(Known::default, |(_, k)| k.clone());
                    known.phase = fact.phase;
                    known.pure = fact.pure;
                    self.pattern_facts(*part, &known);
                }
            }
            PatternKind::Constructor { payload, .. } => {
                if let Some(Target::Type { origin, name }) =
                    self.resolved.pattern(pattern)
                {
                    let name = TypeName {
                        origin: origin.clone(),
                        name: name.clone(),
                    };
                    let start = self.typed.checks.len();
                    self.named_entry(
                        &name,
                        fact,
                        self.module.patterns[pattern.0].span,
                        Frequency::Once,
                        &mut Vec::new(),
                    );
                    for check in &mut self.typed.checks[start..] {
                        check.gate = true;
                    }
                    let mut fact = fact.clone();
                    self.named_facts(&name, &mut fact, &mut Vec::new());
                    self.pattern_facts(*payload, &fact);
                }
            }
            PatternKind::Group(inner) => self.pattern_facts(*inner, fact),
            _ => (),
        }
    }

    fn keep_facts(&mut self) {
        for expr in &self.module.expressions {
            if let ExprKind::Bind { pattern, value, .. } = expr.kind {
                if let PatternKind::Name(name) =
                    &self.module.patterns[pattern.0].kind
                {
                    if let Some(fact) =
                        self.locals.get_mut(&(name.span.start, name.span.end))
                    {
                        fact.phase = self.typed.known[value.0].phase;
                        fact.demanded = self.typed.known[value.0].demanded;
                        fact.holding
                            .clone_from(&self.typed.known[value.0].holding);
                    }
                }
            }
        }
    }

    fn entries(&mut self, entries: &[(ExprId, Type, Span, Place)]) {
        let module = self.module;
        let branches: Vec<_> = entries
            .iter()
            .filter(|(id, _, _, _)| {
                matches!(module.expressions[id.0].kind, ExprKind::If { .. })
            })
            .collect();
        let annotated: BTreeSet<_> = module
            .expressions
            .iter()
            .filter_map(|e| {
                if let ExprKind::Bind {
                    value,
                    annotation: Some(_),
                    ..
                } = e.kind
                {
                    Some(value.0)
                } else {
                    None
                }
            })
            .collect();
        for (id, ty, place, place_kind) in entries {
            if !has_limits(ty) && !self.typed.expr(*id).is_some_and(has_limits)
            {
                continue;
            }
            let expr = &module.expressions[id.0];
            if self.typed.expr(*id) == Some(ty) && call_head(module, *id).is_some_and(|callee| matches!(self.resolved.expr(callee), Some(Target::Type { .. })) || matches!(self.resolved.expr(callee), Some(Target::Value { origin: Origin::Prelude, name }) if name == "to")) { continue; }

            // A group, block or `with` enters by its last value and a
            // tuple by its parts, unless the whole is cast: then the
            // cast value enters, known by its meaning alone.
            let cast = self.typed.casts.get(id.0).is_some_and(Option::is_some);
            if !cast
                && matches!(
                    expr.kind,
                    ExprKind::Group(_)
                        | ExprKind::Block(_)
                        | ExprKind::Tuple(_)
                        | ExprKind::With { .. }
                )
            {
                continue;
            }
            if branches.iter().copied().any(
                |(outer, outer_ty, outer_place, _)| {
                    let parent = &module.expressions[outer.0];
                    *outer != *id
                        && outer_place == place
                        && outer_ty == ty
                        && matches!(parent.kind, ExprKind::If { .. })
                        && parent.span.start <= expr.span.start
                        && expr.span.end <= parent.span.end
                },
            ) {
                continue;
            }
            if matches!(expr.kind, ExprKind::Arms(_))
                && annotated.contains(&id.0)
            {
                continue;
            }
            if matches!(expr.kind, ExprKind::List(_))
                && (matches!(ty, Type::Array { .. })
                    || matches!(ty, Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "Vec" | "Deque")))
            {
                continue;
            }
            let mut fact = self.typed.known[id.0].clone();
            if !cast && matches!(expr.kind, ExprKind::List(_)) {
                fact.demanded = true;
            }
            self.place_kind = place_kind.clone();
            self.entry_phase =
                (fact.demanded || fact.is_exact()).then_some(Phase::Compile);
            let start = self.typed.checks.len();
            self.enter(ty, &fact, *place, self.frequency(expr.span));
            for check in &mut self.typed.checks[start..] {
                check.value = Some(*id);
            }
        }
    }

    fn owner(&self, origin: &Origin) -> Option<&Module> {
        match origin {
            Origin::This => Some(self.module),
            Origin::Prelude => Some(&self.declarations.prelude.module),
            Origin::Commands => {
                self.declarations.commands.as_ref().map(|n| &n.module)
            }
            Origin::Host(name) => {
                self.declarations.get_module(name).map(|n| &n.module)
            }
            Origin::File(name) => {
                self.files.iter().find(|(n, _)| n == name).map(|(_, m)| m)
            }
            Origin::Line(_) => self.frame?.module(origin),
        }
    }
    fn declaration(
        &self,
        name: &TypeName,
    ) -> Option<(&Module, Option<TypeId>, &[Limit])> {
        let module = self.owner(&name.origin)?;
        let index = *self.declarations_by_name.get(name)?;
        let Entry::Item(item) = &module.entries[index] else {
            return None;
        };
        let ItemKind::Type { base, limits, .. } = &item.kind else {
            return None;
        };
        Some((module, *base, limits.as_slice()))
    }

    fn type_name(
        &self,
        module: &Module,
        origin: &Origin,
        id: TypeId,
    ) -> Option<TypeName> {
        if core::ptr::eq(module, self.module) {
            if let Some(Target::Type { origin, name }) = self.resolved.ty(id) {
                return Some(TypeName {
                    origin: origin.clone(),
                    name: name.clone(),
                });
            }
        }
        if let Origin::File(stem) = origin {
            if let Some(Target::Type {
                origin: target_origin,
                name,
            }) = self
                .cache
                .files
                .borrow()
                .get(stem)
                .and_then(|c| c.resolved.ty(id))
            {
                return Some(TypeName {
                    origin: if *target_origin == Origin::This {
                        origin.clone()
                    } else {
                        target_origin.clone()
                    },
                    name: name.clone(),
                });
            }
        }
        if let Origin::Line(index) = origin {
            if let Some(Target::Type {
                origin: target_origin,
                name,
            }) = self
                .frame
                .and_then(|f| f.lines.get(*index))
                .and_then(|l| l.resolved.ty(id))
            {
                return Some(TypeName {
                    origin: if *target_origin == Origin::This {
                        origin.clone()
                    } else {
                        target_origin.clone()
                    },
                    name: name.clone(),
                });
            }
        }
        let TypeKind::Named { name, .. } = &module.types[id.0].kind else {
            return None;
        };
        let name = name.segments.last()?.clone();
        let local = self.declarations_by_name.contains_key(&TypeName {
            origin: origin.clone(),
            name: name.clone(),
        });
        Some(TypeName {
            origin: if local {
                origin.clone()
            } else {
                Origin::Prelude
            },
            name,
        })
    }
    fn negated_gate(&self, ty: &Type, gates: &[TypeName]) -> bool {
        match ty {
            Type::Not(inner) => {
                matches!(&**inner, Type::Named { name, .. } if gates.contains(name))
            }
            Type::Both(parts) => {
                parts.iter().any(|p| self.negated_gate(p, gates))
                    && parts.iter().all(|p| {
                        if matches!(p, Type::Not(_)) {
                            return true;
                        }
                        let Type::Named { name, .. } = p else {
                            return false;
                        };
                        self.declaration(name).is_some_and(
                            |(module, base, limits)| {
                                let meaning = self.declarations_by_name.get(name)
                                    .is_some_and(|index| matches!(&module.entries[*index], Entry::Item(item) if matches!(item.kind, ItemKind::Type { members: Some(_), .. })));
                                (base.is_none() || meaning) && limits.is_empty()
                            },
                        )
                    })
            }
            _ => false,
        }
    }
    fn seed(&mut self, id: ExprId) {
        let ExprKind::Bind { value, .. } = self.module.expressions[id.0].kind
        else {
            return;
        };
        let ExprKind::Arms(arms) = &self.module.expressions[value.0].kind
        else {
            return;
        };
        let Some(scheme) = self.typed.binding(id) else {
            return;
        };
        let ty = scheme.ty.clone();
        for arm in arms {
            let mut ty = &ty;
            for pattern in &arm.patterns {
                let Type::Function { parameter, result } = ty else {
                    break;
                };
                if let PatternKind::Name(name) =
                    &self.module.patterns[pattern.0].kind
                {
                    let mut known = Known {
                        pure: true,
                        ..Known::default()
                    };
                    self.place_facts(parameter, &mut known);
                    self.locals.insert((name.span.start, name.span.end), known);
                }
                ty = result;
            }
        }
    }
    fn value(&mut self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let get = |id: ExprId| self.typed.known[id.0].clone();
        match &expression.kind {
            ExprKind::Literal(_) => self.literal_value(id),
            ExprKind::Name(_) => self.target(self.resolved.expr(id)),
            ExprKind::Field { value, field } => get(*value)
                .part(field.segments.last().map_or("", String::as_str)),
            ExprKind::Group(lines) | ExprKind::Block(lines) => {
                lines.last().map_or_else(Known::default, |id| get(*id))
            }
            ExprKind::List(items) => {
                if self.typed.expr(id).is_some_and(|ty| !matches!(crate::infer::meaning(ty), Type::Array { .. }) && !matches!(crate::infer::meaning(ty), Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "Vec" | "Deque"))) {
                    return Known { narrowing: true, ..Known::default() };
                }
                let mut parts = vec![(
                    String::from("len"),
                    exact(Integer::from(items.len() as u64)),
                )];
                parts.extend(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, id)| (i.to_string(), get(*id))),
                );
                Known {
                    parts,
                    ..Known::default()
                }
            }
            ExprKind::Tuple(parts) => Known {
                parts: parts
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (i.to_string(), get(*id)))
                    .collect(),
                ..Known::default()
            },
            ExprKind::Outcome { good, payload } => Known {
                constructor: Some(String::from(if *good {
                    "Ok"
                } else {
                    "Err"
                })),
                parts: payload
                    .iter()
                    .map(|id| (String::from("v"), get(*id)))
                    .collect(),
                ..Known::default()
            },
            ExprKind::Record { .. } => Known {
                parts: self.record_facts(id),
                ..Known::default()
            },
            ExprKind::Binary { .. } => self.binary_value(id),
            ExprKind::Not { value, .. } => Known {
                boolean: get(*value).boolean.map(|b| !b),
                ..Known::default()
            },
            ExprKind::Pipe {
                op: op @ (PipeOp::And | PipeOp::Or),
                left,
                right,
                ..
            } => bool_op(*op, &get(*left), &get(*right)),
            ExprKind::Call { .. }
            | ExprKind::Pipe {
                op: PipeOp::Plain, ..
            } => self.call_value(id),
            ExprKind::If { .. } | ExprKind::Pipe { .. } => Known {
                narrowing: true,
                ..Known::default()
            },
            _ => Known::default(),
        }
    }
    fn call_value(&mut self, id: ExprId) -> Known {
        if call_head(self.module, id)
            .and_then(|callee| {
                crate::resolve::timing(self.resolved.expr(callee))
            })
            .is_some()
        {
            return first_input(self.module, id)
                .map(|id| self.typed.known[id.0].clone())
                .unwrap_or_default();
        }
        match call_head(self.module, id)
            .and_then(|callee| self.resolved.expr(callee))
        {
            Some(Target::Value {
                origin: Origin::Prelude,
                name,
            }) if matches!(name.as_str(), "to" | "try" | "read") => {
                self.convert_value(id)
            }
            Some(Target::Value {
                origin: Origin::Prelude,
                name,
            }) if name == "len" => first_input(self.module, id)
                .map(|input| self.typed.known[input.0].part("len"))
                .unwrap_or_default(),
            Some(Target::Constructor { .. } | Target::SelfType) => {
                self.construct_value(id)
            }
            Some(Target::Type { .. }) => self.gate_value(id),
            _ => Known {
                narrowing: true,
                ..Known::default()
            },
        }
    }
    fn literal_value(&self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let ExprKind::Literal(literal) = &expression.kind else {
            return Known::default();
        };

        let mut known = literal_fact(literal);
        if let Literal::Float(text) = literal {
            if integer_type(self.typed.expr(id)) {
                if let Some((negative, magnitude)) =
                    crate::token::Magnitude::whole_of_fraction(text)
                {
                    known = exact(Integer::new(negative, magnitude));
                }
            }
        }
        if !integer_type(self.typed.expr(id))
            && matches!(literal, Literal::Int { .. } | Literal::Float(_))
        {
            let text = match literal {
                Literal::Float(text) => text.clone(),
                Literal::Int {
                    negative,
                    magnitude,
                    ..
                } => Integer::new(*negative, magnitude.clone()).to_string(),
                _ => String::new(),
            };
            if !self.typed.expr(id).is_some_and(|ty| matches!(crate::infer::meaning(ty), Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "F32" | "F64"))) { return Known::default(); }
            known.range = None;
            known.exact = text.parse::<f64>().ok().map(|value| {
                if self
                    .typed
                    .expr(id)
                    .is_some_and(|ty| ty.to_string() == "F32")
                {
                    text.parse::<f32>().map_or_else(
                        |_| String::new(),
                        |value| format!("{value}"),
                    )
                } else {
                    format!("{value}")
                }
            });
        }
        if let Some(exact) = &mut known.exact {
            if exact.parse::<f64>().is_ok_and(f64::is_finite)
                && !exact.contains(['.', 'e', 'E'])
            {
                exact.push_str(".0");
            }
        }
        known
    }
    fn binary_value(&mut self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let ExprKind::Binary {
            op,
            left,
            right,
            operator,
        } = &expression.kind
        else {
            return Known::default();
        };
        let get = |id: ExprId| self.typed.known[id.0].clone();

        let a = get(*left);
        let b = get(*right);
        let when_integer = matches!(
            self.typed.expr(*right),
            Some(Type::Var(_) | Type::Rigid(_))
        );
        // A division in a type's test is the test's own fault, never a
        // separate entry; one in a field default enters at each
        // construction, as a function spot's default does at each call.
        if matches!(op, BinaryOp::Divide | BinaryOp::Remainder)
            && (integer_type(self.typed.expr(*right)) || when_integer)
            && self.divisor_entered(expression.span)
        {
            let range = b.range.clone().unwrap_or_else(Range::unknown);
            if !when_integer && range == Range::exact(0.into()) {
                self.errors.push(Error {
                    span: *operator,
                    reason: TypeReason::Limit {
                        message: String::from(
                            "division by zero: divisor is exact 0",
                        ),
                    },
                });
            } else if range.contains_zero() {
                self.typed.checks.push(Check {
                    value: Some(*right),
                    path: Vec::new(),
                    target: None,
                    place: self.module.expressions[right.0].span,
                    place_kind: Place::Parameter(Some(if when_integer {
                        match &self.module.expressions[right.0].kind {
                            ExprKind::Name(name) => name.segments.join("."),
                            _ => String::from("divisor"),
                        }
                    } else {
                        String::from("divisor")
                    })),
                    limit: *operator,
                    phase: self.site_phase(expression.span),
                    reason: Reason::UnknownInput,
                    frequency: self.frequency(expression.span),
                    place_origin: Origin::This,
                    limit_origin: Origin::This,
                    name: String::from("nonzero divisor"),
                    test_count: 1,
                    known: b.clone(),
                    gate: false,
                    when_integer,
                    owner: self.entry_owner(expression.span),
                });
            }
        }
        if integer_type(self.typed.expr(*left)) {
            binary(*op, &a, &b)
        } else {
            Known::default()
        }
    }
    fn convert_value(&mut self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let Some(callee) = call_head(self.module, id) else {
            return Known::default();
        };
        let get = |id: ExprId| self.typed.known[id.0].clone();

        let Some(Target::Value { name, .. }) = self.resolved.expr(callee)
        else {
            return Known::default();
        };
        let fallible = name != "to";
        let mut known = if name == "read" {
            Known::default()
        } else {
            first_input(self.module, id).map(get).unwrap_or_default()
        };
        let ty = self.typed.expr(id).cloned();
        let target = if fallible {
            ty.as_ref().and_then(|ty| {
                if let Type::Named { arguments, .. } = ty {
                    arguments.first()
                } else {
                    None
                }
            })
        } else {
            ty.as_ref()
        };
        if let Some(target) = target {
            let source = first_input(self.module, id)
                .and_then(|input| self.typed.expr(input));
            if source.is_none_or(|source| {
                crate::infer::meaning(source) != crate::infer::meaning(target)
            }) {
                known = Known {
                    narrowing: true,
                    ..Known::default()
                };
            }
            let start = self.typed.checks.len();
            let errors = self.errors.len();
            self.enter(
                target,
                &known,
                expression.span,
                self.frequency(expression.span),
            );
            for check in &mut self.typed.checks[start..] {
                check.gate = true;
            }
            if fallible && self.errors.len() > errors {
                self.errors.truncate(errors);
                self.typed.checks.truncate(start);
                return Known {
                    constructor: Some(String::from("Err")),
                    parts: vec![(String::from("Err"), Known::default())],
                    ..Known::default()
                };
            }
            self.place_facts(target, &mut known);
        }
        if fallible {
            Known {
                parts: vec![(String::from("Ok"), known)],
                ..Known::default()
            }
        } else {
            known
        }
    }
    fn construct_value(&self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let Some(callee) = call_head(self.module, id) else {
            return Known::default();
        };

        let (origin, ty, name) = match self.resolved.expr(callee) {
            Some(Target::Constructor { origin, ty, name }) => {
                (origin, ty, name)
            }
            Some(Target::SelfType) => {
                let Some(Type::Named { name, .. }) = self.typed.expr(id) else {
                    return Known::default();
                };
                (&name.origin, &name.name, &name.name)
            }
            _ => return Known::default(),
        };
        let Some(module) = self.owner(origin) else {
            return Known::default();
        };
        let written = core::iter::once((self.module, Origin::This))
            .chain(self.frame.into_iter().flat_map(|f| f.lines.iter().enumerate().map(|(i,l)| (&l.module, Origin::Line(i)))))
            .chain(self.files.iter().map(|(n,m)| (m, Origin::File(n.clone()))))
            .any(|(m, o)| m.entries.iter().any(|entry| matches!(entry, Entry::Item(item) if matches!(&item.kind, ItemKind::Impl { target, members, .. } if self.type_name(m, &o, *target).as_ref() == Some(&TypeName { origin: origin.clone(), name: ty.clone() }) && members.iter().any(|member| matches!(member, crate::ast::ImplMember::Constructor { .. }))))));
        if written
            && !matches!(self.resolved.expr(callee), Some(Target::SelfType))
        {
            return Known {
                narrowing: true,
                ..Known::default()
            };
        }
        let fields = construction_fields(module, ty, name);
        let stage = match expression.kind {
            ExprKind::Pipe {
                op: PipeOp::Plain,
                right,
                ..
            } => self.module.expressions[right.0].span,
            _ => expression.span,
        };
        let supplied = self.typed.supplied.get(&(stage.start, stage.end));
        // A field default is read with the fields before it, which it
        // may name, as the construction fills them in order.
        let mut parts: Vec<(String, Known)> = Vec::new();
        for field in fields {
            let name = field.name.segments.last().cloned().unwrap_or_default();
            let value = supplied
                .and_then(|values| values.iter().find(|(n, _)| *n == name))
                .map(|(_, supplied)| self.supply_fact(supplied))
                .or_else(|| {
                    field.default.map(|id| spot_test(module, id, &parts))
                })
                .unwrap_or_default();
            parts.push((name, value));
        }
        Known {
            parts,
            constructor: Some(name.clone()),
            ..Known::default()
        }
    }
    fn gate_value(&mut self, id: ExprId) -> Known {
        let expression = &self.module.expressions[id.0];
        let Some(callee) = call_head(self.module, id) else {
            return Known::default();
        };
        let get = |id: ExprId| self.typed.known[id.0].clone();

        let mut known =
            first_input(self.module, id).map(get).unwrap_or_default();
        if let Some(Target::Type { origin, name }) = self.resolved.expr(callee)
        {
            let name = TypeName {
                origin: origin.clone(),
                name: name.clone(),
            };
            let start = self.typed.checks.len();
            self.named_entry(
                &name,
                &known,
                expression.span,
                self.frequency(expression.span),
                &mut Vec::new(),
            );
            for check in &mut self.typed.checks[start..] {
                check.gate = true;
            }
            self.named_facts(&name, &mut known, &mut Vec::new());
        }
        known
    }
    fn host_fact(&self, origin: &Origin, name: &str) -> Known {
        self.owner(origin)
            .and_then(|module| {
                phase::binding(module, name)
                    .map(|id| test(module, id, &Known::default()))
            })
            .unwrap_or_default()
    }
    fn target(&self, target: Option<&Target>) -> Known {
        match target {
            Some(Target::Local { site }) => self
                .locals
                .get(&(site.start, site.end))
                .or_else(|| {
                    self.frame
                        .and_then(|f| f.facts.get(&(site.start, site.end)))
                })
                .cloned()
                .unwrap_or_default(),
            Some(Target::Value { origin, name })
                if matches!(origin, Origin::Host(_) | Origin::Commands) =>
            {
                self.host_fact(origin, name)
            }
            Some(Target::Value {
                origin: Origin::File(stem),
                name,
            }) => {
                let Some((_, module)) =
                    self.files.iter().find(|(n, _)| n == stem)
                else {
                    return Known::default();
                };
                let cache = self.cache.files.borrow();
                let Some(checked) = cache.get(stem) else {
                    return Known::default();
                };
                if !checked.resolve_errors.is_empty()
                    || !checked.type_errors.is_empty()
                {
                    return Known {
                        failure: Some(known::Failure::Dependency),
                        narrowing: true,
                        ..Known::default()
                    };
                }
                let mut fact = module
                    .entries
                    .iter()
                    .rev()
                    .find_map(|entry| {
                        let Entry::Line(id) = entry else {
                            return None;
                        };
                        let ExprKind::Bind { pattern, .. } =
                            module.expressions[id.0].kind
                        else {
                            return None;
                        };
                        let PatternKind::Name(n) =
                            &module.patterns[pattern.0].kind
                        else {
                            return None;
                        };
                        if n.segments.last() != Some(name) {
                            return None;
                        }
                        checked
                            .typed
                            .facts
                            .iter()
                            .find(|(site, _)| {
                                *site == (n.span.start, n.span.end)
                            })
                            .map(|(_, known)| known.clone())
                    })
                    .unwrap_or_default();
                fact.rehome(&Origin::File(stem.clone()));
                fact
            }
            Some(Target::Members { head, members }) => {
                let mut fact = self.target(Some(head));
                for member in members {
                    fact = fact.part(member);
                }
                fact
            }
            Some(Target::Constructor {
                origin: Origin::Prelude,
                name,
                ..
            }) if name == "True" || name == "False" => Known {
                boolean: Some(name == "True"),
                ..Known::default()
            },
            Some(Target::Constructor { name, .. }) => Known {
                constructor: Some(name.clone()),
                ..Known::default()
            },
            _ => Known::default(),
        }
    }
    fn place_facts(&self, ty: &Type, known: &mut Known) {
        if let Type::Named { name, .. } = crate::infer::meaning(ty) {
            known.meaning = Some(name.clone());
        }
        if let Type::Tuple(parts) = ty {
            for (i, ty) in parts.iter().enumerate() {
                let label = i.to_string();
                let index = known
                    .parts
                    .iter()
                    .position(|(n, _)| *n == label)
                    .unwrap_or_else(|| {
                        known.parts.push((label, Known::default()));
                        known.parts.len() - 1
                    });
                self.place_facts(ty, &mut known.parts[index].1);
            }
        }
        if let Type::Array { length, .. } = ty {
            if let Some(n) = length.number() {
                known.parts.retain(|(name, _)| name != "len");
                known.parts.push((
                    String::from("len"),
                    exact(Integer {
                        negative: false,
                        magnitude: n.clone(),
                    }),
                ));
            }
        }
        if let Type::Limited { name, .. } = ty {
            self.limit_facts(name, known);
        }
    }
    fn limit_facts(&self, ty: &Type, known: &mut Known) {
        match ty {
            Type::Limited { name, .. } => self.limit_facts(name, known),
            Type::Named { name, .. } => {
                self.named_facts(name, known, &mut Vec::new());
            }
            Type::Both(parts) => {
                for part in parts {
                    self.limit_facts(part, known);
                }
            }
            _ => (),
        }
    }
    fn named_facts(
        &self,
        name: &TypeName,
        known: &mut Known,
        visited: &mut Vec<TypeName>,
    ) {
        if visited.contains(name) {
            return;
        }
        visited.push(name.clone());
        if let Some((module, base, limits)) = self.declaration(name) {
            let has_test = limits.iter().any(|l| matches!(l, Limit::Test(_)));
            let has_members = self.declarations_by_name.get(name).is_some_and(|i| matches!(&module.entries[*i], Entry::Item(item) if matches!(item.kind, ItemKind::Type { members: Some(_), .. })));
            if !has_test
                && (has_members
                    || limits.iter().any(|l| matches!(l, Limit::Member { .. })))
            {
                return;
            }

            if let Some(base) = base {
                self.base_facts(module, &name.origin, base, known, visited);
            }
            for limit in limits {
                if let Limit::Test(id) = limit {
                    self.narrow_test(module, &name.origin, *id, known);
                }
            }
        }
        if self.declaration(name).is_some_and(|(_, _, limits)| {
            limits.iter().any(|limit| matches!(limit, Limit::Test(_)))
        }) && !known.names.contains(name)
        {
            known.names.push(name.clone());
        }
    }
    fn base_facts(
        &self,
        module: &Module,
        origin: &Origin,
        id: TypeId,
        known: &mut Known,
        visited: &mut Vec<TypeName>,
    ) {
        match &module.types[id.0].kind {
            TypeKind::Both(parts) => {
                for p in parts {
                    self.base_facts(module, origin, *p, known, visited);
                }
            }
            TypeKind::Named { .. } => {
                if let Some(name) = self.type_name(module, origin, id) {
                    if name.origin == Origin::Prelude
                        && matches!(
                            name.name.as_str(),
                            "Integer" | "F32" | "F64"
                        )
                    {
                        known.meaning = Some(name.clone());
                    }
                    if self.declaration(&name).is_some_and(
                        |(_, base, limits)| {
                            base.is_some() || !limits.is_empty()
                        },
                    ) && !matches!(name.name.as_str(), "Bits" | "Size")
                    {
                        self.named_facts(&name, known, visited);
                    }
                }
            }
            _ => (),
        }
    }
}

fn integer_type(ty: Option<&Type>) -> bool {
    match ty {
        Some(Type::Limited { meaning, .. }) => integer_type(Some(meaning)),
        Some(Type::Named { name, .. }) => {
            name.origin == Origin::Prelude && name.name == "Integer"
        }
        _ => false,
    }
}
fn exact(n: Integer) -> Known {
    Known {
        range: Some(Range::exact(n)),
        ..Known::default()
    }
}
fn literal_fact(literal: &Literal) -> Known {
    match literal {
        Literal::Int {
            negative,
            magnitude,
            ..
        } => exact(Integer::new(*negative, magnitude.clone())),
        Literal::Str { value, .. } => Known {
            exact: Some(format!("{value:?}")),
            parts: vec![(
                String::from("len"),
                exact(Integer::from(value.len() as u64)),
            )],
            ..Known::default()
        },
        _ => Known::default(),
    }
}
fn test_binary(op: BinaryOp, a: &Known, b: &Known) -> Known {
    if a.failure == Some(known::Failure::Fault)
        || b.failure == Some(known::Failure::Fault)
    {
        return Known {
            failure: Some(known::Failure::Fault),
            ..Known::default()
        };
    }
    if matches!(op, BinaryOp::Divide | BinaryOp::Remainder)
        && b.range.as_ref().is_none_or(Range::contains_zero)
    {
        return Known {
            failure: (b.range == Some(Range::exact(0.into())))
                .then_some(known::Failure::Fault),
            ..Known::default()
        };
    }
    binary(op, a, b)
}
fn binary(op: BinaryOp, a: &Known, b: &Known) -> Known {
    if a.failure == Some(known::Failure::Dependency)
        || b.failure == Some(known::Failure::Dependency)
    {
        return Known {
            failure: Some(known::Failure::Dependency),
            narrowing: true,
            ..Known::default()
        };
    }
    let narrowing = a.narrowing || b.narrowing;
    let (a, b) = (
        a.range.clone().unwrap_or_else(Range::unknown),
        b.range.clone().unwrap_or_else(Range::unknown),
    );
    if matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder
    ) {
        Known {
            range: Some(a.arithmetic(op, &b)),
            narrowing,
            ..Known::default()
        }
    } else {
        Known {
            boolean: a.compare(op, &b),
            narrowing,
            ..Known::default()
        }
    }
}
fn bool_op(op: PipeOp, a: &Known, b: &Known) -> Known {
    if a.failure == Some(known::Failure::Fault)
        || (b.failure == Some(known::Failure::Fault)
            && !matches!(
                (op, a.boolean),
                (PipeOp::And, Some(false)) | (PipeOp::Or, Some(true))
            ))
    {
        return Known {
            failure: Some(known::Failure::Fault),
            ..Known::default()
        };
    }
    let boolean = match (op, a.boolean, b.boolean) {
        (PipeOp::And, Some(false), _) | (PipeOp::And, _, Some(false)) => {
            Some(false)
        }
        (PipeOp::Or, Some(true), _) | (PipeOp::Or, _, Some(true)) => Some(true),
        (PipeOp::And | PipeOp::Or, Some(a), Some(b)) => {
            Some(if op == PipeOp::And { a && b } else { a || b })
        }
        _ => None,
    };
    Known {
        boolean,
        narrowing: a.narrowing || b.narrowing,
        ..Known::default()
    }
}
fn test(module: &Module, id: ExprId, value: &Known) -> Known {
    match &module.expressions[id.0].kind {
        ExprKind::Literal(l) => literal_fact(l),
        ExprKind::Name(n)
            if n.segments.first().is_some_and(|s| s == "self") =>
        {
            let mut k = value.clone();
            for part in &n.segments[1..] {
                k = k
                    .parts
                    .iter()
                    .find(|(n, _)| n == part)
                    .map_or_else(Known::default, |(_, k)| k.clone());
            }
            k
        }
        ExprKind::Field { value: v, field } => test(module, *v, value)
            .parts
            .into_iter()
            .find(|(n, _)| Some(n) == field.segments.last())
            .map_or_else(Known::default, |(_, k)| k),
        ExprKind::Binary {
            op, left, right, ..
        } => test_binary(
            *op,
            &test(module, *left, value),
            &test(module, *right, value),
        ),
        ExprKind::Pipe {
            op, left, right, ..
        } if matches!(op, PipeOp::And | PipeOp::Or) => bool_op(
            *op,
            &test(module, *left, value),
            &test(module, *right, value),
        ),
        ExprKind::Not { value: v, .. } => {
            let mut result = test(module, *v, value);
            result.boolean = result.boolean.map(|b| !b);
            result
        }
        ExprKind::Group(ids) => ids
            .last()
            .map_or_else(Known::default, |id| test(module, *id, value)),
        ExprKind::Call { .. } => Known {
            narrowing: true,
            ..Known::default()
        },
        _ => Known::default(),
    }
}
/// Evaluate a default's expression with the facts of the spots it may
/// name: an earlier field or parameter by its bare name.
fn spot_test(module: &Module, id: ExprId, spots: &[(String, Known)]) -> Known {
    match &module.expressions[id.0].kind {
        ExprKind::Literal(l) => literal_fact(l),
        ExprKind::Name(n) if n.segments.len() == 1 => spots
            .iter()
            .find(|(name, _)| *name == n.segments[0])
            .map_or_else(Known::default, |(_, k)| k.clone()),
        ExprKind::Field { value, field } => spot_test(module, *value, spots)
            .parts
            .into_iter()
            .find(|(n, _)| Some(n) == field.segments.last())
            .map_or_else(Known::default, |(_, k)| k),
        ExprKind::Binary {
            op, left, right, ..
        } => test_binary(
            *op,
            &spot_test(module, *left, spots),
            &spot_test(module, *right, spots),
        ),
        ExprKind::Group(ids) => ids
            .last()
            .map_or_else(Known::default, |id| spot_test(module, *id, spots)),
        _ => Known::default(),
    }
}
impl Pass<'_> {
    fn narrow_test(
        &self,
        module: &Module,
        origin: &Origin,
        id: ExprId,
        known: &mut Known,
    ) {
        if let Some((_, value)) = self.timed_test(module, origin, id) {
            self.narrow_test(module, origin, value, known);
            return;
        }
        if let ExprKind::Pipe {
            op: PipeOp::And,
            left,
            right,
            ..
        } = module.expressions[id.0].kind
        {
            self.narrow_test(module, origin, left, known);
            self.narrow_test(module, origin, right, known);
            return;
        }
        if let ExprKind::Group(ids) = &module.expressions[id.0].kind {
            if let Some(id) = ids.last() {
                self.narrow_test(module, origin, *id, known);
            }
            return;
        }
        let ExprKind::Binary {
            op, left, right, ..
        } = module.expressions[id.0].kind
        else {
            return;
        };
        let path = |id: ExprId| match &module.expressions[id.0].kind {
            ExprKind::Name(n)
                if n.segments.first().is_some_and(|s| s == "self") =>
            {
                Some(n.segments[1..].to_vec())
            }
            _ => None,
        };
        let (op, value, path) = if let Some(path) = path(left) {
            (op, right, path)
        } else if let Some(path) = path(right) {
            (
                match op {
                    BinaryOp::Less => BinaryOp::Greater,
                    BinaryOp::LessEqual => BinaryOp::GreaterEqual,
                    BinaryOp::Greater => BinaryOp::Less,
                    BinaryOp::GreaterEqual => BinaryOp::LessEqual,
                    other => other,
                },
                left,
                path,
            )
        } else {
            return;
        };
        let Some(Range {
            lo: End::Finite(n),
            hi: End::Finite(hi),
        }) = self
            .test_value(module, origin, value, &Known::default())
            .range
        else {
            return;
        };
        if n != hi {
            return;
        }
        let one = Integer::from(1);
        let range = match op {
            BinaryOp::Less => Range {
                lo: End::Negative,
                hi: End::Finite(n.add(&one.negated())),
            },
            BinaryOp::LessEqual => Range {
                lo: End::Negative,
                hi: End::Finite(n),
            },
            BinaryOp::Greater => Range {
                lo: End::Finite(n.add(&one)),
                hi: End::Positive,
            },
            BinaryOp::GreaterEqual => Range {
                lo: End::Finite(n),
                hi: End::Positive,
            },
            BinaryOp::Equal => Range::exact(n),
            _ => return,
        };
        narrow_path(known, path, &range);
    }
}

pub(crate) fn has_limits(ty: &Type) -> bool {
    match ty {
        Type::Limited { .. } => true,
        Type::Named { arguments, .. } => arguments.iter().any(has_limits),
        Type::Array { element, .. } => has_limits(element),
        Type::Tuple(parts) | Type::Both(parts) | Type::Either(parts) => {
            parts.iter().any(has_limits)
        }
        Type::Not(inner) => has_limits(inner),
        Type::Function { parameter, result } => {
            has_limits(parameter) || has_limits(result)
        }
        _ => false,
    }
}

fn construction_fields<'a>(
    module: &'a Module,
    ty: &str,
    name: &str,
) -> Vec<&'a crate::ast::Field> {
    module
        .entries
        .iter()
        .find_map(|entry| {
            let Entry::Item(item) = entry else {
                return None;
            };
            let ItemKind::Type {
                name: n,
                members: Some(members),
                ..
            } = &item.kind
            else {
                return None;
            };
            if n.segments.last().is_none_or(|n| n != ty) {
                return None;
            }
            let fields: Vec<_> = if ty == name {
                members
                    .iter()
                    .filter_map(|m| {
                        if let crate::ast::Member::Field(field) = m {
                            Some(field)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                members
                    .iter()
                    .find_map(|m| {
                        if let crate::ast::Member::Constructor {
                            name: n,
                            fields,
                            ..
                        } = m
                        {
                            (n.segments.last().is_some_and(|n| n == name))
                                .then_some(fields.iter().collect())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default()
            };
            Some(fields)
        })
        .unwrap_or_default()
}

fn call_head(module: &Module, id: ExprId) -> Option<ExprId> {
    match module.expressions[id.0].kind {
        ExprKind::Call { callee, .. } => Some(callee),
        ExprKind::Pipe {
            op: PipeOp::Plain,
            right,
            ..
        } => Some(call_head(module, right).unwrap_or(right)),
        _ => None,
    }
}

fn first_input(module: &Module, id: ExprId) -> Option<ExprId> {
    match &module.expressions[id.0].kind {
        ExprKind::Call { arguments, .. } => {
            arguments.first().and_then(|argument| match argument.kind {
                crate::ast::ArgumentKind::Positional(id) => Some(id),
                crate::ast::ArgumentKind::Flag { .. } => None,
            })
        }
        ExprKind::Pipe {
            op: PipeOp::Plain,
            left,
            ..
        } => Some(*left),
        _ => None,
    }
}

pub(crate) fn frequency(module: &Module, span: Span) -> Frequency {
    if module.expressions.iter().any(|e| {
        matches!(e.kind, ExprKind::Arms(_))
            && e.span.start <= span.start
            && span.end <= e.span.end
    }) {
        Frequency::PerCall
    } else {
        Frequency::Once
    }
}

/// These limits carry no meaning and never participate in unification.
pub(crate) fn modifier(ty: &Type) -> bool {
    matches!(ty, Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "Bits" | "Size" | "Compile" | "Init"))
}

fn narrow_path(known: &mut Known, path: Vec<String>, range: &Range) {
    let mut part = known;
    for name in path {
        let index = part
            .parts
            .iter()
            .position(|(n, _)| *n == name)
            .unwrap_or_else(|| {
                part.parts.push((name, Known::default()));
                part.parts.len() - 1
            });
        part = &mut part.parts[index].1;
    }
    if !part
        .meaning
        .as_ref()
        .is_some_and(|n| n.origin == Origin::Prelude && n.name == "Integer")
        && part.range.is_none()
    {
        return;
    }
    part.range
        .get_or_insert_with(Range::unknown)
        .intersect(range);
}
