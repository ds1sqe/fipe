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
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use known::{End, Integer, Known, Range};

type Site = (usize, usize);

/// Where a condition, guard or `&&` left side narrows: the bodies after
/// it, or by its negation the else branch and later arms (07-checks
/// §What is known 4).
#[derive(Clone, Debug)]
enum Narrows {
    Body {
        span: Span,
        holds: bool,
    },
    /// The later arms of an arm list, by a guard's negation at the same
    /// parameter position.
    Later {
        arms: ExprId,
        index: usize,
    },
    /// The arms a bare name is piped into, by their literal patterns.
    Piped {
        arms: ExprId,
    },
}

/// A local, or its `len` when `len` holds, narrowed to a range,
/// for the reads inside `span`.
#[derive(Clone, Debug)]
struct Narrowed {
    span: Span,
    site: Site,
    len: bool,
    range: Range,
}

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

/// A module's bindings, found once: by the site of a binding's pattern
/// (the first such binding), and by a top-level line's name (the last).
#[derive(Clone, Debug, Default)]
pub(crate) struct Bindings {
    sites: BTreeMap<Site, ExprId>,
    names: BTreeMap<String, ExprId>,
    /// Whether the module writes an `impl`.
    pub(crate) impls: bool,
    /// Whether the module declares a type.
    pub(crate) types: bool,
}

impl Bindings {
    pub(crate) fn of(module: &Module) -> Self {
        let mut bindings = Self::default();
        for expression in &module.expressions {
            if let ExprKind::Bind { pattern, value, .. } = expression.kind {
                let span = module.patterns[pattern.0].span;
                bindings
                    .sites
                    .entry((span.start, span.end))
                    .or_insert(value);
            }
        }
        bindings.impls = module.entries.iter().any(|entry| {
            matches!(entry, Entry::Item(item) if matches!(item.kind, ItemKind::Impl { .. }))
        });
        bindings.types = module.entries.iter().any(|entry| {
            matches!(entry, Entry::Item(item) if matches!(item.kind, ItemKind::Type { .. }))
        });
        for entry in &module.entries {
            let Entry::Line(id) = entry else { continue };
            let ExprKind::Bind { pattern, value, .. } =
                module.expressions[id.0].kind
            else {
                continue;
            };
            if let PatternKind::Name(name) = &module.patterns[pattern.0].kind {
                if let Some(last) = name.segments.last() {
                    bindings.names.insert(last.clone(), value);
                }
            }
        }
        bindings
    }

    /// The value bound by the pattern at `site`.
    pub(crate) fn site(&self, site: Span) -> Option<ExprId> {
        self.sites.get(&(site.start, site.end)).copied()
    }

    /// The value of the module's last top-level binding of `name`.
    pub(crate) fn name(&self, name: &str) -> Option<ExprId> {
        self.names.get(name).copied()
    }
}

pub(crate) fn check(
    inputs: &Inputs<'_>,
    entries: &[(ExprId, Type, Span, Place, bool)],
    instance: Option<ExprId>,
    typed: &mut Typed,
) -> Vec<Error> {
    let module = inputs.module;
    let mut pass = Pass::new(inputs, typed, Place::Result);
    pass.instance = instance;
    pass.typed.known = vec![Known::default(); module.expressions.len()];
    let conditions = conditions(module);
    pass.seed_places();
    pass.check_purity();
    let mut entry_types = vec![None; module.expressions.len()];
    for (id, ty, ..) in entries {
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
        pass.settle_default_sites(id);
        pass.expression_holding(id, entry_types[index]);
        pass.typed.known[index] = known.clone();
        pass.binding_facts(id);
        for narrows in conditions.get(&id.0).into_iter().flatten() {
            pass.narrow(id, narrows);
        }
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

/// Whether a function known by its declared type `actual` enters a
/// place of type `expected` (07-checks §What is known 8).
pub(crate) fn function_proved(
    inputs: &Inputs<'_>,
    expected: &Type,
    actual: &Type,
) -> Option<bool> {
    let mut typed = Typed::default();
    let mut pass = Pass::new(inputs, &mut typed, Place::Result);
    let mut known = Known::default();
    pass.place_facts(actual, &mut known);
    pass.function_outcome(
        expected,
        Some(actual),
        &known,
        (&[], 0),
        Span::default(),
    )
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
                .filter(|(_, l)| l.bindings.types)
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
    /// The bindings of each module this pass looked into, by address.
    bindings: core::cell::RefCell<BTreeMap<usize, Rc<Bindings>>>,
    /// Each reachable type declaration's entry, found when first read.
    declarations_by_name: core::cell::OnceCell<BTreeMap<TypeName, usize>>,
    contexts: BTreeMap<usize, (Phase, Frequency, Option<String>)>,
    checks_seen:
        BTreeSet<(Span, Place, Span, Origin, Vec<crate::ir::Projection>)>,
    /// The one declared expression this pass types for a use site, a
    /// default for a call or construction or a limited type's test.
    instance: Option<ExprId>,
    /// The ranges conditions narrowed locals to so far, met by each read
    /// they hold.
    narrowed: Vec<Narrowed>,
    /// What is known at each parameter of a function argument that is a
    /// variable of its callee's scheme, by the argument's index.
    takes: BTreeMap<usize, Vec<Option<Known>>>,
    /// The type of the value entering, as far as it is followed into
    /// its parts.
    value_ty: Option<Type>,
    /// The entering value is of the record place's own type, so its
    /// fields have the place's field types.
    value_is_place: bool,
    /// The open parameters of the function entry being proved.
    opens: Vec<(usize, Type)>,
    /// The arm lists with a signature: an annotated binding's value or
    /// an `impl` member's.
    signed: BTreeSet<usize>,
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
            bindings: core::cell::RefCell::new(BTreeMap::new()),
            declarations_by_name: core::cell::OnceCell::new(),
            contexts: contexts(inputs.module),
            checks_seen: BTreeSet::new(),
            instance: None,
            narrowed: Vec::new(),
            takes: BTreeMap::new(),
            value_ty: None,
            value_is_place: false,
            opens: Vec::new(),
            signed: signed(inputs.module),
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
        // Text shown or taken as bytes is `Vec<U8>` by construction, so
        // its elements meet that place; only their count is unknown.
        if cast.as_ref().is_some_and(|cast| {
            matches!(
                cast.row,
                crate::typed::CastRow::Bytes | crate::typed::CastRow::Show
            ) && bytes_place(&cast.to)
        }) {
            known.element = Some(alloc::boxed::Box::new(self.byte_facts()));
        }
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
        let in_type_item = self.module.entries.iter().any(|entry| {
            let Entry::Item(item) = entry else {
                return false;
            };
            matches!(item.kind, ItemKind::Type { .. }) && within(item.span)
        });
        !in_type_item
            || self
                .field_default_of(span)
                .is_some_and(|default| self.instance == Some(default))
    }

    /// The record field default an expression stands in, if any.
    fn field_default_of(&self, span: Span) -> Option<ExprId> {
        let within =
            |outer: Span| outer.start <= span.start && span.end <= outer.end;
        self.module.entries.iter().find_map(|entry| {
            let Entry::Item(item) = entry else {
                return None;
            };
            let ItemKind::Type { members, .. } = &item.kind else {
                return None;
            };
            members.iter().flatten().find_map(|member| {
                let crate::ast::Member::Field(field) = member else {
                    return None;
                };
                field.default.filter(|default| {
                    within(self.module.expressions[default.0].span)
                })
            })
        })
    }

    /// A default's open entries are listed where it is applied: at each
    /// construction or call, with that site's phase and frequency, as
    /// the checker adopted them. The declaration keeps only what it
    /// decides.
    fn settle_default_sites(&mut self, id: ExprId) {
        let Some(stage) = self.binding_stage(id) else {
            return;
        };
        let phase = self.site_phase(stage);
        let frequency = self.frequency(stage);
        // The checks at this stage now are the adopted ones: the pass
        // enters the stage's own places after this loop.
        for check in &mut self.typed.checks {
            if check.place != stage || check.phase == Phase::Compile {
                continue;
            }
            check.frequency = frequency;
            check.phase = if frequency == Frequency::PerCall {
                Phase::Eval
            } else {
                phase
            };
        }
    }

    /// The facts a call or construction supplies to each spot by name,
    /// in declaration order; a spot left to its declared default says
    /// what the default says, read with the spots before it.
    fn supplied_facts(&self, stage: Span) -> Vec<(String, Known)> {
        let mut parts: Vec<(String, Known)> = Vec::new();
        let Some(values) = self.typed.supplied.get(&(stage.start, stage.end))
        else {
            return parts;
        };
        for (name, supply) in values {
            let fact = match supply {
                crate::infer::Supply::Declared(origin, id) => {
                    self.owner(origin).map_or_else(Known::default, |module| {
                        spot_test(module, *id, &parts)
                    })
                }
                other => self.supply_fact(other),
            };
            parts.push((name.clone(), fact));
        }
        parts
    }

    fn supply_fact(&self, supply: &crate::infer::Supply) -> Known {
        let get = |id: ExprId| self.typed.known[id.0].clone();
        match supply {
            crate::infer::Supply::Declared(..) => Known::default(),
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
    /// The stage an expression binds spots at: a call, a construction,
    /// a pipe's right side, or a bare callable whose spots all default.
    fn binding_stage(&self, id: ExprId) -> Option<Span> {
        let expression = &self.module.expressions[id.0];
        match expression.kind {
            ExprKind::Pipe {
                op: PipeOp::Plain,
                right,
                ..
            } => Some(self.module.expressions[right.0].span),
            ExprKind::Call { .. } | ExprKind::Record { .. } => {
                Some(expression.span)
            }
            ExprKind::Name(_)
            | ExprKind::Instance { .. }
            | ExprKind::Field { .. } => self
                .typed
                .supplied
                .contains_key(&(expression.span.start, expression.span.end))
                .then_some(expression.span),
            _ => None,
        }
    }

    fn settle_default_divisors(&mut self, id: ExprId) {
        let Some(stage) = self.binding_stage(id) else {
            return;
        };
        let expression = &self.module.expressions[id.0];
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
        // Parameters are places: seed their written limits before reading
        // bodies, for every bound arm list, nested ones included, and
        // each `impl` member with a signature.
        for index in 0..self.module.expressions.len() {
            self.seed(ExprId(index));
        }
        for entry in &self.module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl { members, .. } = &item.kind else {
                continue;
            };
            for member in members {
                if let crate::ast::ImplMember::Named {
                    annotation: Some(_),
                    value: Some(value),
                    ..
                } = member
                {
                    self.seed_member(*value);
                }
            }
        }
    }

    /// An `impl` member's parameters, by the types its signature gives
    /// their patterns.
    fn seed_member(&mut self, value: ExprId) {
        let ExprKind::Arms(arms) = &self.module.expressions[value.0].kind
        else {
            return;
        };
        for arm in arms {
            for pattern in &arm.patterns {
                if let (PatternKind::Name(name), Some(ty)) = (
                    &self.module.patterns[pattern.0].kind,
                    self.typed.pattern(*pattern),
                ) {
                    let mut known = Known {
                        pure: true,
                        ..Known::default()
                    };
                    self.place_facts(ty, &mut known);
                    self.locals.insert((name.span.start, name.span.end), known);
                }
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
            if matches!(
                self.module.expressions[value.0].kind,
                ExprKind::Arms(_)
            ) {
                if let (Some(function), Some(scheme)) =
                    (fact.function.as_deref_mut(), self.typed.binding(id))
                {
                    function.feeds = feeds(scheme, function.parameters.len());
                    function.takes = takes(scheme, function.parameters.len());
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

    fn entries(&mut self, entries: &[(ExprId, Type, Span, Place, bool)]) {
        let module = self.module;
        let branches: Vec<_> = entries
            .iter()
            .filter(|(id, ..)| {
                matches!(module.expressions[id.0].kind, ExprKind::If { .. })
            })
            .collect();
        // A binding without an annotation takes its value as it is.
        let bound: BTreeMap<_, _> = module
            .expressions
            .iter()
            .filter_map(|e| match e.kind {
                ExprKind::Bind {
                    value, annotation, ..
                } => Some((value.0, annotation.is_some())),
                _ => None,
            })
            .collect();
        for (id, ty, place, place_kind, variable) in entries {
            if !has_limits(ty)
                && !self.typed.expr(*id).is_some_and(has_limits)
                && (bound.get(&id.0) == Some(&false)
                    || !self.typed.known[id.0].carries())
            {
                continue;
            }
            // A function value at a type variable is what is known there
            // (07-checks §What is known 8): it meets nothing but itself.
            if *variable && matches!(ty, Type::Function { .. }) {
                self.variable_function(*id, ty, *place, place_kind);
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
                |(outer, outer_ty, outer_place, ..)| {
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
                && bound.get(&id.0) == Some(&true)
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
            self.value_ty = self.typed.expr(*id).cloned();
            let takes = self.takes.get(&id.0).cloned();
            self.enter_value(
                ty,
                &fact,
                *place,
                self.frequency(expr.span),
                takes.as_deref(),
            );
            self.value_ty = None;
            // A field default's open entries are listed per construction
            // site, not at the declaration; what the declaration decides
            // it has reported.
            if self
                .field_default_of(expr.span)
                .is_some_and(|default| self.instance != Some(default))
            {
                self.typed.checks.truncate(start);
                continue;
            }
            for check in &mut self.typed.checks[start..] {
                check.value = Some(*id);
            }
        }
    }

    /// A function value entering a type variable, which drops its
    /// parameter limits from its type: proved when its knowledge comes
    /// out as the call's result, so later calls test by it (07-checks
    /// §What is known 6, 8); otherwise its limits are an open entry,
    /// tested on each call of the value.
    fn variable_function(
        &mut self,
        id: ExprId,
        ty: &Type,
        place: Span,
        place_kind: &Place,
    ) {
        let known = self.typed.known[id.0].clone();
        let opens: Vec<_> = own_limits(&known, self.typed.expr(id))
            .into_iter()
            .enumerate()
            .filter_map(|(i, limit)| Some((i, limit?)))
            .collect();
        if opens.is_empty() || self.carried(id) {
            return;
        }
        self.place_kind = place_kind.clone();
        let start = self.typed.checks.len();
        let frequency = self.frequency(self.module.expressions[id.0].span);
        self.open_function(ty, &opens, &known, place, frequency);
        for check in &mut self.typed.checks[start..] {
            check.value = Some(id);
        }
    }
    /// Whether a value is an argument of a full call whose result is the
    /// type variable it goes in at: the result is then known by it.
    fn carried(&self, value: ExprId) -> bool {
        // ponytail: a scan of the later expressions per such entry; index
        // arguments by call if a long file slows.
        (value.0 + 1..self.module.expressions.len()).any(|index| {
            let call = ExprId(index);
            let Some(callee) = call_head(self.module, call) else {
                return false;
            };
            let Some(feeds) = self.typed.known[callee.0]
                .function
                .as_deref()
                .and_then(|f| f.feeds.as_ref())
            else {
                return false;
            };
            let Some(bound) = self
                .binding_stage(call)
                .and_then(|s| self.typed.bound.get(&(s.start, s.end)))
            else {
                return false;
            };
            bound.open.is_empty()
                && bound.supplies.iter().any(|(spot, _)| {
                    self.argument(call, bound, *spot)
                        .map(|a| last_value(self.module, a))
                        == Some(value)
                        && feeds.contains(&known::Feed::Value(*spot))
                })
        })
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
        let index = *self.declarations_by_name().get(name)?;
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
        let local = self.declarations_by_name().contains_key(&TypeName {
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
                                let meaning = self.declarations_by_name().get(name)
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
            ExprKind::Name(_) => {
                self.target(self.resolved.expr(id), Some(expression.span))
            }
            ExprKind::Field { value, field } => get(*value)
                .part(field.segments.last().map_or("", String::as_str)),
            ExprKind::Group(lines) | ExprKind::Block(lines) => {
                lines.last().map_or_else(Known::default, |id| get(*id))
            }
            ExprKind::List(items) => {
                // A list made into another sequence type is cast, known
                // by that type's meaning alone (07-checks §What is known
                // 10).
                if self.typed.expr(id).is_some_and(|ty| !matches!(crate::infer::meaning(ty), Type::Array { .. }) && !matches!(crate::infer::meaning(ty), Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "Vec" | "Deque"))) {
                    return Known::default();
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
            ExprKind::If {
                then_body,
                else_body,
                ..
            } => else_body.map_or_else(Known::default, |else_body| {
                get(*then_body).join(&get(else_body))
            }),
            // A marked pipe is its left value or the stage's result
            // (07-checks §What is known 7). A function stage's result is
            // rebuilt into the left's type, whose join keeps nothing but
            // its being dead.
            ExprKind::Pipe { left, right, .. } => {
                let stage = get(*right);
                let stage = match &stage.function {
                    Some(f) if f.result.is_dead() => f.result.clone(),
                    _ if matches!(
                        self.typed.expr(*right),
                        Some(Type::Function { .. })
                    ) =>
                    {
                        Known::default()
                    }
                    _ => stage,
                };
                get(*left).join(&stage)
            }
            ExprKind::Arms(arms) => self.arms_value(id, arms),
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
            // `fault` never returns: its branch is empty.
            Some(Target::Value {
                origin: Origin::Prelude,
                name,
            }) if name == "fault" => Known {
                failure: Some(known::Failure::Never),
                ..Known::default()
            },
            _ => {
                self.record_takes(id);
                self.applied(id)
            }
        }
    }
    /// What is known at each variable parameter of this call's function
    /// arguments (07-checks §What is known 8), kept for their entries.
    fn record_takes(&mut self, id: ExprId) {
        let Some(callee) = call_head(self.module, id) else {
            return;
        };
        let Some(function) = self.typed.known[callee.0].function.as_deref()
        else {
            return;
        };
        if function.takes.is_empty() {
            return;
        }
        let Some(bound) = self
            .binding_stage(id)
            .and_then(|s| self.typed.bound.get(&(s.start, s.end)))
        else {
            return;
        };
        let mut found = Vec::new();
        for take in &function.takes {
            if let Some(argument) = self.argument(id, bound, take.spot) {
                found.push((
                    last_value(self.module, argument).0,
                    take.parameter,
                    self.fed(id, bound, &take.feeds),
                ));
            }
        }
        for (argument, parameter, known) in found {
            let at = self.takes.entry(argument).or_default();
            if at.len() <= parameter {
                at.resize(parameter + 1, None);
            }
            at[parameter] = Some(known);
        }
    }
    /// What an arm list is known as (07-checks §What is known 5, 8): its
    /// declared parameters' facts, and its result: the join of its
    /// bodies, read once at those parameters, met with the declared
    /// result. An arm list with no signature, as one checked against
    /// a parameter's function type, declares nothing: it is known by its
    /// body alone.
    fn arms_value(&self, id: ExprId, arms: &[crate::ast::Arm]) -> Known {
        let Some(mut ty) = self.typed.expr(id) else {
            return Known::default();
        };
        let signed = self.signed.contains(&id.0);
        let mut parameters = Vec::new();
        for _ in arms.first().map_or(&[][..], |arm| &arm.patterns) {
            let Type::Function { parameter, result } = ty else {
                return Known::default();
            };
            parameters.push(
                (signed && has_limits(parameter))
                    .then(|| (**parameter).clone()),
            );
            ty = result;
        }
        let mut declared = Known::default();
        if signed {
            self.place_facts(ty, &mut declared);
        }
        let result = arms
            .iter()
            .map(|arm| self.typed.known[arm.body.0].clone())
            .reduce(|a, b| a.join(&b))
            .unwrap_or_default()
            .result()
            .meet(&declared);
        Known {
            function: Some(alloc::boxed::Box::new(known::Function {
                result,
                ..known::Function::of(parameters)
            })),
            ..Known::default()
        }
    }
    /// What the declared type of a call's value says of it.
    fn declared(&self, id: ExprId) -> Known {
        let mut known = Known::default();
        if let Some(ty) = self.typed.expr(id) {
            self.place_facts(ty, &mut known);
        }
        known
    }
    /// A call of a function value (07-checks §What is known 5, 6, 8): its
    /// declared result, met with what the callee is known to give; each
    /// argument is tested against the callee's known parameter limits
    /// where its spot's type does not carry them. A callee not yet read,
    /// in a cycle or past the checked unit, is known by the declared
    /// result alone; a partial call is a function of the remaining
    /// parameters.
    fn applied(&mut self, id: ExprId) -> Known {
        let declared = self.declared(id);
        let Some(callee) = call_head(self.module, id) else {
            return declared;
        };
        let head = self.typed.known[callee.0].clone();
        if head.failure == Some(known::Failure::Dependency) {
            return Known {
                failure: head.failure,
                ..Known::default()
            };
        }
        let Some(function) = head.function.as_deref() else {
            return declared;
        };
        let Some(bound) = self
            .binding_stage(id)
            .and_then(|s| self.typed.bound.get(&(s.start, s.end)))
            .cloned()
        else {
            // A bare function stage, as an arm list piped into, takes
            // the piped value as its one argument.
            let ExprKind::Pipe {
                op: PipeOp::Plain,
                left,
                right,
                ..
            } = self.module.expressions[id.0].kind
            else {
                return declared;
            };
            if right != callee || function.parameters.len() != 1 {
                return declared;
            }
            if let Some(limit) = &function.parameters[0] {
                self.argument_entry(left, limit, None);
            }
            return function.result.clone().meet(&declared);
        };
        for (spot, _) in &bound.supplies {
            let (Some(argument), Some(Some(limit))) = (
                self.argument(id, &bound, *spot),
                function.parameters.get(*spot),
            ) else {
                continue;
            };
            let own = bound.spots.get(*spot).map(|(_, ty)| ty);
            self.argument_entry(argument, limit, own);
        }
        if !bound.open.is_empty() {
            if !bound.applied.is_empty() {
                return declared;
            }
            return Known {
                function: Some(alloc::boxed::Box::new(known::Function {
                    result: function.result.clone(),
                    ..known::Function::of(
                        bound
                            .open
                            .iter()
                            .filter_map(|i| {
                                function.parameters.get(*i).cloned()
                            })
                            .collect(),
                    )
                })),
                ..Known::default()
            };
        }
        let mut result = function.result.clone();
        if let Some(feeds) = &function.feeds {
            result = result.meet(&self.fed(id, &bound, feeds));
        }
        for (ty, argument) in &bound.applied {
            let Some(function) = result.function.take() else {
                result = Known::default();
                break;
            };
            if let Some(Some(limit)) = function.parameters.first() {
                self.argument_entry(*argument, limit, Some(ty));
            }
            result = function.after(1);
        }
        result.meet(&declared)
    }
    /// An argument entering a callee's known parameter limit (07-checks
    /// §What is known 8), unless its spot's own type carries it, whose
    /// entry inference recorded.
    fn argument_entry(
        &mut self,
        argument: ExprId,
        limit: &Type,
        own: Option<&Type>,
    ) {
        if own == Some(limit) {
            return;
        }
        let known = self.typed.known[argument.0].clone();
        let span = self.module.expressions[argument.0].span;
        let place_kind =
            core::mem::replace(&mut self.place_kind, Place::Parameter(None));
        let entry_phase = core::mem::replace(
            &mut self.entry_phase,
            (known.demanded || known.is_exact()).then_some(Phase::Compile),
        );
        let value_ty = core::mem::replace(
            &mut self.value_ty,
            self.typed.expr(argument).cloned(),
        );
        let start = self.typed.checks.len();
        self.enter_value(limit, &known, span, self.frequency(span), None);
        for check in &mut self.typed.checks[start..] {
            check.value = Some(argument);
        }
        self.place_kind = place_kind;
        self.entry_phase = entry_phase;
        self.value_ty = value_ty;
    }
    /// What comes out at a callee's result variable (07-checks §What is
    /// known 6): the join of each argument of its type and each declared
    /// result of a function argument returning it. With nothing going in,
    /// or an argument not known one by one, it is unknown.
    fn fed(
        &self,
        id: ExprId,
        bound: &crate::typed::Bound,
        feeds: &[known::Feed],
    ) -> Known {
        let argument = |spot: usize| self.argument(id, bound, spot);
        let mut joined: Option<Known> = None;
        for feed in feeds {
            let known = match *feed {
                known::Feed::Value(spot) => {
                    argument(spot).map(|a| self.typed.known[a.0].branch())
                }
                known::Feed::Returns(spot, arity) => argument(spot)
                    .and_then(|a| self.typed.expr(a))
                    .and_then(|ty| {
                        let ty = peel(ty, arity)?;
                        // A function argument with no declared limit
                        // may return anything.
                        has_limits(ty).then(|| {
                            let mut known = Known::default();
                            self.place_facts(ty, &mut known);
                            known
                        })
                    }),
                known::Feed::Element(spot) => {
                    argument(spot).and_then(|a| self.contents(a))
                }
            };
            // An unknown feed is a value not known, which a join keeps
            // nothing of but a function's parameter limits.
            let known = known.unwrap_or_default();
            joined = Some(match joined {
                Some(joined) => joined.join(&known),
                None => known,
            });
        }
        joined.unwrap_or_default()
    }
    /// What is known of every element of a sequence argument, joined;
    /// dead when it has none. `None` when it is no sequence or its
    /// elements are not known.
    fn contents(&self, argument: ExprId) -> Option<Known> {
        let element = match self.typed.expr(argument).map(crate::infer::meaning)
        {
            Some(Type::Array { element, .. }) => &**element,
            Some(Type::Named { name, arguments })
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Vec" | "Deque")
                    && arguments.len() == 1 =>
            {
                &arguments[0]
            }
            _ => return None,
        };
        let known = &self.typed.known[argument.0];
        let parts: Vec<_> = known
            .parts
            .iter()
            .filter(|(name, _)| name != "len")
            .map(|(_, part)| part.branch())
            .collect();
        if let Some(joined) = parts.iter().cloned().reduce(|a, b| a.join(&b)) {
            return Some(joined);
        }
        if known.parts.iter().any(|(name, k)| {
            name == "len" && k.range == Some(Range::exact(0.into()))
        }) {
            return Some(Known {
                failure: Some(known::Failure::Never),
                ..Known::default()
            });
        }
        // Elements known alike, or by the element type's limits.
        let mut each = known
            .element
            .as_deref()
            .map_or_else(Known::default, Known::branch);
        self.place_facts(element, &mut each);
        Some(each)
    }
    /// The argument a call supplies at a spot, when it is one expression.
    fn argument(
        &self,
        id: ExprId,
        bound: &crate::typed::Bound,
        spot: usize,
    ) -> Option<ExprId> {
        let span = match bound.supplies.iter().find(|(i, _)| *i == spot)? {
            (_, crate::typed::Supply::Expression(id)) => return Some(*id),
            (
                _,
                crate::typed::Supply::Pipe(span)
                | crate::typed::Supply::Head(span),
            ) => *span,
            _ => return None,
        };
        first_input(self.module, id)
            .filter(|input| self.module.expressions[input.0].span == span)
    }
    /// What a byte is known to be: a `U8`, with its chain of limits.
    fn byte_facts(&self) -> Known {
        let mut known = Known::default();
        self.named_facts(
            &TypeName {
                origin: Origin::Prelude,
                name: String::from("U8"),
            },
            &mut known,
            &mut Vec::new(),
        );
        known
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
        if let Literal::Affixed(affixed) = literal {
            // A bytes literal is exact: its length, and every byte a `U8`.
            if let Some(bytes) = crate::resolve::bytes(affixed) {
                known.parts.push((
                    String::from("len"),
                    exact(Integer::from(bytes.len() as u64)),
                ));
                known.element = Some(alloc::boxed::Box::new(self.byte_facts()));
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
            // A literal divisor is exact whatever meaning the instance
            // gives it (07-checks §What is known 1): no entry for `n / 8`.
            let literal = match &self.module.expressions[right.0].kind {
                ExprKind::Literal(literal @ Literal::Int { .. })
                    if when_integer =>
                {
                    literal_fact(literal).range
                }
                _ => None,
            };
            let range = literal
                .or_else(|| b.range.clone())
                .unwrap_or_else(Range::unknown);
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
            // Across meanings the conversion is a cast: what it gives is
            // known by the target's limits alone (§What is known 10).
            if source.is_none_or(|source| {
                crate::infer::meaning(source) != crate::infer::meaning(target)
            }) {
                known = Known::default();
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
        // A written constructor is a call, known by its declared result
        // (07-checks §What is known 5).
        if written
            && !matches!(self.resolved.expr(callee), Some(Target::SelfType))
        {
            return self.declared(id);
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
                .and_then(|values| {
                    values.iter().find(|(n, supply)| {
                        *n == name
                            && !matches!(
                                supply,
                                crate::infer::Supply::Declared(..)
                            )
                    })
                })
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
                self.bindings_of(module)
                    .name(name)
                    .map(|id| test(module, id, &Known::default()))
            })
            .unwrap_or_default()
    }
    /// A prelude or host function, known by its declared scheme alone
    /// (07-checks §What is known 5, 8): its parameters' and result's
    /// limits, and where each variable parameter of a function argument
    /// is fed.
    fn declared_function(
        &self,
        origin: &Origin,
        name: &str,
    ) -> Option<known::Function> {
        let namespace = match origin {
            Origin::Prelude => Some(&self.declarations.prelude),
            Origin::Commands => self.declarations.commands.as_ref(),
            Origin::Host(module) => self.declarations.get_module(module),
            _ => None,
        }?;
        let declared = namespace.values.iter().find(|v| v.name == name)?;
        let crate::decl::Kind::Function { spots, result, .. } = &declared.kind
        else {
            return None;
        };
        let module = &namespace.module;
        let types: Vec<_> = spots
            .iter()
            .map(|spot| self.syntax_type(module, origin, spot.ty))
            .collect();
        let parameters = types
            .iter()
            .map(|ty| has_limits(ty).then(|| ty.clone()))
            .collect();
        let mut result_known = Known::default();
        self.place_facts(
            &self.syntax_type(module, origin, *result),
            &mut result_known,
        );
        let first = spots.first().map(|spot| spot.ty);
        let contents = first
            .map(|first| self.contents_limits(module, origin, first))
            .unwrap_or_default();
        Some(known::Function {
            parameters,
            result: result_known,
            feeds: None,
            takes: declared_takes(&types, &contents),
        })
    }
    /// The pairs `(a, c)` of a declared scheme's limits saying that the
    /// argument of type `c` holds values of type `a`: `Items<a> c`, or a
    /// member limit on `c` taking a function of `a`, as `c.map`.
    fn contents_limits(
        &self,
        module: &Module,
        origin: &Origin,
        first: TypeId,
    ) -> Vec<(String, String)> {
        let Some(constraints) = module.types.iter().find_map(|node| {
            let TypeKind::Qualified { constraints, ty } = &node.kind else {
                return None;
            };
            let TypeKind::Function { parameter, .. } = &module.types[ty.0].kind
            else {
                return None;
            };
            let inner = match &module.types[parameter.0].kind {
                TypeKind::Labeled { ty, .. } => *ty,
                _ => *parameter,
            };
            (inner == first).then_some(constraints)
        }) else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        for constraint in constraints {
            match &constraint.kind {
                crate::ast::ConstraintKind::Named { class, variable } => {
                    if let Type::Named { name, arguments } =
                        self.syntax_type(module, origin, *class)
                    {
                        if let (true, [a]) = (
                            name.origin == Origin::Prelude
                                && name.name == "Items",
                            arguments.as_slice(),
                        ) {
                            let Some(a) = self::variable(a) else { continue };
                            pairs.push((
                                String::from(a),
                                variable
                                    .segments
                                    .last()
                                    .cloned()
                                    .unwrap_or_default(),
                            ));
                        }
                    }
                }
                crate::ast::ConstraintKind::Member { member, ty } => {
                    let Some(receiver) = member.segments.first() else {
                        continue;
                    };
                    let mut ty = self.syntax_type(module, origin, *ty);
                    while let Type::Function { parameter, result } = ty {
                        if let Type::Function { parameter, .. } = *parameter {
                            if let Some(a) = variable(&parameter) {
                                pairs.push((String::from(a), receiver.clone()));
                            }
                        }
                        ty = *result;
                    }
                }
            }
        }
        pairs
    }
    /// A local read at `read` meets each range it is narrowed to there.
    fn local(&self, site: Span, read: Option<Span>) -> Known {
        let site = (site.start, site.end);
        let mut known = self
            .locals
            .get(&site)
            .or_else(|| self.frame.and_then(|f| f.facts.get(&site)))
            .cloned()
            .unwrap_or_default();
        // ponytail: linear scan; index by site if a long file slows.
        for narrowed in read.iter().flat_map(|read| {
            self.narrowed.iter().filter(move |n| {
                n.site == site
                    && n.span.start <= read.start
                    && read.end <= n.span.end
            })
        }) {
            let path = if narrowed.len {
                vec![String::from("len")]
            } else {
                Vec::new()
            };
            meet_path(&mut known, path, &narrowed.range);
        }
        known
    }
    /// What is known of a target, read at `read`: a local meets each
    /// range it is narrowed to where the read is.
    fn target(&self, target: Option<&Target>, read: Option<Span>) -> Known {
        match target {
            Some(Target::Local { site }) => self.local(*site, read),
            Some(Target::Value { origin, name })
                if matches!(
                    origin,
                    Origin::Host(_) | Origin::Commands | Origin::Prelude
                ) =>
            {
                let mut known = if *origin == Origin::Prelude {
                    Known::default()
                } else {
                    self.host_fact(origin, name)
                };
                if let Some(function) = self.declared_function(origin, name) {
                    known.function = Some(alloc::boxed::Box::new(function));
                }
                known
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
                let mut fact = self.target(Some(head), read);
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
        // A function place is known by its parameter and result limits
        // (07-checks §What is known 2, 8).
        if let Type::Function { .. } = ty {
            if known.function.is_none() && has_limits(ty) {
                let mut parameters = Vec::new();
                let mut ty = ty;
                while let Type::Function { parameter, result } = ty {
                    parameters.push(
                        has_limits(parameter).then(|| (**parameter).clone()),
                    );
                    ty = result;
                }
                let mut result = Known::default();
                self.place_facts(ty, &mut result);
                known.function =
                    Some(alloc::boxed::Box::new(known::Function {
                        result,
                        ..known::Function::of(parameters)
                    }));
            }
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
            let has_members = self.declarations_by_name().get(name).is_some_and(|i| matches!(&module.entries[*i], Entry::Item(item) if matches!(item.kind, ItemKind::Type { members: Some(_), .. })));
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

/// A `Vec<U8>` place, the bytes of a text.
fn bytes_place(ty: &Type) -> bool {
    let Type::Named { name, arguments } = crate::infer::meaning(ty) else {
        return false;
    };
    let [element] = &arguments[..] else {
        return false;
    };
    let element = match element {
        Type::Limited { name, .. } => &**name,
        other => other,
    };
    name.origin == Origin::Prelude
        && name.name == "Vec"
        && matches!(element, Type::Named { name, .. }
            if name.origin == Origin::Prelude && name.name == "U8")
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
            ..Known::default()
        };
    }
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
            ..Known::default()
        }
    } else {
        Known {
            boolean: a.compare(op, &b),
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
        // A call, as a test's helper, is unknown here: the body read
        // once at its declared parameters (rule 5) gives a Bool not
        // known but for a constant body, which this reading leaves open.
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

/// Each condition, guard, `&&` left side and piped name, with where it
/// narrows. Ids are post-order, so each precedes what it narrows.
fn conditions(module: &Module) -> BTreeMap<usize, Vec<Narrows>> {
    let mut conditions: BTreeMap<usize, Vec<Narrows>> = BTreeMap::new();
    let span = |id: ExprId| module.expressions[id.0].span;
    for (index, expression) in module.expressions.iter().enumerate() {
        let mut add = |id: ExprId, narrows| {
            conditions.entry(id.0).or_default().push(narrows);
        };
        match &expression.kind {
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                add(
                    *condition,
                    Narrows::Body {
                        span: span(*then_body),
                        holds: true,
                    },
                );
                if let Some(else_body) = else_body {
                    add(
                        *condition,
                        Narrows::Body {
                            span: span(*else_body),
                            holds: false,
                        },
                    );
                }
            }
            ExprKind::Pipe {
                op: PipeOp::And,
                left,
                right,
                ..
            } => add(
                *left,
                Narrows::Body {
                    span: span(*right),
                    holds: true,
                },
            ),
            ExprKind::Pipe {
                op: PipeOp::Plain,
                left,
                right,
                ..
            } if matches!(
                module.expressions[right.0].kind,
                ExprKind::Arms(_)
            ) =>
            {
                add(*left, Narrows::Piped { arms: *right });
            }
            ExprKind::Arms(arms) => {
                for (i, arm) in arms.iter().enumerate() {
                    let Some(guard) = arm.guard else { continue };
                    add(
                        guard,
                        Narrows::Body {
                            span: span(arm.body),
                            holds: true,
                        },
                    );
                    add(
                        guard,
                        Narrows::Later {
                            arms: ExprId(index),
                            index: i,
                        },
                    );
                }
            }
            _ => (),
        }
    }
    conditions
}

impl Pass<'_> {
    /// Record what `id`, read as a condition, narrows (07-checks §What
    /// is known 4).
    fn narrow(&mut self, id: ExprId, narrows: &Narrows) {
        let arms_of = |arms: ExprId| match &self.module.expressions[arms.0].kind
        {
            ExprKind::Arms(arms) => arms.as_slice(),
            _ => &[],
        };
        match narrows {
            Narrows::Body { span, holds } => {
                for (site, len, range) in self.condition(id, *holds) {
                    self.narrowed.push(Narrowed {
                        span: *span,
                        site,
                        len,
                        range,
                    });
                }
            }
            Narrows::Later { arms, index } => {
                self.later_arms(id, arms_of(*arms), *index);
            }
            Narrows::Piped { arms } => self.piped_arms(id, arms_of(*arms)),
        }
    }

    /// A guard's negation narrows the later arms at the same parameter
    /// position. A later arm is reached past the guard's failure only
    /// when the guarded arm's patterns cannot fail.
    fn later_arms(
        &mut self,
        guard: ExprId,
        arms: &[crate::ast::Arm],
        index: usize,
    ) {
        let module = self.module;
        let name = |pattern: &crate::ast::PatternId| match &module.patterns
            [pattern.0]
            .kind
        {
            PatternKind::Name(name) => Some((name.span.start, name.span.end)),
            _ => None,
        };
        let arm = &arms[index];
        if !arm.patterns.iter().all(|p| {
            matches!(
                module.patterns[p.0].kind,
                PatternKind::Name(_) | PatternKind::Wildcard
            )
        }) {
            return;
        }
        for (site, len, range) in self.condition(guard, false) {
            let Some(position) =
                arm.patterns.iter().position(|p| name(p) == Some(site))
            else {
                continue;
            };
            for later in &arms[index + 1..] {
                if let Some(site) = later.patterns.get(position).and_then(name)
                {
                    self.narrowed.push(Narrowed {
                        span: later.span,
                        site,
                        len,
                        range: range.clone(),
                    });
                }
            }
        }
    }

    /// A literal pattern narrows the piped bare name to itself in its
    /// arm, and by `!=` the later arms past an unguarded one.
    fn piped_arms(&mut self, piped: ExprId, arms: &[crate::ast::Arm]) {
        let Some((site, false)) = self.narrowable(piped) else {
            return;
        };
        let read = self.typed.known[piped.0]
            .range
            .clone()
            .unwrap_or_else(Range::unknown);
        let mut own = read.clone();
        for arm in arms {
            let mut push = |range: Range| {
                self.narrowed.push(Narrowed {
                    span: arm.span,
                    site,
                    len: false,
                    range,
                });
            };
            if own != read {
                push(own.clone());
            }
            let [pattern] = arm.patterns[..] else {
                continue;
            };
            let PatternKind::Literal(literal @ Literal::Int { .. }) =
                &self.module.patterns[pattern.0].kind
            else {
                continue;
            };
            let Some(value) = literal_fact(literal).range else {
                continue;
            };
            push(value.clone());
            if arm.guard.is_none() {
                if let Some(range) =
                    Range::narrowed(BinaryOp::NotEqual, &value, Some(&own))
                {
                    own.intersect(&range);
                }
            }
        }
    }

    /// What holds of each narrowable name when `id` evaluates to
    /// `holds`: one range per local, or per local's `len`.
    fn condition(&self, id: ExprId, holds: bool) -> Vec<(Site, bool, Range)> {
        match &self.module.expressions[id.0].kind {
            ExprKind::Group(lines) => lines
                .last()
                .map_or_else(Vec::new, |id| self.condition(*id, holds)),
            ExprKind::Not { value, .. } => self.condition(*value, !holds),
            ExprKind::Pipe {
                op: PipeOp::And,
                left,
                right,
                ..
            } if holds => {
                let mut both = self.condition(*left, true);
                both.extend(self.condition(*right, true));
                both
            }
            ExprKind::Pipe {
                op: PipeOp::Or,
                left,
                right,
                ..
            } if !holds => {
                let mut both = self.condition(*left, false);
                both.extend(self.condition(*right, false));
                both
            }
            ExprKind::Binary {
                op, left, right, ..
            } => {
                use BinaryOp as B;
                let op = if holds {
                    *op
                } else {
                    match op {
                        B::Less => B::GreaterEqual,
                        B::LessEqual => B::Greater,
                        B::Greater => B::LessEqual,
                        B::GreaterEqual => B::Less,
                        B::Equal => B::NotEqual,
                        B::NotEqual => B::Equal,
                        _ => return Vec::new(),
                    }
                };
                let flipped = match op {
                    B::Less => B::Greater,
                    B::LessEqual => B::GreaterEqual,
                    B::Greater => B::Less,
                    B::GreaterEqual => B::LessEqual,
                    other => other,
                };
                [(*left, *right, op), (*right, *left, flipped)]
                    .into_iter()
                    .filter_map(|(x, e, op)| {
                        let (site, len) = self.narrowable(x)?;
                        let range = Range::narrowed(
                            op,
                            self.typed.known[e.0].range.as_ref()?,
                            self.typed.known[x.0].range.as_ref(),
                        )?;
                        Some((site, len, range))
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The local an `Integer` read names, by itself or by its `len`.
    fn narrowable(&self, id: ExprId) -> Option<(Site, bool)> {
        if !integer_type(self.typed.expr(id)) {
            return None;
        }
        let local = |id: ExprId| match self.resolved.expr(id) {
            Some(Target::Local { site }) => Some((site.start, site.end)),
            _ => None,
        };
        let len = |field: &crate::ast::Name| {
            field.segments.last().is_some_and(|n| n == "len")
        };
        match (&self.module.expressions[id.0].kind, self.resolved.expr(id)) {
            (ExprKind::Name(_), Some(Target::Local { site })) => {
                Some(((site.start, site.end), false))
            }
            (ExprKind::Name(_), Some(Target::Members { head, members }))
                if members.len() == 1 && members[0] == "len" =>
            {
                match &**head {
                    Target::Local { site } => {
                        Some(((site.start, site.end), true))
                    }
                    _ => None,
                }
            }
            (ExprKind::Field { value, field }, _) if len(field) => {
                Some((local(*value)?, true))
            }
            _ => None,
        }
    }
}

/// The arm lists a signature declares: annotated bindings' values and
/// annotated `impl` members.
fn signed(module: &Module) -> BTreeSet<usize> {
    let mut signed: BTreeSet<_> = module
        .expressions
        .iter()
        .filter_map(|e| match e.kind {
            ExprKind::Bind {
                value,
                annotation: Some(_),
                ..
            } => Some(value.0),
            _ => None,
        })
        .collect();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Impl { members, .. } = &item.kind else {
            continue;
        };
        for member in members {
            if let crate::ast::ImplMember::Named {
                annotation: Some(_),
                value: Some(value),
                ..
            } = member
            {
                signed.insert(value.0);
            }
        }
    }
    signed
}

/// A function type's result past `arity` parameters.
/// The expression whose value a group or block gives, which enters a
/// place for it.
fn last_value(module: &Module, mut id: ExprId) -> ExprId {
    while let ExprKind::Group(lines) | ExprKind::Block(lines) =
        &module.expressions[id.0].kind
    {
        let Some(last) = lines.last() else { break };
        id = *last;
    }
    id
}

/// The parameter limits of a function value, in order across its
/// arrows: by what is known of it, else by its own type.
fn own_limits(known: &Known, ty: Option<&Type>) -> Vec<Option<Type>> {
    if let Some(function) = known.function.as_deref() {
        let mut limits = function.parameters.clone();
        limits.extend(own_limits(&function.result, None));
        return limits;
    }
    let mut limits = Vec::new();
    let mut ty = ty.map(crate::infer::meaning);
    while let Some(Type::Function { parameter, result }) = ty {
        limits.push(has_limits(parameter).then(|| (**parameter).clone()));
        ty = Some(crate::infer::meaning(result));
    }
    limits
}

fn peel(mut ty: &Type, arity: usize) -> Option<&Type> {
    for _ in 0..arity {
        let Type::Function { result, .. } = ty else {
            return None;
        };
        ty = result;
    }
    Some(ty)
}

/// Where the result variable of a scheme over `arity` parameters goes in
/// (07-checks §What is known 6): `None` when the result is no variable
/// of the scheme, or something may produce it unseen: a literal of its
/// type, a limit whose member may return it, a function argument
/// returning it inside another type, or an argument holding it in a
/// part, whose facts are not read one by one.
fn feeds(
    scheme: &crate::types::Scheme,
    arity: usize,
) -> Option<Vec<known::Feed>> {
    let mut ty = &scheme.ty;
    let mut parameters = Vec::new();
    for _ in 0..arity {
        let Type::Function { parameter, result } = ty else {
            return None;
        };
        parameters.push(&**parameter);
        ty = result;
    }
    let Type::Var(var) = ty else {
        return None;
    };
    feeds_at(scheme, &parameters, *var)
}

/// Where a variable of a scheme goes in, over its first parameters.
fn feeds_at(
    scheme: &crate::types::Scheme,
    parameters: &[&Type],
    var: crate::types::Var,
) -> Option<Vec<known::Feed>> {
    let own = Type::Var(var);
    if !scheme.variables.contains(&var)
        || scheme.literals.iter().any(|(ty, ..)| ty.contains(var))
        || scheme.obligations.iter().any(|o| produces(o, var))
    {
        return None;
    }
    let mut feeds = Vec::new();
    for (spot, parameter) in parameters.iter().enumerate() {
        if **parameter == own {
            feeds.push(known::Feed::Value(spot));
            continue;
        }
        if !parameter.contains(var) {
            continue;
        }
        // A function argument: the variable only as one of its
        // parameters, which it takes, or as its result, which it gives.
        let mut arity = 0;
        let mut result = *parameter;
        while let Type::Function {
            parameter,
            result: rest,
        } = result
        {
            if parameter.contains(var) && **parameter != own {
                return None;
            }
            arity += 1;
            result = rest;
        }
        if arity == 0 || (result.contains(var) && *result != own) {
            return None;
        }
        if *result == own {
            feeds.push(known::Feed::Returns(spot, arity));
        }
    }
    Some(feeds)
}

/// Each parameter of a function parameter, among a scheme's first
/// `arity`, that is a variable whose feeds are known (07-checks §What is
/// known 8): what is known at it is what a function value passed there
/// meets.
fn takes(scheme: &crate::types::Scheme, arity: usize) -> Vec<known::Take> {
    let mut parameters = Vec::new();
    let mut ty = &scheme.ty;
    for _ in 0..arity {
        let Type::Function { parameter, result } = ty else {
            break;
        };
        parameters.push(&**parameter);
        ty = result;
    }
    let mut takes = Vec::new();
    for (spot, mut argument) in parameters.iter().copied().enumerate() {
        let mut index = 0;
        while let Type::Function { parameter, result } = argument {
            if let Type::Var(var) = **parameter {
                if let Some(feeds) = feeds_at(scheme, &parameters, var) {
                    takes.push(known::Take {
                        spot,
                        parameter: index,
                        feeds,
                    });
                }
            }
            index += 1;
            argument = result;
        }
    }
    takes
}

/// The takes of a declared scheme's spots (07-checks §What is known 8),
/// its variables written names: a function spot's parameter at a
/// variable `v` is fed by each spot of type `v`, each sequence of `v`,
/// each argument `contents` says holds `v`, and each function spot's
/// declared result when it gives `v`. It is unknown when a spot holds
/// `v` otherwise.
fn declared_takes(
    spots: &[Type],
    contents: &[(String, String)],
) -> Vec<known::Take> {
    let mut takes = Vec::new();
    for (spot, mut argument) in spots.iter().enumerate() {
        let mut index = 0;
        while let Type::Function { parameter, result } = argument {
            if let Some(v) = variable(parameter) {
                if let Some(feeds) = declared_feeds(spots, v, contents) {
                    takes.push(known::Take {
                        spot,
                        parameter: index,
                        feeds,
                    });
                }
            }
            index += 1;
            argument = result;
        }
    }
    takes
}

fn declared_feeds(
    spots: &[Type],
    v: &str,
    contents: &[(String, String)],
) -> Option<Vec<known::Feed>> {
    let mut feeds = Vec::new();
    for (spot, ty) in spots.iter().enumerate() {
        if let Type::Function { .. } = ty {
            // A function spot takes `v` only as a whole parameter and
            // gives it only as its whole result.
            let (mut tail, mut arity) = (ty, 0);
            while let Type::Function { parameter, result } = tail {
                if mentions(parameter, v) && variable(parameter) != Some(v) {
                    return None;
                }
                arity += 1;
                tail = result;
            }
            if variable(tail) == Some(v) {
                feeds.push(known::Feed::Returns(spot, arity));
            } else if mentions(tail, v) {
                return None;
            }
            continue;
        }
        let element = match crate::infer::meaning(ty) {
            Type::Array { element, .. } => Some(&**element),
            Type::Named { name, arguments }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Vec" | "Deque") =>
            {
                arguments.first()
            }
            _ => None,
        };
        match variable(ty) {
            Some(n) if n == v => feeds.push(known::Feed::Value(spot)),
            Some(c) if contents.iter().any(|(a, held)| a == v && held == c) => {
                feeds.push(known::Feed::Element(spot));
            }
            _ if element.and_then(variable) == Some(v) => {
                feeds.push(known::Feed::Element(spot));
            }
            _ if mentions(ty, v) => return None,
            _ => (),
        }
    }
    Some(feeds)
}

/// A declared scheme's type variable by its written name: rigid, or a
/// lowercase name the declaration module does not declare.
fn variable(ty: &Type) -> Option<&str> {
    match ty {
        Type::Rigid(n) => Some(n),
        Type::Named { name, arguments }
            if arguments.is_empty()
                && name.name.starts_with(|c: char| c.is_ascii_lowercase()) =>
        {
            Some(&name.name)
        }
        _ => None,
    }
}

/// Whether a declared type mentions the variable written `v`.
fn mentions(ty: &Type, v: &str) -> bool {
    if let Some(n) = variable(ty) {
        return n == v;
    }
    match ty {
        Type::Function { parameter, result } => {
            mentions(parameter, v) || mentions(result, v)
        }
        Type::Named { arguments, .. } => {
            arguments.iter().any(|a| mentions(a, v))
        }
        Type::Tuple(parts) | Type::Both(parts) | Type::Either(parts) => {
            parts.iter().any(|p| mentions(p, v))
        }
        Type::Array { element, .. } => mentions(element, v),
        Type::Limited { name, meaning } => {
            mentions(name, v) || mentions(meaning, v)
        }
        Type::Not(inner) => mentions(inner, v),
        _ => false,
    }
}

/// Whether a limit of a scheme may produce a value of `var`'s type: a
/// member whose result holds it, or a claim other than one whose
/// members return a fixed type.
fn produces(
    obligation: &crate::types::Obligation,
    var: crate::types::Var,
) -> bool {
    match obligation {
        crate::types::Obligation::Member { ty, .. } => {
            let mut ty = ty;
            while let Type::Function { result, .. } = ty {
                ty = result;
            }
            ty.contains(var)
        }
        crate::types::Obligation::Claim { head, class, .. } => {
            (head.contains(var) || class.contains(var))
                && !matches!(class, Type::Named { name, .. }
                    if name.origin == Origin::Prelude
                        && matches!(name.name.as_str(), "Eq" | "Ord" | "Show" | "Len"))
        }
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
    let part = path_part(known, path);
    if part
        .meaning
        .as_ref()
        .is_some_and(|n| n.origin == Origin::Prelude && n.name == "Integer")
        || part.range.is_some()
    {
        part.range
            .get_or_insert_with(Range::unknown)
            .intersect(range);
    }
}

/// Meet the part of `known` at `path` with `range`.
fn meet_path(known: &mut Known, path: Vec<String>, range: &Range) {
    path_part(known, path)
        .range
        .get_or_insert_with(Range::unknown)
        .intersect(range);
}

/// The part of `known` at `path`, made when it is missing.
fn path_part(known: &mut Known, path: Vec<String>) -> &mut Known {
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
    part
}

impl Pass<'_> {
    fn declarations_by_name(&self) -> &BTreeMap<TypeName, usize> {
        self.declarations_by_name.get_or_init(|| {
            declaration_index(
                self.module,
                self.declarations,
                self.files,
                self.frame,
            )
        })
    }

    /// `module`'s bindings: an accepted line's are kept by the frame,
    /// any other module's are found once per pass.
    pub(super) fn bindings_of(&self, module: &Module) -> Rc<Bindings> {
        let key = core::ptr::from_ref(module) as usize;
        if let Some(bindings) = self.bindings.borrow().get(&key) {
            return Rc::clone(bindings);
        }
        let bindings =
            self.frame
                .and_then(|frame| {
                    frame.lines.iter().find(|line| {
                        core::ptr::eq(&raw const line.module, module)
                    })
                })
                .map_or_else(
                    || Rc::new(Bindings::of(module)),
                    |line| Rc::clone(&line.bindings),
                );
        self.bindings.borrow_mut().insert(key, Rc::clone(&bindings));
        bindings
    }
}
