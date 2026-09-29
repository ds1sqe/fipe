//! Bindings and their schemes ([types.md] §Core judgments, BIND and
//! Generalization): a binding generalizes over the variables not free
//! in the environment and carries their obligations; a variable held
//! only by literals defaults first; an obligation whose variable left
//! the type is ambiguous, or `DotNeedsType` for a member; an annotation
//! quantifies its written names; a `pub` binding ends closed. A member
//! obligation's type stays with its head.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/types.md

use super::{
    is_prelude, num, prelude, Error, Expected, Infer, Lit, Local, Owed, Reason,
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
    pub public: bool,
}

impl Infer<'_> {
    /// `pattern = value` (BIND). An arm binding's own name is in scope
    /// in its arms, monomorphic there and generalized after.
    pub(super) fn bind(&mut self, id: ExprId, file: bool) -> Option<Lift> {
        let module = self.module;
        let ExprKind::Bind {
            public,
            pattern,
            annotation,
            value,
        } = &module.expressions[id.0].kind
        else {
            return None;
        };
        let mark = self.mark();
        let env = self.state.env.len();
        let mut expected = annotation.map(|annotation| {
            let span = module.types[annotation.0].span;
            Expected::new(self.lower_here(annotation), span)
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
                    public: *public,
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
                self.state.env.push(ty.clone());
            }
        }
        let good = match &pattern_node.kind {
            PatternKind::Outcome { good: true, .. } => true,
            PatternKind::Constructor { .. } => matches!(
                self.resolved.pattern(*pattern),
                Some(Target::Constructor { origin: Origin::Prelude, name, .. })
                    if name == "Just" || name == "Ok"
            ),
            _ => false,
        };
        (good && !file).then_some(Lift {
            value: ty,
            span: pattern_node.span,
        })
    }

    pub(super) const fn mark(&self) -> Mark {
        Mark {
            obligations: self.state.obligations.len(),
            errors: self.state.errors.len(),
        }
    }

    /// Whether `ty` is a variable owing `Num` or `Float`: a number whose
    /// type is still open, never an outcome or a function.
    pub(super) fn numeric_var(&self, ty: &Type) -> bool {
        let ty = self.apply(ty);
        matches!(ty, Type::Var(_))
            && self.state.obligations.iter().any(|owed| {
                matches!(&owed.obligation, Obligation::Class { class, ty: t }
                    if matches!(class.as_str(), "Num" | "Float")
                        && self.apply(t) == ty)
            })
    }

    /// VAR: fresh variables, and the scheme's obligations owed at `span`.
    pub(super) fn instantiate(&mut self, scheme: &Scheme, span: Span) -> Type {
        let (ty, obligations) = scheme.instantiate(&mut self.state.subst);
        for obligation in obligations {
            self.state.obligations.push(Owed::new(obligation, span));
        }
        ty
    }

    /// The free variables of the environment: every monomorphic type in
    /// scope, every use held until its group is typed, the session's
    /// weak variables, and every item typed without a scheme, except
    /// `group`.
    fn environment(&self, group: &[usize]) -> Vec<Var> {
        let mut vars = Vec::new();
        for ty in &self.state.env {
            self.apply(ty).variables(&mut vars);
        }
        for used in &self.state.deferred {
            self.apply(&used.ty).variables(&mut vars);
        }
        for var in 0..self.weak {
            self.apply(&Type::Var(Var(var))).variables(&mut vars);
        }
        for (index, item) in self.items.iter().enumerate() {
            let live = match self.state.status[index] {
                Status::Typing => !group.contains(&index),
                Status::Pending | Status::Done(_) => false,
            };
            if live {
                self.apply(&item.ty).variables(&mut vars);
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
            for owed in self.state.obligations.iter().filter(|o| !o.solved) {
                let Obligation::Member { head, ty, .. } = &owed.obligation
                else {
                    continue;
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

    /// Generalize `parts`, typed together, over the obligations owed
    /// since `mark`. An error while typing them silences ambiguity.
    pub(super) fn generalize(
        &mut self,
        parts: &[Part],
        mark: Mark,
        group: &[usize],
    ) -> Vec<Scheme> {
        self.dots();
        self.derive(mark.obligations);
        let env = self.environment(group);
        let quiet = self.state.errors.len() > mark.errors;
        let mark = mark.obligations;
        let reach: Vec<Vec<Var>> = parts
            .iter()
            .map(|part| {
                let mut vars = Vec::new();
                self.apply(&part.ty).variables(&mut vars);
                self.close(&mut vars);
                vars
            })
            .collect();
        self.default_literals(mark, &env, &reach.concat());
        let types: Vec<Type> =
            parts.iter().map(|p| self.apply(&p.ty)).collect();
        let mut carried = alloc::vec![Vec::new(); parts.len()];
        let mut kept = Vec::new();
        let mut left: Vec<Obligation> = Vec::new();
        for owed in self.state.obligations.split_off(mark) {
            if owed.solved {
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
            .map(|((part, ty), obligations)| {
                self.scheme(part, ty, obligations, &env)
            })
            .collect()
    }

    /// Derived `Eq` and `Ord` on a type with arguments owe the same on
    /// each argument (types.md §Traits), down to the variables.
    fn derive(&mut self, mark: usize) {
        let mut pending = self.state.obligations.split_off(mark);
        pending.reverse();
        while let Some(owed) = pending.pop() {
            let Obligation::Class { class, ty } = &owed.obligation else {
                self.state.obligations.push(owed);
                continue;
            };
            let arguments = match self.apply(ty) {
                Type::Named { arguments, .. } if !arguments.is_empty() => {
                    arguments
                }
                Type::Array { element, .. } => alloc::vec![*element],
                _ => Vec::new(),
            };
            if arguments.is_empty() || !matches!(class.as_str(), "Eq" | "Ord") {
                self.state.obligations.push(owed);
                continue;
            }
            for argument in arguments.into_iter().rev() {
                let obligation = Obligation::Class {
                    class: class.clone(),
                    ty: argument,
                };
                pending.push(Owed::new(obligation, owed.span));
            }
        }
    }

    /// Obligations whose variable left the type: `DotNeedsType` at a
    /// member's dot, else ambiguous at `site`, once per class.
    fn left(&mut self, left: Vec<Obligation>, site: Span) {
        let mut errors: Vec<Error> = Vec::new();
        for obligation in left {
            let error = match obligation {
                Obligation::Member { dot, .. } => Error {
                    span: dot,
                    reason: Reason::DotNeedsType,
                },
                Obligation::Class { class, .. } => Error {
                    span: site,
                    reason: Reason::Ambiguous { class },
                },
            };
            if !errors.contains(&error) {
                errors.push(error);
            }
        }
        self.state.errors.extend(errors);
    }

    fn scheme(
        &mut self,
        part: &Part,
        ty: Type,
        obligations: Vec<Obligation>,
        env: &[Var],
    ) -> Scheme {
        let mut variables = Vec::new();
        ty.variables(&mut variables);
        if part.public && !variables.is_empty() {
            self.error(part.site, Reason::NotComplete);
        }
        if !part.annotated {
            for obligation in &obligations {
                obligation.variables(&mut variables);
            }
            variables.retain(|var| !env.contains(var));
            return Scheme {
                variables,
                obligations,
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
        Scheme {
            variables: fresh.iter().map(|(_, var)| *var).collect(),
            obligations: Vec::new(),
            ty: unrigid(&ty, &fresh),
        }
    }

    /// A line of the implicit main generalizes nothing: its variables
    /// leave there and default if they can. A trait obligation still on
    /// a variable is then ambiguous, in the line's type or not (types.md
    /// §Core judgments: "a line whose target is still unknown is an
    /// ambiguity error"); a member obligation is `DotNeedsType` once its
    /// head has left the line's type.
    pub(super) fn line_boundary(&mut self, ty: &Type, span: Span, mark: Mark) {
        self.dots();
        self.derive(mark.obligations);
        let env = self.environment(&[]);
        self.default_literals(mark.obligations, &env, &[]);
        let mut stay = Vec::new();
        self.apply(ty).variables(&mut stay);
        self.close(&mut stay);
        let quiet = self.state.errors.len() > mark.errors;
        let mut left = Vec::new();
        for owed in self.state.obligations.split_off(mark.obligations) {
            if owed.solved {
                continue;
            }
            let class = matches!(owed.obligation, Obligation::Class { .. });
            match self.apply(owed.obligation.subject()) {
                Type::Var(var)
                    if !env.contains(&var)
                        && (class || !stay.contains(&var)) =>
                {
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
    /// has left it: `DotNeedsType`.
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
            let Obligation::Member { head, .. } = &owed.obligation else {
                continue;
            };
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

    /// Among the obligations since `mark`, a variable outside `env`
    /// that only literals hold becomes `i64`, or `f64` for a float
    /// literal. One not in `stay` becomes the same where it leaves when
    /// it owes `Num`, `Float` or a literal and otherwise at most `Eq`
    /// and `Ord`.
    pub(super) fn default_literals(
        &mut self,
        mark: usize,
        env: &[Var],
        stay: &[Var],
    ) {
        struct Held {
            var: Var,
            only_literals: bool,
            /// `f64` for a `Float`, else `i64` for a `Num`.
            numeric: Option<&'static str>,
            other: bool,
        }
        let mut seen: Vec<Held> = Vec::new();
        for owed in &self.state.obligations[mark..] {
            let Type::Var(var) = self.apply(owed.obligation.subject()) else {
                continue;
            };
            if owed.solved || env.contains(&var) {
                continue;
            }
            let index =
                seen.iter().position(|s| s.var == var).unwrap_or_else(|| {
                    seen.push(Held {
                        var,
                        only_literals: true,
                        numeric: None,
                        other: false,
                    });
                    seen.len() - 1
                });
            let held = &mut seen[index];
            held.only_literals &= owed.literal.is_some();
            let Obligation::Class { class, .. } = &owed.obligation else {
                held.other = true;
                continue;
            };
            let class = class.as_str();
            match class {
                "Float" => held.numeric = Some("f64"),
                "Num" => held.numeric = held.numeric.or(Some("i64")),
                "Eq" | "Ord" => {}
                _ => held.other = true,
            }
        }
        for held in seen {
            let defaults = if stay.contains(&held.var) {
                held.only_literals
            } else {
                !held.other
            };
            if let Some(name) = held.numeric.filter(|_| defaults) {
                self.hint(&Type::Var(held.var), &prelude(name));
            }
        }
    }

    /// Check what the module still owes and return what is left on
    /// variables. `Num` is the one class checked here (the stub).
    pub(super) fn settle(&mut self) -> Vec<Obligation> {
        let mut unsolved = Vec::new();
        for owed in core::mem::take(&mut self.state.obligations) {
            if owed.solved {
                continue;
            }
            let obligation = owed.obligation.map(|ty| self.apply(ty));
            if let Obligation::Class { class, ty } = &obligation {
                match owed.literal {
                    Some(literal) => self.check_literal(literal, ty, owed.span),
                    None if class == "Num" && num(ty) == Some(false) => {
                        self.error(owed.span, Reason::NotNumeric);
                    }
                    None => {}
                }
            }
            if matches!(obligation.subject(), Type::Var(_))
                && !unsolved.contains(&obligation)
            {
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
    ) {
        let float = is_prelude(ty, "f32") || is_prelude(ty, "f64");
        let actual = match literal {
            _ if matches!(ty, Type::Var(_)) || float => return,
            Lit::Int {
                negative,
                magnitude,
            } => match in_range(ty, negative, magnitude) {
                Some(true) => return,
                Some(false) => {
                    self.error(span, Reason::OutOfRange);
                    return;
                }
                None => prelude("i64"),
            },
            Lit::Float => prelude("f64"),
        };
        let expected = ty.clone();
        self.error(
            span,
            Reason::Mismatch {
                expected,
                actual,
                other: span,
            },
        );
    }
}

/// Whether an integer literal fits an integer type; `None` for any
/// other type.
pub(super) fn in_range(
    ty: &Type,
    negative: bool,
    magnitude: u64,
) -> Option<bool> {
    const INTEGERS: [&str; 8] =
        ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"];
    let index = INTEGERS.iter().position(|name| is_prelude(ty, name))?;
    let width = 8u32 << (index % 4);
    Some(if index < 4 {
        let bound = 1u64 << (width - 1);
        magnitude < bound || (negative && magnitude == bound)
    } else {
        magnitude <= u64::MAX >> (64 - width) && (!negative || magnitude == 0)
    })
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
        Type::Array { element, .. } => rigids(element, out),
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
        Type::Array { element, length } => Type::Array {
            element: Box::new(unrigid(element, fresh)),
            length: *length,
        },
        Type::Function { parameter, result } => {
            Type::function(unrigid(parameter, fresh), unrigid(result, fresh))
        }
        Type::Unit | Type::Var(_) => ty.clone(),
    }
}
