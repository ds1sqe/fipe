//! Bindings and their schemes ([types.md] §Core judgments, BIND and
//! Generalization): a binding generalizes over the variables not free
//! in the environment and carries their obligations; a variable held
//! only by literals, or bound as a non-function value, defaults first; an obligation whose variable left
//! the type is ambiguous at its member; an annotation
//! quantifies its written names; a `pub` binding keeps its scheme. A member
//! obligation's type stays with its head.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md

use super::{
    is_prelude, prelude, Error, Expected, Infer, Lit, Local, Owed, Reason,
    Status,
};
use crate::{
    ast::{ExprId, ExprKind, PatternKind},
    resolve::{Origin, Target},
    token::Span,
    types::{Obligation, Scheme, Subst, Type, Var},
};
use alloc::{boxed::Box, string::String, vec::Vec};

/// A refutable binding in a body, `[:x] = e`: the rest of the body is
/// `e |: (x => rest)` (evaluation.md §Pattern binding).
#[derive(Clone)]
pub(super) struct Lift {
    pub value: Type,
    pub span: Span,
}

/// Where a binding's typing began: its obligations and errors since.
#[derive(Clone, Copy)]
pub(super) struct Mark {
    obligations: usize,
    errors: usize,
}

/// One binding at a generalization boundary.
pub(super) struct Part {
    pub ty: Type,
    pub site: Span,
    pub annotated: bool,
}

impl Infer<'_> {
    /// `pattern = value` (BIND). An arm binding's own name is in scope
    /// in its arms, monomorphic there and generalized after.
    pub(super) fn bind(&mut self, id: ExprId) -> Option<Lift> {
        let module = self.module;
        let ExprKind::Bind {
            pattern,
            annotation,
            value,
            ..
        } = &module.expressions[id.0].kind
        else {
            return None;
        };
        let mark = self.mark();
        let env = self.state.env.len();
        let mut expected = annotation.map(|annotation| {
            let span = module.types[annotation.0].span;
            let expected = Expected::new(self.lower_here(annotation), span)
                .at(crate::typed::Place::Result);
            if matches!(module.expressions[value.0].kind, ExprKind::Arms(_)) {
                expected
            } else {
                expected.at(crate::typed::Place::Binding(
                    match &module.patterns[pattern.0].kind {
                        PatternKind::Name(name) => super::last(name),
                        _ => String::from("pattern"),
                    },
                ))
            }
        });
        if let (PatternKind::Name(name), ExprKind::Arms(arms)) = (
            &module.patterns[pattern.0].kind,
            &module.expressions[value.0].kind,
        ) {
            if !name.is_uppercase() {
                let ty = expected.as_ref().map_or_else(
                    || self.skeleton(arms),
                    |expected| expected.ty.clone(),
                );
                let local = Local {
                    scheme: Scheme::mono(ty.clone()),
                    heads: Some(self.heads(arms)),
                };
                self.state
                    .locals
                    .insert((name.span.start, name.span.end), local);
                // Monomorphic in its arms: a binding inside them must not
                // generalize over it, or recursion would be polymorphic.
                self.state.env.push(ty.clone());
                if expected.is_none() {
                    expected = Some(Expected::new(ty, name.span));
                }
            }
        }
        let ty = self.expr(*value, expected.as_ref());
        self.state.env.truncate(env);
        let pattern_node = &module.patterns[pattern.0];
        match &pattern_node.kind {
            PatternKind::Name(name) if !name.is_uppercase() => {
                let part = Part {
                    ty: expected
                        .as_ref()
                        .map_or_else(|| ty.clone(), |e| e.ty.clone()),
                    site: name.span,
                    annotated: annotation.is_some(),
                };
                let scheme = self.generalize(&[part], mark, &[]).remove(0);
                let heads = match &module.expressions[value.0].kind {
                    ExprKind::Arms(arms) => Some(self.heads(arms)),
                    _ => None,
                };
                let local = Local {
                    scheme: scheme.clone(),
                    heads,
                };
                let key = (name.span.start, name.span.end);
                self.state.locals.insert(key, local);
                self.state.bindings[id.0] = Some(scheme);
            }
            _ => {
                self.pattern(*pattern, &ty, self.span(*value));
                self.dots();
                if !matches!(self.apply(&ty), Type::Function { .. }) {
                    let env = self.environment(&[]);
                    self.default_literals(mark.obligations, &env, &[]);
                }
                self.state.env.push(ty.clone());
            }
        }
        let good = self.good_side(*pattern);
        good.then_some(Lift {
            value: ty,
            span: pattern_node.span,
        })
    }

    fn good_side(&self, pattern: crate::ast::PatternId) -> bool {
        let module = self.module;
        let mut side = pattern;
        while let PatternKind::Group(inner) = module.patterns[side.0].kind {
            side = inner;
        }
        match &module.patterns[side.0].kind {
            PatternKind::Outcome { good: true, .. } => true,
            PatternKind::Constructor { .. } => matches!(
                self.resolved.pattern(side),
                Some(Target::Constructor { origin: Origin::Prelude, name, .. })
                    if name == "Just" || name == "Ok"
            ),
            _ => false,
        }
    }

    pub(super) const fn mark(&self) -> Mark {
        Mark {
            obligations: self.state.obligations.len(),
            errors: self.state.errors.len(),
        }
    }

    /// An open literal meaning cannot be an outcome or function.
    pub(super) fn numeric_var(&self, ty: &Type) -> bool {
        let ty = self.apply(ty);
        matches!(ty, Type::Var(_))
            && self.state.obligations.iter().any(|owed| {
                matches!(&owed.obligation, Obligation::Claim { class, head, .. }
                    if is_prelude(class, "Literal") && self.apply(head) == ty)
            })
    }

    /// VAR: fresh variables, and the scheme's obligations owed at `span`.
    pub(super) fn instantiate_with_meanings(
        &mut self,
        scheme: &Scheme,
        span: Span,
    ) -> (Type, Vec<Type>) {
        let fresh: Vec<_> = scheme
            .variables
            .iter()
            .map(|v| (*v, self.fresh()))
            .collect();
        let ty = crate::types::replace(&scheme.ty, &fresh);
        let obligations = scheme
            .obligations
            .iter()
            .map(|o| o.map(|t| crate::types::replace(t, &fresh)));
        for (head, form, dot) in &scheme.literals {
            let head = crate::types::replace(head, &fresh);
            let mut owed = Owed::new(
                Obligation::Claim {
                    head,
                    class: prelude("Literal"),
                    dot: *dot,
                },
                span,
            );
            owed.literal = Some(form.clone());
            self.state.obligations.push(owed);
        }
        for obligation in obligations {
            if matches!(&obligation, Obligation::Claim { head, class, .. } if is_prelude(class, "Literal") && scheme.literals.iter().any(|(h, _, _)| crate::types::replace(h, &fresh) == *head))
            {
                continue;
            }
            self.state.obligations.push(Owed::new(obligation, span));
        }
        (ty, fresh.into_iter().map(|(_, ty)| ty).collect())
    }

    /// The free variables of the environment: every monomorphic type in
    /// scope, every use held until its group is typed, the session's
    /// weak variables, and every item typed without a scheme, except
    /// `group`.
    fn environment(&self, group: &[usize]) -> Vec<Var> {
        let mut vars = Vec::new();
        let mut seen = alloc::collections::BTreeSet::new();
        for ty in &self.state.env {
            super::gather(&self.apply(ty), &mut seen, &mut vars);
        }
        for used in &self.state.deferred {
            super::gather(&self.apply(&used.ty), &mut seen, &mut vars);
        }
        for var in 0..self.weak {
            let ty = self.apply(&Type::Var(Var(var)));
            super::gather(&ty, &mut seen, &mut vars);
        }
        for (index, item) in self.items.iter().enumerate() {
            let live = match self.state.status[index] {
                Status::Typing => !group.contains(&index),
                Status::Pending | Status::Done(_) => false,
            };
            if live {
                super::gather(&self.apply(&item.ty), &mut seen, &mut vars);
            }
        }
        self.close(&mut vars);
        vars
    }

    /// Add the type of every pending member obligation whose head is
    /// among `vars`: it is fixed once the head is.
    pub(super) fn close(&self, vars: &mut Vec<Var>) {
        loop {
            let before = vars.len();
            for owed in self
                .state
                .obligations
                .iter()
                .skip(self.state.dots_start)
                .filter(|o| !o.solved)
            {
                let head = owed.obligation.subject();
                let ty = match &owed.obligation {
                    Obligation::Member { ty, .. } => ty,
                    Obligation::Claim { class, .. } => class,
                };
                if let Type::Var(var) = self.apply(head) {
                    if vars.contains(&var) {
                        self.apply(ty).variables(vars);
                    }
                }
            }
            if vars.len() == before {
                return;
            }
        }
    }

    fn reachable(&self, ty: &Type) -> Vec<Var> {
        let mut vars = Vec::new();
        self.apply(ty).variables(&mut vars);
        self.close(&mut vars);
        vars
    }

    /// Generalize `parts`, typed together, over the obligations owed
    /// since `mark`. An error while typing them silences ambiguity.
    pub(super) fn generalize(
        &mut self,
        parts: &[Part],
        mark: Mark,
        group: &[usize],
    ) -> Vec<Scheme> {
        self.dots();
        let env = self.environment(group);
        let errors = mark.errors;
        let mark = mark.obligations;
        let reach: Vec<Vec<Var>> =
            parts.iter().map(|part| self.reachable(&part.ty)).collect();
        let stay: Vec<_> = parts
            .iter()
            .zip(&reach)
            .filter(|(part, _)| {
                matches!(self.apply(&part.ty), Type::Function { .. })
            })
            .flat_map(|(_, vars)| vars.iter().copied())
            .collect();
        self.default_literals(mark, &env, &stay);
        let quiet = self.state.errors.len() > errors;
        let types: Vec<Type> =
            parts.iter().map(|p| self.apply(&p.ty)).collect();
        let mut carried = alloc::vec![Vec::new(); parts.len()];
        let mut literals = alloc::vec![Vec::new(); parts.len()];
        let mut kept = Vec::new();
        let mut left: Vec<Obligation> = Vec::new();
        for owed in self.split_obligations(mark) {
            if owed.solved {
                if let Some(form) = &owed.literal {
                    let head = self.apply(owed.obligation.subject());
                    if let Type::Rigid(name) = &head {
                        for (index, part) in parts.iter().enumerate() {
                            let mut names = Vec::new();
                            rigids(&part.ty, &mut names);
                            if part.annotated && names.contains(name) {
                                literals[index].push((
                                    head.clone(),
                                    form.clone(),
                                    owed.span,
                                ));
                            }
                        }
                    }
                }
                if owed.assumed {
                    for (index, part) in parts.iter().enumerate() {
                        if part.annotated {
                            carried[index].push(owed.obligation.clone());
                        }
                    }
                }
                continue;
            }
            let obligation = owed.obligation.map(|ty| self.apply(ty));
            let &Type::Var(var) = obligation.subject() else {
                kept.push(owed);
                continue;
            };
            if env.contains(&var) {
                kept.push(owed);
                continue;
            }
            let mut placed = false;
            let mut keep = false;
            for (index, part) in parts.iter().enumerate() {
                if reach[index].contains(&var) {
                    placed = true;
                    if let Some(form) = &owed.literal {
                        literals[index].push((
                            obligation.subject().clone(),
                            form.clone(),
                            owed.span,
                        ));
                    }
                    let list: &mut Vec<Obligation> = &mut carried[index];
                    // Not written: an annotated type leaves it.
                    if part.annotated {
                        keep = true;
                    } else if !list.contains(&obligation) {
                        list.push(obligation.clone());
                    }
                }
            }
            if keep {
                kept.push(owed);
            } else if !placed && !quiet {
                left.push(obligation);
            }
        }
        self.state.obligations.extend(kept);
        let site = parts.first().map_or_else(Span::default, |part| part.site);
        self.left(left, site);
        parts
            .iter()
            .zip(types)
            .zip(carried)
            .zip(literals)
            .map(|(((part, ty), obligations), literals)| {
                let mut scheme =
                    self.scheme(part, ty, obligations, literals, &env);
                self.trim_claims(&mut scheme);
                scheme
            })
            .collect()
    }

    fn finish_lengths(&mut self, site: Span) {
        match self.state.subst.settle_lengths() {
            Ok(false) => (),
            Ok(true) => self.error(
                site,
                Reason::Ambiguous {
                    class: String::from("length"),
                },
            ),
            Err(_) => self.error(
                site,
                Reason::Limit {
                    message: String::from("length mismatch"),
                },
            ),
        }
    }

    /// An escaping obligation is ambiguous once per member name.
    fn left(&mut self, left: Vec<Obligation>, site: Span) {
        self.finish_lengths(site);
        let mut seen = alloc::collections::BTreeSet::new();
        for obligation in left {
            let (name, dot) = match obligation {
                Obligation::Member { name, dot, .. } => (name, dot),
                Obligation::Claim { class, dot, .. } => {
                    (alloc::format!("{class}"), dot)
                }
            };
            let error = Error {
                span: if dot == Span::default() { site } else { dot },
                reason: Reason::Ambiguous {
                    class: name.clone(),
                },
            };
            if seen.insert((error.span, name)) {
                self.state.errors.push(error);
            }
        }
    }

    fn scheme(
        &mut self,
        part: &Part,
        ty: Type,
        obligations: Vec<Obligation>,
        literals: Vec<(Type, Lit, Span)>,
        env: &[Var],
    ) -> Scheme {
        let mut variables = Vec::new();
        ty.variables(&mut variables);
        if !part.annotated {
            for obligation in &obligations {
                obligation.variables(&mut variables);
            }
            variables.retain(|var| !env.contains(var));
            self.state.body_parameters.insert(
                (part.site.start, part.site.end),
                variables.iter().map(|var| Type::Var(*var)).collect(),
            );
            return Scheme {
                variables,
                obligations,
                literals,
                ty,
            };
        }
        // The written names, each a quantified variable.
        let mut names = Vec::new();
        rigids(&ty, &mut names);
        let fresh: Vec<(String, Var)> = names
            .into_iter()
            .map(|name| (name, self.state.subst.fresh()))
            .collect();
        self.state.body_parameters.insert(
            (part.site.start, part.site.end),
            variables
                .iter()
                .filter(|var| !env.contains(var))
                .map(|var| Type::Var(*var))
                .chain(fresh.iter().map(|(name, _)| Type::Rigid(name.clone())))
                .collect(),
        );
        Scheme {
            variables: variables
                .into_iter()
                .filter(|v| !env.contains(v))
                .chain(fresh.iter().map(|(_, var)| *var))
                .collect(),
            literals: literals
                .iter()
                .map(|(ty, form, span)| {
                    (unrigid(ty, &fresh), form.clone(), *span)
                })
                .collect(),
            obligations: obligations
                .iter()
                .map(|o| o.map(|t| unrigid(t, &fresh)))
                .collect(),
            ty: unrigid(&ty, &fresh),
        }
    }

    /// A main line defaults literal meanings, then reports any member
    /// whose head remains unknown.
    pub(super) fn line_boundary(&mut self, span: Span, mark: Mark) {
        self.dots();
        let env = self.environment(&[]);
        self.default_literals(mark.obligations, &env, &[]);
        let quiet = self.state.errors.len() > mark.errors;
        let mut left = Vec::new();
        for owed in self.split_obligations(mark.obligations) {
            if owed.solved {
                continue;
            }
            match self.apply(owed.obligation.subject()) {
                Type::Var(var) if !env.contains(&var) => {
                    if !quiet {
                        left.push(owed.obligation);
                    }
                }
                _ => self.state.obligations.push(owed),
            }
        }
        self.left(left, span);
    }

    /// At the end, a member obligation whose head is in no line's type
    /// has left it: ambiguous at its dot.
    pub(super) fn left_members(&mut self) {
        let mut stay = Vec::new();
        for entry in &self.module.entries {
            let crate::ast::Entry::Line(id) = entry else {
                continue;
            };
            if let Some(ty) = &self.state.types[id.0] {
                self.apply(ty).variables(&mut stay);
            }
        }
        self.close(&mut stay);
        let mut left = Vec::new();
        for owed in &mut self.state.obligations {
            let head = owed.obligation.subject();
            let head = self.state.subst.apply(head);
            if !owed.solved
                && matches!(head, Type::Var(v) if !stay.contains(&v))
            {
                owed.solved = true;
                left.push(owed.obligation.clone());
            }
        }
        self.left(left, Span::default());
    }

    /// Literal forms choose one meaning together. A binding retains a
    /// literal with other member requirements in a function; other values
    /// default to the first meaning that meets their claims.
    pub(super) fn default_literals(
        &mut self,
        mark: usize,
        env: &[Var],
        stay: &[Var],
    ) {
        let groups = self.literal_groups(mark, env);
        for (var, indices) in groups {
            if !matches!(self.apply(&Type::Var(var)), Type::Var(_)) {
                continue;
            }
            let held: Vec<_> = indices
                .iter()
                .map(|i| &self.state.obligations[*i])
                .filter(|o| !o.solved)
                .collect();
            let literal = held.iter().any(|o| matches!(&o.obligation, Obligation::Claim { class, .. } if is_prelude(class, "Literal")));
            let only = held.iter().all(|o| matches!(&o.obligation, Obligation::Claim { class, .. } if is_prelude(class, "Literal")));
            if !literal || stay.contains(&var) {
                continue;
            }
            let forms: Vec<_> =
                held.iter().filter_map(|o| o.literal.as_ref()).collect();
            let mut kinds = Vec::new();
            for form in &forms {
                let kind = match form {
                    Lit::Int { .. } => "Whole",
                    Lit::Float { .. } => "Fraction",
                    Lit::Text => "Text",
                    Lit::Bytes => "Bytes",
                };
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
            if kinds.len() > 1
                && !kinds.iter().all(|k| matches!(*k, "Whole" | "Fraction"))
            {
                let start = held.iter().map(|o| o.span.start).min().unwrap();
                let end = held.iter().map(|o| o.span.end).max().unwrap();
                let span = self
                    .module
                    .expressions
                    .iter()
                    .map(|e| e.span)
                    .filter(|s| s.start <= start && s.end >= end)
                    .min_by_key(|s| s.end - s.start)
                    .unwrap_or_else(|| {
                        held.iter().max_by_key(|o| o.span.end).unwrap().span
                    });
                let forms = kinds.into_iter().map(String::from).collect();
                for index in mark..self.state.obligations.len() {
                    if self.apply(
                        self.state.obligations[index].obligation.subject(),
                    ) == Type::Var(var)
                    {
                        self.state.obligations[index].solved = true;
                    }
                }
                self.error(span, Reason::NoCommonMeaning { forms });
                continue;
            }
            let name = if forms.iter().any(|l| matches!(l, Lit::Float { .. })) {
                "F64"
            } else if forms.iter().any(|l| matches!(l, Lit::Int { .. })) {
                "Integer"
            } else if forms.iter().any(|l| matches!(l, Lit::Text)) {
                "String"
            } else if forms.iter().any(|l| matches!(l, Lit::Bytes)) {
                "Vec"
            } else {
                "Integer"
            };
            let ty = if name == "Vec" {
                super::prelude_of(
                    "Vec",
                    alloc::vec![self.limited(prelude("U8"))],
                )
            } else {
                prelude(name)
            };
            if name == "Vec" {
                self.state.subst.default_bytes(var, ty);
            } else if name == "Integer" && !only {
                let before = self.state.clone();
                self.hint(&Type::Var(var), &ty);
                self.dots();
                if self.state.errors.len() > before.errors.len() {
                    let errors = before.errors.len();
                    let integer = core::mem::replace(&mut self.state, before);
                    self.hint(&Type::Var(var), &prelude("F64"));
                    self.dots();
                    if self.state.errors.len() > errors {
                        self.state = integer;
                    }
                }
            } else {
                self.hint(&Type::Var(var), &ty);
            }
        }
        self.dots();
    }

    pub(super) fn settle(&mut self) -> Vec<Obligation> {
        self.dots();
        let mut unsolved = Vec::new();
        for owed in core::mem::take(&mut self.state.obligations) {
            if owed.solved {
                continue;
            }
            let obligation = owed.obligation.map(|ty| self.apply(ty));
            if !unsolved.contains(&obligation) {
                unsolved.push(obligation);
            }
        }
        unsolved
    }

    /// A literal whose type was left open, against the type it got.
    pub(super) fn check_literal(
        &mut self,
        literal: Lit,
        ty: &Type,
        span: Span,
        other: Span,
    ) {
        if let Type::Limited { meaning, .. } = ty {
            return self.check_literal(literal, meaning, span, other);
        }
        let float = is_prelude(ty, "F32") || is_prelude(ty, "F64");
        let actual = match literal {
            _ if matches!(ty, Type::Var(_)) => return,
            Lit::Int { ref magnitude, .. } if float => {
                let (precision, exponent) = if is_prelude(ty, "F32") {
                    (24, 128)
                } else {
                    (53, 1024)
                };
                if !magnitude.fits_binary_float(precision, exponent) {
                    self.error(span, Reason::OutOfRange);
                }
                return;
            }
            Lit::Float { .. } if float => return,
            Lit::Int { .. } if is_prelude(ty, "Integer") => return,
            // A fraction at an integer place is the integer it spells
            // exactly, or a check error (04-members §Conversions).
            Lit::Float { whole: Some(_) } if is_prelude(ty, "Integer") => {
                return;
            }
            Lit::Float { whole: None } if is_prelude(ty, "Integer") => {
                self.error(span, Reason::NotExact);
                return;
            }
            Lit::Int { .. } => prelude("Integer"),
            Lit::Float { .. } => prelude("F64"),
            Lit::Text if is_prelude(ty, "String") => return,
            Lit::Text => prelude("String"),
            Lit::Bytes if matches!(ty, Type::Named { name, arguments } if name.origin == Origin::Prelude && name.name == "Vec" && arguments.len() == 1 && is_prelude(super::meaning(&arguments[0]), "Integer")) => {
                return
            }
            Lit::Bytes => super::prelude_of(
                "Vec",
                alloc::vec![self.limited(prelude("U8"))],
            ),
        };
        let expected = ty.clone();
        self.error(
            span,
            Reason::Mismatch {
                expected,
                actual,
                other,
            },
        );
    }
}

/// A scheme with the substitution applied to its type and obligations.
pub(super) fn applied(scheme: &Scheme, subst: &Subst) -> Scheme {
    Scheme {
        variables: scheme.variables.clone(),
        obligations: scheme
            .obligations
            .iter()
            .map(|obligation| obligation.map(|ty| subst.apply(ty)))
            .collect(),
        literals: scheme
            .literals
            .iter()
            .map(|(ty, form, span)| (subst.apply(ty), form.clone(), *span))
            .collect(),
        ty: subst.apply(&scheme.ty),
    }
}

fn rigids(ty: &Type, out: &mut Vec<String>) {
    match ty {
        Type::Rigid(name) if !out.contains(name) => out.push(name.clone()),
        Type::Named { arguments, .. } => {
            for argument in arguments {
                rigids(argument, out);
            }
        }
        Type::Not(inner) => rigids(inner, out),
        Type::Sum(parts)
        | Type::Tuple(parts)
        | Type::Either(parts)
        | Type::Both(parts) => {
            for p in parts {
                rigids(p, out);
            }
        }
        Type::Limited { name, .. } => rigids(name, out),
        Type::Array { element, length } => {
            rigids(element, out);
            rigids(length, out);
        }
        Type::Function { parameter, result } => {
            rigids(parameter, out);
            rigids(result, out);
        }
        _ => {}
    }
}

fn unrigid(ty: &Type, fresh: &[(String, Var)]) -> Type {
    match ty {
        Type::Rigid(name) => fresh
            .iter()
            .find(|(n, _)| n == name)
            .map_or_else(|| ty.clone(), |(_, var)| Type::Var(*var)),
        Type::Named { name, arguments } => Type::Named {
            name: name.clone(),
            arguments: arguments.iter().map(|a| unrigid(a, fresh)).collect(),
        },
        Type::Sum(parts) => {
            Type::sum(parts.iter().map(|p| unrigid(p, fresh)).collect())
        }
        Type::Tuple(parts) => {
            Type::Tuple(parts.iter().map(|p| unrigid(p, fresh)).collect())
        }
        Type::Either(parts) => {
            Type::Either(parts.iter().map(|p| unrigid(p, fresh)).collect())
        }
        Type::Both(parts) => {
            Type::Both(parts.iter().map(|p| unrigid(p, fresh)).collect())
        }
        Type::Not(inner) => Type::Not(Box::new(unrigid(inner, fresh))),
        Type::Limited { name, meaning } => Type::Limited {
            name: Box::new(unrigid(name, fresh)),
            meaning: Box::new(unrigid(meaning, fresh)),
        },
        Type::Array { element, length } => Type::Array {
            element: Box::new(unrigid(element, fresh)),
            length: Box::new(unrigid(length, fresh)),
        },
        Type::Function { parameter, result } => {
            Type::function(unrigid(parameter, fresh), unrigid(result, fresh))
        }
        Type::Unit | Type::Number(_) | Type::Var(_) => ty.clone(),
    }
}

impl Infer<'_> {
    fn split_obligations(&mut self, mark: usize) -> Vec<Owed> {
        self.state.dots_start = self.state.dots_start.min(mark);
        self.state.obligations.split_off(mark)
    }
    fn literal_groups(
        &self,
        mark: usize,
        env: &[Var],
    ) -> alloc::collections::BTreeMap<Var, Vec<usize>> {
        let mut groups = alloc::collections::BTreeMap::<Var, Vec<usize>>::new();
        for (index, owed) in self
            .state
            .obligations
            .iter()
            .enumerate()
            .skip(mark.max(self.state.dots_start))
        {
            if !owed.solved {
                if let Type::Var(var) = self.apply(owed.obligation.subject()) {
                    if !env.contains(&var) {
                        groups.entry(var).or_default().push(index);
                    }
                }
            }
        }
        groups
    }
}

impl Infer<'_> {
    pub(super) fn default_entering(&mut self, actual: &Type) {
        let mut own = Vec::new();
        self.apply(actual).variables(&mut own);
        let mut env = self.environment(&[]);
        for owed in &self.state.obligations {
            let mut variables = Vec::new();
            self.apply(owed.obligation.subject())
                .variables(&mut variables);
            env.extend(variables.into_iter().filter(|v| !own.contains(v)));
        }
        self.default_literals(0, &env, &[]);
    }
}
