//! Static facts and entry checks over the typed tree; never unification.
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
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use known::{End, Integer, Known, Range};

type Site = (usize, usize);

pub(crate) fn check(
    module: &Module,
    resolved: &Resolved,
    declarations: &Declarations,
    (files, cache): (&[(String, Module)], &crate::infer::files::FileCache),
    frame: Option<&Frame>,
    entries: &[(ExprId, Type, Span, Place)],
    typed: &mut Typed,
) -> Vec<Error> {
    let mut pass = Pass {
        module,
        resolved,
        declarations,
        files,
        cache,
        frame,
        typed,
        locals: BTreeMap::new(),
        errors: Vec::new(),
        active: Vec::new(),
        place_kind: Place::Result,
        entry_phase: None,
        purity: core::cell::RefCell::new(Vec::new()),
    };
    pass.typed.known = vec![Known::default(); module.expressions.len()];
    for (site, local) in &pass.typed.frozen {
        let mut known = Known::default();
        pass.place_facts(&local.scheme.ty, &mut known);
        pass.locals.insert(*site, known);
    }
    pass.coverage();
    // Parameters are places: seed their written limits before reading bodies.
    for entry in &module.entries {
        if let Entry::Line(id) = entry {
            pass.seed(*id);
        }
    }
    for index in 0..module.expressions.len() {
        let id = ExprId(index);
        if pass.typed.expr(id).is_none() {
            continue;
        }
        let mut known = pass.value(id);
        known.pure = pass.pure_expression(id);
        known.phase = pass.expression_phase(id);
        known.demanded |= call_head(module, id)
            .and_then(|callee| crate::resolve::timing(resolved.expr(callee)))
            == Some(Phase::Compile);
        if let Some(ty) = pass.typed.expr(id) {
            known.holding = pass.holding(ty, &mut Vec::new()).or(known.holding);
        }
        if matches!(
            module.expressions[index].kind,
            ExprKind::Record { .. }
                | ExprKind::Call { .. }
                | ExprKind::Pipe {
                    op: PipeOp::Plain,
                    ..
                }
        ) {
            if let Some(Type::Named { name, .. }) = pass.typed.expr(id).cloned()
            {
                if matches!(
                    module.expressions[index].kind,
                    ExprKind::Record { .. }
                ) || matches!(
                    call_head(module, id)
                        .and_then(|callee| resolved.expr(callee)),
                    Some(Target::Constructor { .. })
                ) {
                    pass.named_entry(
                        &name,
                        &known,
                        module.expressions[index].span,
                        frequency(module, module.expressions[index].span),
                        &mut Vec::new(),
                    );
                    if pass.declaration(&name).is_some_and(|(_, _, limits)| {
                        limits.iter().any(|l| matches!(l, Limit::Test(_)))
                    }) {
                        pass.named_facts(&name, &mut known, &mut Vec::new());
                    }
                }
            }
        }
        pass.typed.known[index] = known.clone();
        pass.binding_facts(id);
    }
    pass.timing_places();
    pass.holdings();
    pass.demands();
    for check in &mut pass.typed.checks {
        if module
            .expressions
            .iter()
            .enumerate()
            .any(|(i, e)| e.span == check.place && pass.typed.known[i].demanded)
        {
            check.phase = Phase::Compile;
            check.reason = Reason::CompileTimeEvaluation;
        }
    }
    pass.entries(entries);
    pass.keep_facts();
    pass.typed.facts = pass
        .locals
        .into_iter()
        .filter(|(site, _)| site.0 >= module.span.start)
        .collect();
    pass.errors
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
    entry_phase: Option<Phase>,
    purity: core::cell::RefCell<Vec<(Target, bool)>>,
}
impl Pass<'_> {
    fn binding_facts(&mut self, id: ExprId) {
        if let ExprKind::Bind {
            pattern,
            annotation,
            value,
            ..
        } = self.module.expressions[id.0].kind
        {
            let mut fact = self.typed.known[value.0].clone();
            if annotation.is_some() {
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
            if let PatternKind::Name(name) =
                &self.module.patterns[pattern.0].kind
            {
                self.locals.insert((name.span.start, name.span.end), fact);
            }
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
        for (id, ty, place, place_kind) in entries {
            if !has_limits(ty) {
                continue;
            }
            let expr = &module.expressions[id.0];
            if matches!(
                expr.kind,
                ExprKind::Group(_)
                    | ExprKind::Block(_)
                    | ExprKind::List(_)
                    | ExprKind::With { .. }
            ) {
                continue;
            }
            if entries.iter().any(|(outer, outer_ty, outer_place, _)| {
                let parent = &module.expressions[outer.0];
                *outer != *id
                    && outer_place == place
                    && outer_ty == ty
                    && matches!(parent.kind, ExprKind::If { .. })
                    && parent.span.start <= expr.span.start
                    && expr.span.end <= parent.span.end
            }) {
                continue;
            }
            if matches!(expr.kind, ExprKind::Arms(_)) && module.expressions.iter().any(|e| matches!(e.kind, ExprKind::Bind { value, annotation: Some(_), .. } if value == *id)) { continue; }
            let fact = self.typed.known[id.0].clone();
            self.place_kind = place_kind.clone();
            self.entry_phase =
                (fact.demanded || fact.is_exact()).then_some(Phase::Compile);
            self.enter(ty, &fact, *place, frequency(module, expr.span));
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
        module.entries.iter().find_map(|entry| match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Type {
                    name: n,
                    base,
                    limits,
                    ..
                } if n.segments.last() == Some(&name.name) => {
                    Some((module, *base, limits.as_slice()))
                }
                _ => None,
            },
            Entry::Line(_) => None,
        })
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
        let local = module.entries.iter().any(|e| matches!(e, Entry::Item(item) if matches!(&item.kind, ItemKind::Type { name: n, .. } if n.segments.last() == Some(&name))));
        Some(TypeName {
            origin: if local {
                origin.clone()
            } else {
                Origin::Prelude
            },
            name,
        })
    }
    fn coverage(&mut self) {
        for expression in &self.module.expressions {
            let ExprKind::Arms(arms) = &expression.kind else {
                continue;
            };
            let mut gates = Vec::new();
            let mut wildcard = false;
            for arm in arms {
                if arm.guard.is_some() {
                    continue;
                }
                let [pattern] = arm.patterns.as_slice() else {
                    continue;
                };
                wildcard |= matches!(&self.module.patterns[pattern.0].kind, PatternKind::Name(crate::ast::Name { segments, .. }) if segments.first().is_some_and(|s| s.starts_with(char::is_lowercase)));
                if matches!(
                    self.module.patterns[pattern.0].kind,
                    PatternKind::Wildcard
                ) {
                    wildcard = true;
                }
                if let Some(Target::Type { origin, name }) =
                    self.resolved.pattern(*pattern)
                {
                    let gate = TypeName {
                        origin: origin.clone(),
                        name: name.clone(),
                    };
                    if self
                        .declaration(&gate)
                        .is_some_and(|(_, base, _)| base.is_some())
                    {
                        gates.push(gate);
                    }
                }
            }
            if gates.is_empty() || wildcard {
                continue;
            }
            let covered = gates.iter().any(|gate| {
                self.declaration(gate)
                    .and_then(|(module, base, _)| {
                        base.map(|id| {
                            self.syntax_type(module, &gate.origin, id)
                        })
                    })
                    .is_some_and(|base| negated_gate(&base, &gates))
            });
            if !covered {
                self.errors.push(Error {
                    span: expression.span,
                    reason: TypeReason::Limit {
                        message: String::from(
                            "gate arms need a complementary gate or a fallback",
                        ),
                    },
                });
            }
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
                    let mut known = Known::default();
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
            ExprKind::Field { value, field } => {
                let fact = get(*value);
                fact.parts
                    .iter()
                    .find(|(n, _)| Some(n) == field.segments.last())
                    .map_or_else(Known::default, |(_, k)| k.clone())
            }
            ExprKind::Group(lines) | ExprKind::Block(lines) => {
                lines.last().map_or_else(Known::default, |id| get(*id))
            }
            ExprKind::List(items) => {
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
            ExprKind::Record { fields, .. } => Known {
                parts: fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.segments.last().cloned().unwrap_or_default(),
                            get(f.value),
                        )
                    })
                    .collect(),
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
            Some(Target::Constructor { .. }) => self.construct_value(id),
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
        if !integer_type(self.typed.expr(id))
            && matches!(literal, Literal::Int { .. } | Literal::Float(_))
        {
            let text = match literal {
                Literal::Float(text) => text.clone(),
                Literal::Int {
                    negative,
                    magnitude,
                } => Integer::new(*negative, magnitude.clone()).to_string(),
                _ => String::new(),
            };
            if !self.typed.expr(id).is_some_and(|ty| matches!(ty, Type::Named { name, .. } if name.origin == Origin::Prelude && matches!(name.name.as_str(), "F32" | "F64"))) { return Known::default(); }
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
        if matches!(op, BinaryOp::Divide | BinaryOp::Remainder)
            && integer_type(self.typed.expr(*right))
        {
            let range = b.range.clone().unwrap_or_else(Range::unknown);
            if range == Range::exact(0.into()) {
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
                    place: self.module.expressions[right.0].span,
                    place_kind: Place::Parameter(Some(String::from("divisor"))),
                    limit: *operator,
                    phase: self.site_phase(expression.span),
                    reason: Reason::UnknownInput,
                    frequency: frequency(self.module, expression.span),
                    place_origin: Origin::This,
                    limit_origin: Origin::This,
                    name: String::from("nonzero divisor"),
                    test_count: 1,
                    known: b.clone(),
                    gate: false,
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
            let start = self.typed.checks.len();
            let errors = self.errors.len();
            self.enter(
                target,
                &known,
                expression.span,
                frequency(self.module, expression.span),
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
        let get = |id: ExprId| self.typed.known[id.0].clone();

        let Some(Target::Constructor { origin, ty, name }) =
            self.resolved.expr(callee)
        else {
            return Known::default();
        };
        let Some(module) = self.owner(origin) else {
            return Known::default();
        };
        let fields = construction_fields(module, ty, name);
        let supplied = self
            .typed
            .supplied
            .get(&(expression.span.start, expression.span.end));
        let parts = fields
            .iter()
            .map(|field| {
                let name =
                    field.name.segments.last().cloned().unwrap_or_default();
                let value = supplied
                    .and_then(|values| values.iter().find(|(n, _)| *n == name))
                    .map(|(_, supplied)| match supplied {
                        crate::infer::Supply::Expr(id) => get(*id),
                        crate::infer::Supply::True => Known {
                            boolean: Some(true),
                            ..Known::default()
                        },
                        crate::infer::Supply::Typed(_, span) => self
                            .module
                            .expressions
                            .iter()
                            .position(|e| e.span == *span)
                            .map_or_else(Known::default, |i| get(ExprId(i))),
                    })
                    .or_else(|| {
                        field
                            .default
                            .map(|id| test(module, id, &Known::default()))
                    })
                    .unwrap_or_default();
                (name, value)
            })
            .collect();
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
                frequency(self.module, expression.span),
                &mut Vec::new(),
            );
            for check in &mut self.typed.checks[start..] {
                check.gate = true;
            }
            self.named_facts(&name, &mut known, &mut Vec::new());
        }
        known
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
                self.owner(origin)
                    .and_then(|module| {
                        phase::binding(module, name)
                            .map(|id| test(module, id, &Known::default()))
                    })
                    .unwrap_or_default()
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
                let cache = self.cache.borrow();
                let Some(checked) = cache.get(stem) else {
                    return Known::default();
                };
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
                    fact = fact
                        .parts
                        .iter()
                        .find(|(n, _)| n == member)
                        .map_or_else(Known::default, |(_, k)| k.clone());
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
            let has_members = module.entries.iter().any(|e| matches!(e, Entry::Item(item) if matches!(&item.kind, ItemKind::Type { name: n, members: Some(_), .. } if n.segments.last() == Some(&name.name))));
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
        if !known.names.contains(name) {
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
fn binary(op: BinaryOp, a: &Known, b: &Known) -> Known {
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
        } => binary(
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
        ExprKind::Not { value: v, .. } => Known {
            boolean: test(module, *v, value).boolean.map(|b| !b),
            ..Known::default()
        },
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
        part.range
            .get_or_insert_with(Range::unknown)
            .intersect(&range);
    }
}

fn negated_gate(ty: &Type, gates: &[TypeName]) -> bool {
    match ty {
        Type::Not(inner) => {
            matches!(&**inner, Type::Named { name, .. } if gates.contains(name))
        }
        Type::Both(parts) => parts.iter().any(|part| negated_gate(part, gates)),
        _ => false,
    }
}

pub(crate) fn has_limits(ty: &Type) -> bool {
    match ty {
        Type::Limited { .. } => true,
        Type::Named { arguments, .. } => arguments.iter().any(has_limits),
        Type::Array { element, .. } => has_limits(element),
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
