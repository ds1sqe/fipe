//! The type representation of [types.md]: types, schemes, substitution
//! and unification. Nothing here reads syntax; inference does.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md

use crate::{resolve::Origin, token::Span};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;

/// An inference variable, an index into a [`Subst`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Var(pub u32);

/// A declared type's identity: where it was declared and its name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypeName {
    pub origin: Origin,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Unit,
    /// A literal number argument of a holding or container.
    Number(crate::token::Magnitude),
    Var(Var),
    /// A rigid variable, written in an annotation.
    Rigid(String),
    Named {
        name: TypeName,
        arguments: Vec<Self>,
    },
    /// Canonical sum: sorted variables followed by a nonzero constant.
    Sum(Vec<Self>),
    Tuple(Vec<Self>),
    Either(Vec<Self>),
    Both(Vec<Self>),
    Not(Box<Self>),
    Limited {
        name: Box<Self>,
        meaning: Box<Self>,
    },
    Array {
        element: Box<Self>,
        length: Box<Self>,
    },
    Function {
        parameter: Box<Self>,
        result: Box<Self>,
    },
}

impl Type {
    /// Normalize addition without evaluating a program.
    #[must_use]
    pub fn sum(parts: Vec<Self>) -> Self {
        let mut terms = Vec::new();
        let mut constant = crate::token::Magnitude::default();
        let mut pending = parts;
        while let Some(part) = pending.pop() {
            match part {
                Self::Sum(parts) => pending.extend(parts),
                Self::Number(n) => constant = constant.add(&n),
                other => terms.push(other),
            }
        }
        terms.sort_by(number_order);
        if constant != crate::token::Magnitude::default() || terms.is_empty() {
            terms.push(Self::Number(constant));
        }
        if terms.len() == 1 {
            terms.remove(0)
        } else {
            Self::Sum(terms)
        }
    }

    /// A constant number argument, when this number has no variables.
    #[must_use]
    pub const fn number(&self) -> Option<&crate::token::Magnitude> {
        if let Self::Number(n) = self {
            Some(n)
        } else {
            None
        }
    }
    #[must_use]
    pub fn named(origin: Origin, name: &str, arguments: Vec<Self>) -> Self {
        Self::Named {
            name: TypeName {
                origin,
                name: String::from(name),
            },
            arguments,
        }
    }

    #[must_use]
    pub fn function(parameter: Self, result: Self) -> Self {
        Self::Function {
            parameter: Box::new(parameter),
            result: Box::new(result),
        }
    }

    /// The meaning used when binding an inference variable. Limits belong
    /// to declared places, including places nested inside a container.
    #[must_use]
    pub fn without_limits(&self) -> Self {
        match self {
            Self::Limited { meaning, .. } => meaning.without_limits(),
            Self::Named { name, arguments } => Self::Named {
                name: name.clone(),
                arguments: arguments.iter().map(Self::without_limits).collect(),
            },
            Self::Sum(parts) => {
                Self::sum(parts.iter().map(Self::without_limits).collect())
            }
            Self::Tuple(parts) => {
                Self::Tuple(parts.iter().map(Self::without_limits).collect())
            }
            Self::Either(parts) => {
                Self::Either(parts.iter().map(Self::without_limits).collect())
            }
            Self::Both(parts) => {
                Self::Both(parts.iter().map(Self::without_limits).collect())
            }
            Self::Not(inner) => Self::Not(Box::new(inner.without_limits())),
            Self::Array { element, length } => Self::Array {
                element: Box::new(element.without_limits()),
                length: Box::new(length.without_limits()),
            },
            Self::Function { parameter, result } => Self::function(
                parameter.without_limits(),
                result.without_limits(),
            ),
            Self::Unit | Self::Number(_) | Self::Var(_) | Self::Rigid(_) => {
                self.clone()
            }
        }
    }

    /// The inference variables in this type, in order of appearance.
    pub fn variables(&self, out: &mut Vec<Var>) {
        match self {
            Self::Unit | Self::Number(_) | Self::Rigid(_) => {}
            Self::Var(var) => {
                if !out.contains(var) {
                    out.push(*var);
                }
            }
            Self::Named { arguments, .. } => {
                for argument in arguments {
                    argument.variables(out);
                }
            }
            Self::Not(inner) => inner.variables(out),
            Self::Sum(parts)
            | Self::Tuple(parts)
            | Self::Either(parts)
            | Self::Both(parts) => {
                for p in parts {
                    p.variables(out);
                }
            }
            Self::Limited { name, .. } => name.variables(out),
            Self::Array { element, length } => {
                element.variables(out);
                length.variables(out);
            }
            Self::Function { parameter, result } => {
                parameter.variables(out);
                result.variables(out);
            }
        }
    }

    /// This type with the declarations of the module it was typed in,
    /// `Origin::This`, named by `origin`: how a file module's or an
    /// accepted line's types keep their identity outside it.
    #[must_use]
    pub fn rehome(&self, origin: &Origin) -> Self {
        self.renamed(&Origin::This, origin)
    }

    /// The inverse of [`Self::rehome`]: `origin`'s declarations named
    /// as its own module names them, for an expression typed in it.
    #[must_use]
    pub fn unhome(&self, origin: &Origin) -> Self {
        self.renamed(origin, &Origin::This)
    }

    fn renamed(&self, from: &Origin, to: &Origin) -> Self {
        let renamed = |ty: &Self| ty.renamed(from, to);
        match self {
            Self::Named { name, arguments } => Self::Named {
                name: TypeName {
                    origin: if name.origin == *from {
                        to.clone()
                    } else {
                        name.origin.clone()
                    },
                    name: name.name.clone(),
                },
                arguments: arguments.iter().map(renamed).collect(),
            },
            Self::Sum(parts) => Self::sum(parts.iter().map(renamed).collect()),
            Self::Tuple(parts) => {
                Self::Tuple(parts.iter().map(renamed).collect())
            }
            Self::Either(parts) => {
                Self::Either(parts.iter().map(renamed).collect())
            }
            Self::Both(parts) => {
                Self::Both(parts.iter().map(renamed).collect())
            }
            Self::Not(inner) => Self::Not(Box::new(renamed(inner))),
            Self::Limited { name, meaning } => Self::Limited {
                name: Box::new(renamed(name)),
                meaning: Box::new(renamed(meaning)),
            },
            Self::Array { element, length } => Self::Array {
                element: Box::new(renamed(element)),
                length: Box::new(renamed(length)),
            },
            Self::Function { parameter, result } => {
                Self::function(renamed(parameter), renamed(result))
            }
            Self::Unit | Self::Number(_) | Self::Var(_) | Self::Rigid(_) => {
                self.clone()
            }
        }
    }

    /// Whether `var` occurs in this type.
    #[must_use]
    pub fn contains(&self, var: Var) -> bool {
        let mut vars = Vec::new();
        self.variables(&mut vars);
        vars.contains(&var)
    }
}

/// A member required of a type not yet known, `a.name: t`, found at
/// the dot `dot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Obligation {
    Claim {
        head: Type,
        class: Type,
        dot: Span,
    },
    Member {
        head: Type,
        name: String,
        ty: Type,
        dot: Span,
    },
}

impl Obligation {
    /// The head whose member is required.
    #[must_use]
    pub const fn subject(&self) -> &Type {
        match self {
            Self::Member { head: ty, .. } | Self::Claim { head: ty, .. } => ty,
        }
    }

    /// The same obligation with `f` applied to each of its types.
    #[must_use]
    pub fn map(&self, mut f: impl FnMut(&Type) -> Type) -> Self {
        match self {
            Self::Claim { head, class, dot } => Self::Claim {
                head: f(head),
                class: f(class),
                dot: *dot,
            },
            Self::Member {
                head,
                name,
                ty,
                dot,
            } => Self::Member {
                head: f(head),
                name: name.clone(),
                ty: f(ty),
                dot: *dot,
            },
        }
    }

    /// The inference variables of its types, in order of appearance.
    pub fn variables(&self, out: &mut Vec<Var>) {
        match self {
            Self::Claim { head, class, .. } => {
                head.variables(out);
                class.variables(out);
            }
            Self::Member { head, ty, .. } => {
                head.variables(out);
                ty.variables(out);
            }
        }
    }
}

/// A numeric literal whose type is left to a variable or a rigid name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lit {
    /// `magnitude` is `None` past `u64`, outside every width.
    Int {
        negative: bool,
        magnitude: crate::token::Magnitude,
    },
    /// `whole` is the integer the spelling denotes exactly, if any.
    Float {
        whole: Option<(bool, crate::token::Magnitude)>,
    },
    Text,
    Bytes,
}

impl Lit {
    /// The form of a written literal; `None` for `()` and an affix that
    /// is not bytes.
    pub(crate) fn of(literal: &crate::ast::Literal) -> Option<Self> {
        use crate::ast::Literal;
        Some(match literal {
            Literal::Int {
                negative,
                magnitude,
                ..
            } => Self::Int {
                negative: *negative,
                magnitude: magnitude.clone(),
            },
            Literal::Float(text) => Self::Float {
                whole: crate::token::Magnitude::whole_of_fraction(text),
            },
            Literal::Str { .. } => Self::Text,
            Literal::Affixed(a) if crate::resolve::bytes(a).is_some() => {
                Self::Bytes
            }
            Literal::Unit | Literal::Affixed(_) => return None,
        })
    }
}

/// `∀ variables. obligations ⇒ ty`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub variables: Vec<Var>,
    pub obligations: Vec<Obligation>,
    pub(crate) literals: Vec<(Type, Lit, Span)>,
    pub ty: Type,
}

impl Scheme {
    /// A type quantified over nothing.
    #[must_use]
    pub const fn mono(ty: Type) -> Self {
        Self {
            variables: Vec::new(),
            obligations: Vec::new(),
            literals: Vec::new(),
            ty,
        }
    }

    /// The scheme with its module's declarations named by `origin`
    /// ([`Type::rehome`]).
    #[must_use]
    pub fn rehome(&self, origin: &Origin) -> Self {
        Self {
            variables: self.variables.clone(),
            obligations: self
                .obligations
                .iter()
                .map(|obligation| obligation.map(|ty| ty.rehome(origin)))
                .collect(),
            literals: self
                .literals
                .iter()
                .map(|(ty, form, span)| {
                    (ty.rehome(origin), form.clone(), *span)
                })
                .collect(),
            ty: self.ty.rehome(origin),
        }
    }

    /// Quantify over every variable of the applied type and obligations
    /// that is not free in the environment (BIND).
    #[must_use]
    pub fn generalize(
        ty: &Type,
        obligations: &[Obligation],
        environment: &[Var],
        subst: &Subst,
    ) -> Self {
        let ty = subst.apply(ty);
        let obligations: Vec<_> = obligations
            .iter()
            .map(|obligation| obligation.map(|ty| subst.apply(ty)))
            .collect();
        let mut variables = Vec::new();
        ty.variables(&mut variables);
        for obligation in &obligations {
            obligation.variables(&mut variables);
        }
        variables.retain(|var| !environment.contains(var));
        Self {
            variables,
            obligations,
            literals: Vec::new(),
            ty,
        }
    }

    /// Replace every quantified variable with a fresh one (VAR).
    #[must_use]
    pub fn instantiate(&self, subst: &mut Subst) -> (Type, Vec<Obligation>) {
        let fresh: Vec<_> = self
            .variables
            .iter()
            .map(|var| (*var, Type::Var(subst.fresh())))
            .collect();
        let ty = replace(&self.ty, &fresh);
        let obligations = self
            .obligations
            .iter()
            .map(|obligation| obligation.map(|ty| replace(ty, &fresh)))
            .collect();
        (ty, obligations)
    }
}

/// `ty` with each listed variable replaced.
pub(crate) fn replace(ty: &Type, fresh: &[(Var, Type)]) -> Type {
    match ty {
        Type::Var(var) => fresh
            .iter()
            .find(|(old, _)| old == var)
            .map_or_else(|| ty.clone(), |(_, new)| new.clone()),
        Type::Unit | Type::Number(_) | Type::Rigid(_) => ty.clone(),
        Type::Named { name, arguments } => Type::Named {
            name: name.clone(),
            arguments: arguments.iter().map(|a| replace(a, fresh)).collect(),
        },
        Type::Sum(parts) => {
            Type::sum(parts.iter().map(|p| replace(p, fresh)).collect())
        }
        Type::Tuple(parts) => {
            Type::Tuple(parts.iter().map(|p| replace(p, fresh)).collect())
        }
        Type::Either(parts) => {
            Type::Either(parts.iter().map(|p| replace(p, fresh)).collect())
        }
        Type::Both(parts) => {
            Type::Both(parts.iter().map(|p| replace(p, fresh)).collect())
        }
        Type::Not(inner) => Type::Not(Box::new(replace(inner, fresh))),
        Type::Limited { name, meaning } => Type::Limited {
            name: Box::new(replace(name, fresh)),
            meaning: Box::new(replace(meaning, fresh)),
        },
        Type::Array { element, length } => Type::Array {
            element: Box::new(replace(element, fresh)),
            length: Box::new(replace(length, fresh)),
        },
        Type::Function { parameter, result } => {
            Type::function(replace(parameter, fresh), replace(result, fresh))
        }
    }
}

/// Why two types did not unify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    DepthLimit,
    Different {
        expected: Type,
        actual: Type,
    },
    /// `a` against a type containing `a`: no finite solution.
    Occurs {
        variable: Var,
        ty: Type,
    },
}

/// The solved variables so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subst {
    bindings: Vec<Option<Type>>,
    lengths: Vec<(Type, Type)>,
}

impl Subst {
    /// A substitution over `bindings`, one per variable from `Var(0)`.
    #[must_use]
    pub(crate) const fn from_bindings(bindings: Vec<Option<Type>>) -> Self {
        Self {
            bindings,
            lengths: Vec::new(),
        }
    }

    /// Each variable's solution, from `Var(0)`.
    #[must_use]
    pub(crate) fn bindings(&self) -> &[Option<Type>] {
        &self.bindings
    }

    /// How many variables it has made.
    #[must_use]
    pub(crate) fn count(&self) -> u32 {
        u32::try_from(self.bindings.len()).unwrap_or(u32::MAX)
    }

    /// Which variables are solved, from `Var(0)`.
    #[must_use]
    pub(crate) fn solved(&self) -> Vec<bool> {
        self.bindings.iter().map(Option::is_some).collect()
    }

    /// The solutions made since `solved` was taken, with the module's
    /// own declarations named by `origin` ([`Type::rehome`]): what an
    /// expression typed inside that module solved for its user.
    pub(crate) fn rehome_since(&mut self, solved: &[bool], origin: &Origin) {
        for (index, binding) in self.bindings.iter_mut().enumerate() {
            if solved.get(index).copied().unwrap_or(false) {
                continue;
            }
            if let Some(ty) = binding {
                *ty = ty.rehome(origin);
            }
        }
    }

    pub fn fresh(&mut self) -> Var {
        self.bindings.push(None);
        Var(u32::try_from(self.bindings.len() - 1).unwrap_or(u32::MAX))
    }

    /// The type with every solved variable replaced, to a fixed point.
    #[must_use]
    pub fn apply(&self, ty: &Type) -> Type {
        match ty {
            Type::Var(var) => self
                .get(*var)
                .map_or_else(|| ty.clone(), |bound| self.apply(bound)),
            Type::Unit | Type::Number(_) | Type::Rigid(_) => ty.clone(),
            Type::Named { name, arguments } => Type::Named {
                name: name.clone(),
                arguments: arguments.iter().map(|a| self.apply(a)).collect(),
            },
            Type::Sum(parts) => {
                Type::sum(parts.iter().map(|p| self.apply(p)).collect())
            }
            Type::Tuple(parts) => {
                Type::Tuple(parts.iter().map(|p| self.apply(p)).collect())
            }
            Type::Either(parts) => {
                Type::Either(parts.iter().map(|p| self.apply(p)).collect())
            }
            Type::Both(parts) => {
                Type::Both(parts.iter().map(|p| self.apply(p)).collect())
            }
            Type::Not(inner) => Type::Not(Box::new(self.apply(inner))),
            Type::Limited { name, meaning } => Type::Limited {
                name: Box::new(self.apply(name)),
                meaning: Box::new(self.apply(meaning)),
            },
            Type::Array { element, length } => Type::Array {
                element: Box::new(self.apply(element)),
                length: Box::new(self.apply(length)),
            },
            Type::Function { parameter, result } => {
                Type::function(self.apply(parameter), self.apply(result))
            }
        }
    }

    fn unify_sum(
        &mut self,
        expected: &Type,
        actual: &Type,
    ) -> Result<(), Box<Mismatch>> {
        fn terms(ty: &Type) -> (Vec<Type>, crate::token::Magnitude) {
            let parts = match ty {
                Type::Sum(parts) => parts.clone(),
                other => alloc::vec![other.clone()],
            };
            let mut names = Vec::new();
            let mut constant = crate::token::Magnitude::default();
            for part in parts {
                if let Type::Number(n) = part {
                    constant = constant.add(&n);
                } else {
                    names.push(part);
                }
            }
            (names, constant)
        }
        let (left_terms, a) = terms(expected);
        let (right_terms, b) = terms(actual);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let (mut l, mut r) = (
            left_terms.into_iter().peekable(),
            right_terms.into_iter().peekable(),
        );
        while let (Some(a), Some(b)) = (l.peek(), r.peek()) {
            match number_order(a, b) {
                core::cmp::Ordering::Equal => {
                    l.next();
                    r.next();
                }
                core::cmp::Ordering::Less => left.extend(l.next()),
                core::cmp::Ordering::Greater => right.extend(r.next()),
            }
        }
        left.extend(l);
        right.extend(r);
        for (names, others, constant, other_constant) in
            [(&left, &right, &a, &b), (&right, &left, &b, &a)]
        {
            if let [Type::Var(var)] = names.as_slice() {
                if let Some(difference) = other_constant.checked_sub(constant) {
                    let mut parts = others.clone();
                    parts.push(Type::Number(difference));
                    let ty = Type::sum(parts);
                    if ty.contains(*var) {
                        return Err(Box::new(Mismatch::Occurs {
                            variable: *var,
                            ty,
                        }));
                    }
                    self.bindings[var.0 as usize] = Some(ty);
                    return Ok(());
                }
                if others.is_empty() {
                    return Err(Box::new(Mismatch::Different {
                        expected: expected.clone(),
                        actual: actual.clone(),
                    }));
                }
            }
        }
        let unknown =
            left.iter().chain(&right).any(|t| matches!(t, Type::Var(_)));
        if unknown {
            let pair = (expected.clone(), actual.clone());
            if !self.lengths.contains(&pair) {
                self.lengths.push(pair);
            }
            Ok(())
        } else if left.is_empty() && right.is_empty() && a == b {
            Ok(())
        } else {
            Err(Box::new(Mismatch::Different {
                expected: expected.clone(),
                actual: actual.clone(),
            }))
        }
    }

    /// Retry delayed length equations after their operands have been fixed.
    /// Returns whether any length is still ambiguous.
    ///
    /// # Errors
    /// A now-known equation has no nonnegative solution.
    pub fn settle_lengths(&mut self) -> Result<bool, Box<Mismatch>> {
        if self.lengths.is_empty() {
            return Ok(false);
        }
        loop {
            let before = self.bindings.clone();
            let pending = core::mem::take(&mut self.lengths);
            for (a, b) in pending {
                self.unify(&a, &b)?;
            }
            if self.bindings == before {
                return Ok(!self.lengths.is_empty());
            }
        }
    }

    /// Byte literal defaulting retains its nested U8 place shape.
    /// Ordinary variable binding still erases limits.
    pub(crate) fn default_bytes(&mut self, var: Var, ty: Type) {
        debug_assert_eq!(self.apply(&Type::Var(var)), Type::Var(var));
        self.bindings[var.0 as usize] = Some(ty);
    }

    fn get(&self, var: Var) -> Option<&Type> {
        self.bindings.get(var.0 as usize).and_then(Option::as_ref)
    }

    /// Make `expected` and `actual` equal, binding variables; nominal
    /// names must match by declaration identity, rigid variables only
    /// themselves.
    ///
    /// # Errors
    /// The first pair that cannot be made equal, after substitution.
    pub fn unify(
        &mut self,
        expected: &Type,
        actual: &Type,
    ) -> Result<(), Box<Mismatch>> {
        if self.too_deep(expected) || self.too_deep(actual) {
            return Err(Box::new(Mismatch::DepthLimit));
        }
        let expected = self.apply(expected);
        let actual = self.apply(actual);
        if expected == actual {
            return Ok(());
        }
        if matches!(expected, Type::Sum(_)) || matches!(actual, Type::Sum(_)) {
            return self.unify_sum(&expected, &actual);
        }
        match (&expected, &actual) {
            (Type::Var(a), Type::Var(b)) if a == b => Ok(()),
            (Type::Var(a), Type::Var(b)) => {
                // Keep fresh operands attached to the existing variable,
                // rather than growing a chain for every literal in a sum.
                self.bindings[core::cmp::max(a, b).0 as usize] =
                    Some(Type::Var(*core::cmp::min(a, b)));
                Ok(())
            }
            (Type::Var(var), other) | (other, Type::Var(var)) => {
                if other.contains(*var) {
                    return Err(Box::new(Mismatch::Occurs {
                        variable: *var,
                        ty: other.clone(),
                    }));
                }
                self.bindings[var.0 as usize] = Some(other.without_limits());
                Ok(())
            }
            (Type::Limited { meaning, .. }, other)
            | (other, Type::Limited { meaning, .. }) => {
                self.unify(meaning, other)
            }
            (Type::Tuple(a), Type::Tuple(b))
            | (Type::Either(a), Type::Either(b))
                if a.len() == b.len() =>
            {
                for (a, b) in a.iter().zip(b) {
                    self.unify(a, b)?;
                }
                Ok(())
            }
            (Type::Unit, Type::Unit) => Ok(()),
            (Type::Number(a), Type::Number(b)) if a == b => Ok(()),
            (Type::Rigid(a), Type::Rigid(b)) if a == b => Ok(()),
            (
                Type::Named {
                    name: a,
                    arguments: x,
                },
                Type::Named {
                    name: b,
                    arguments: y,
                },
            ) if a == b && x.len() == y.len() => {
                for (x, y) in x.iter().zip(y) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (
                Type::Array {
                    element: a,
                    length: n,
                },
                Type::Array {
                    element: b,
                    length: m,
                },
            ) => {
                self.unify(n, m)?;
                self.unify(a, b)
            }
            (
                Type::Function {
                    parameter: a,
                    result: x,
                },
                Type::Function {
                    parameter: b,
                    result: y,
                },
            ) => {
                self.unify(a, b)?;
                self.unify(x, y)
            }
            _ => Err(Box::new(Mismatch::Different { expected, actual })),
        }
    }
}

/// A type in the spec's syntax; its inference variables print as `a`,
/// `b`, … in order of appearance.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut variables = Vec::new();
        self.variables(&mut variables);
        write(self, f, &letters(&variables), &[])
    }
}

/// A scheme in the spec's syntax: `Add<b> a => a -> b -> a`, or
/// `a.age: b => b -> a -> Bool` for a member obligation, its quantified
/// variables named `a`, `b`, … in order, and any variable it does not
/// quantify by the letters after them.
impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut variables = self.variables.clone();
        for obligation in &self.obligations {
            obligation.variables(&mut variables);
        }
        self.ty.variables(&mut variables);
        let names = letters(&variables);
        let mut remaining = Vec::new();
        for obligation in &self.obligations {
            let key = obligation.map(Clone::clone);
            if !remaining.iter().any(|o: &Obligation| match (o, &key) {
                (
                    Obligation::Member {
                        head: a,
                        name: b,
                        ty: c,
                        ..
                    },
                    Obligation::Member { head, name, ty, .. },
                ) => a == head && b == name && c == ty,
                (
                    Obligation::Claim {
                        head: a, class: b, ..
                    },
                    Obligation::Claim { head, class, .. },
                ) => a == head && b == class,
                _ => false,
            }) {
                remaining.push(key);
            }
        }
        remaining.sort_by_key(|o| matches!(o, Obligation::Claim { class: Type::Named { name, .. }, .. } if name.name == "Literal"));
        for (index, obligation) in remaining.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            match obligation {
                Obligation::Member { head, name, ty, .. } => {
                    write(head, f, &names, &[])?;
                    write!(f, ".{name}: ")?;
                    write(ty, f, &names, &[])?;
                }
                Obligation::Claim { head, class, .. } => {
                    write(class, f, &names, &[])?;
                    f.write_str(" ")?;
                    write(head, f, &names, &[])?;
                }
            }
        }
        if !self.obligations.is_empty() {
            f.write_str(" => ")?;
        }
        write(&self.ty, f, &names, &[])
    }
}

/// A type written beside another in one message, as [`Type::beside`]
/// makes it.
pub struct Beside<'a> {
    ty: &'a Type,
    other: &'a Type,
}

impl Type {
    /// This type written for a message that also names `other`: a name
    /// the two give to different declarations is written with its
    /// module, `primes.Prime` against this module's `Prime`.
    #[must_use]
    pub const fn beside<'a>(&'a self, other: &'a Self) -> Beside<'a> {
        Beside { ty: self, other }
    }

    /// The declared names in this type, each with its origin.
    fn declared<'a>(&'a self, out: &mut Vec<&'a TypeName>) {
        match self {
            Self::Unit | Self::Number(_) | Self::Var(_) | Self::Rigid(_) => {}
            Self::Named { name, arguments } => {
                out.push(name);
                for argument in arguments {
                    argument.declared(out);
                }
            }
            Self::Not(inner) => inner.declared(out),
            Self::Sum(parts)
            | Self::Tuple(parts)
            | Self::Either(parts)
            | Self::Both(parts) => {
                for p in parts {
                    p.declared(out);
                }
            }
            Self::Limited { name, .. } => name.declared(out),
            Self::Array { element, length } => {
                element.declared(out);
                length.declared(out);
            }
            Self::Function { parameter, result } => {
                parameter.declared(out);
                result.declared(out);
            }
        }
    }
}

impl fmt::Display for Beside<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut declared = Vec::new();
        self.ty.declared(&mut declared);
        self.other.declared(&mut declared);
        let mut clashes: Vec<String> = Vec::new();
        for (index, a) in declared.iter().enumerate() {
            let clash = declared[index + 1..]
                .iter()
                .any(|b| a.name == b.name && a.origin != b.origin);
            if clash && !clashes.contains(&a.name) {
                clashes.push(a.name.clone());
            }
        }
        let mut variables = Vec::new();
        self.ty.variables(&mut variables);
        write(self.ty, f, &letters(&variables), &clashes)
    }
}

fn letters(variables: &[Var]) -> Vec<(Var, String)> {
    variables
        .iter()
        .enumerate()
        .map(|(index, var)| (*var, letter(index)))
        .collect()
}

fn letter(index: usize) -> String {
    let mut name = String::new();
    let letter = u8::try_from(index % 26).unwrap_or(0) + b'a';
    name.push(char::from(letter));
    if index >= 26 {
        write_number(&mut name, index / 26);
    }
    name
}

fn write_number(out: &mut String, number: usize) {
    use fmt::Write as _;
    let _ = write!(out, "{number}");
}

/// Write `ty`; a declared name among `clashes` is written with its
/// module when that is another module.
fn precedence(ty: &Type) -> u8 {
    match ty {
        Type::Limited { name, .. } => precedence(name),
        Type::Function { .. } => 0,
        Type::Either(_) => 1,
        Type::Both(_) => 2,
        Type::Tuple(_) => 3,
        Type::Not(_) => 4,
        _ => 5,
    }
}

fn write(
    ty: &Type,
    f: &mut fmt::Formatter<'_>,
    names: &[(Var, String)],
    clashes: &[String],
) -> fmt::Result {
    match ty {
        Type::Unit => f.write_str("()"),
        Type::Number(n) => write!(f, "{n}"),
        Type::Sum(parts) => {
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    f.write_str(" + ")?;
                }
                write(part, f, names, clashes)?;
            }
            Ok(())
        }
        Type::Var(var) => match names.iter().find(|(v, _)| v == var) {
            Some((_, name)) => f.write_str(name),
            // Every variable is named before writing.
            None => f.write_str("_"),
        },
        Type::Rigid(name) => f.write_str(name),
        Type::Named { name, arguments } => {
            if clashes.contains(&name.name) {
                match &name.origin {
                    Origin::File(module) | Origin::Host(module) => {
                        write!(f, "{module}.")?;
                    }
                    Origin::Prelude => f.write_str("prelude.")?,
                    Origin::Commands => f.write_str("commands.")?,
                    Origin::This => {}
                    Origin::Line(index) => {
                        write!(f, "session[{}].", index + 1)?;
                    }
                }
            }
            f.write_str(&name.name)?;
            if arguments.is_empty() {
                return Ok(());
            }
            f.write_str("<")?;
            for (index, argument) in arguments.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write(argument, f, names, clashes)?;
            }
            f.write_str(">")
        }
        Type::Not(inner) => {
            f.write_str("!")?;
            write_grouped(inner, precedence(inner) < 4, f, names, clashes)
        }
        Type::Tuple(parts) => {
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    f.write_str(" * ")?;
                }
                write_grouped(part, precedence(part) <= 3, f, names, clashes)?;
            }
            Ok(())
        }
        Type::Either(parts) | Type::Both(parts) => {
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    f.write_str(if matches!(ty, Type::Both(_)) {
                        " & "
                    } else {
                        " | "
                    })?;
                }
                write_grouped(
                    p,
                    precedence(p) < precedence(ty),
                    f,
                    names,
                    clashes,
                )?;
            }
            Ok(())
        }
        Type::Limited { name, .. } => write(name, f, names, clashes),
        Type::Array { element, length } => {
            f.write_str("[")?;
            write(element, f, names, clashes)?;
            f.write_str("; ")?;
            write(length, f, names, clashes)?;
            f.write_str("]")
        }
        Type::Function { parameter, result } => {
            write_grouped(
                parameter,
                precedence(parameter) == 0,
                f,
                names,
                clashes,
            )?;
            f.write_str(" -> ")?;
            write(result, f, names, clashes)
        }
    }
}

impl Subst {
    pub(crate) fn too_deep(&self, ty: &Type) -> bool {
        let mut stack = alloc::vec![(ty, 0)];
        while let Some((ty, depth)) = stack.pop() {
            if depth > crate::parse::MAX_NESTING {
                return true;
            }
            match ty {
                Type::Var(var) => {
                    if let Some(bound) = self.get(*var) {
                        stack.push((bound, depth));
                    }
                }
                Type::Named { arguments, .. }
                | Type::Sum(arguments)
                | Type::Tuple(arguments)
                | Type::Either(arguments)
                | Type::Both(arguments) => {
                    stack.extend(arguments.iter().map(|ty| (ty, depth + 1)));
                }
                Type::Function { parameter, result } => {
                    stack.push((parameter, depth + 1));
                    stack.push((result, depth + 1));
                }
                Type::Not(inner) => stack.push((inner, depth + 1)),
                Type::Limited { name, meaning } => {
                    stack.push((name, depth + 1));
                    stack.push((meaning, depth + 1));
                }
                Type::Array { element, length } => {
                    stack.push((element, depth + 1));
                    stack.push((length, depth + 1));
                }
                Type::Unit | Type::Number(_) | Type::Rigid(_) => {}
            }
        }
        false
    }
}

fn number_order(a: &Type, b: &Type) -> core::cmp::Ordering {
    match (a, b) {
        (Type::Var(a), Type::Var(b)) => a.cmp(b),
        (Type::Rigid(a), Type::Rigid(b)) => a.cmp(b),
        (Type::Var(_), _) => core::cmp::Ordering::Less,
        (_, Type::Var(_)) => core::cmp::Ordering::Greater,
        _ => core::cmp::Ordering::Equal,
    }
}

fn write_grouped(
    ty: &Type,
    grouped: bool,
    f: &mut fmt::Formatter<'_>,
    names: &[(Var, String)],
    clashes: &[String],
) -> fmt::Result {
    if grouped {
        f.write_str("(")?;
    }
    write(ty, f, names, clashes)?;
    if grouped {
        f.write_str(")")?;
    }
    Ok(())
}
