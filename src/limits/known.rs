use crate::ast::BinaryOp as B;
use crate::types::Type;
use crate::{ast::BinaryOp, token::Magnitude};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::{cmp::Ordering, fmt};

/// A signed exact integer, with canonical positive zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integer {
    pub negative: bool,
    pub magnitude: Magnitude,
}

impl Integer {
    #[must_use]
    pub const fn new(negative: bool, magnitude: Magnitude) -> Self {
        Self {
            negative: negative && !magnitude.is_zero(),
            magnitude,
        }
    }
    #[must_use]
    pub fn negated(&self) -> Self {
        Self::new(!self.negative, self.magnitude.clone())
    }
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            Self::new(self.negative, self.magnitude.add(&other.magnitude))
        } else if let Some(m) = self.magnitude.checked_sub(&other.magnitude) {
            Self::new(self.negative, m)
        } else {
            Self::new(
                other.negative,
                other
                    .magnitude
                    .checked_sub(&self.magnitude)
                    .unwrap_or_default(),
            )
        }
    }
    #[must_use]
    pub fn multiply(&self, other: &Self) -> Self {
        Self::new(
            self.negative != other.negative,
            self.magnitude.multiply(&other.magnitude),
        )
    }
    #[must_use]
    pub fn divide(&self, other: &Self) -> Option<Self> {
        Some(Self::new(
            self.negative != other.negative,
            self.magnitude.div_rem(&other.magnitude)?.0,
        ))
    }
}
impl From<u64> for Integer {
    fn from(n: u64) -> Self {
        Self::new(false, n.into())
    }
}
impl Ord for Integer {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => other.magnitude.cmp(&self.magnitude),
            (false, false) => self.magnitude.cmp(&other.magnitude),
        }
    }
}
impl PartialOrd for Integer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        self.magnitude.fmt(f)
    }
}

/// An interval endpoint, including infinities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum End {
    Negative,
    Finite(Integer),
    Positive,
}
impl End {
    fn negated(&self) -> Self {
        match self {
            Self::Negative => Self::Positive,
            Self::Positive => Self::Negative,
            Self::Finite(n) => Self::Finite(n.negated()),
        }
    }
    fn add(&self, rhs: &Self) -> Self {
        match (self, rhs) {
            (Self::Finite(a), Self::Finite(b)) => Self::Finite(a.add(b)),
            (Self::Negative, _) | (_, Self::Negative) => Self::Negative,
            _ => Self::Positive,
        }
    }
    fn multiply(&self, rhs: &Self) -> Self {
        let zero = Self::Finite(0.into());
        if self == &zero || rhs == &zero {
            return zero;
        }
        match (self, rhs) {
            (Self::Finite(a), Self::Finite(b)) => Self::Finite(a.multiply(b)),
            _ if (self < &zero) == (rhs < &zero) => Self::Positive,
            _ => Self::Negative,
        }
    }
    fn divide(&self, rhs: &Self) -> Self {
        let zero = Self::Finite(0.into());
        match (self, rhs) {
            (Self::Finite(a), Self::Finite(b)) => {
                Self::Finite(a.divide(b).unwrap_or_else(|| 0.into()))
            }
            (Self::Finite(_), _) => zero,
            _ if (self < &zero) == (rhs < &zero) => Self::Positive,
            _ => Self::Negative,
        }
    }
}
impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finite(n) => n.fmt(f),
            Self::Negative | Self::Positive => Ok(()),
        }
    }
}

/// An inclusive Integer range; lower greater than upper is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub lo: End,
    pub hi: End,
}
impl Range {
    #[must_use]
    pub fn exact(n: Integer) -> Self {
        Self {
            lo: End::Finite(n.clone()),
            hi: End::Finite(n),
        }
    }
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            lo: End::Negative,
            hi: End::Positive,
        }
    }
    #[must_use]
    pub fn contains_zero(&self) -> bool {
        let z = End::Finite(0.into());
        self.lo <= z && z <= self.hi
    }
    pub fn intersect(&mut self, other: &Self) {
        self.lo = self.lo.clone().max(other.lo.clone());
        self.hi = self.hi.clone().min(other.hi.clone());
    }
    #[must_use]
    pub fn arithmetic(&self, op: BinaryOp, rhs: &Self) -> Self {
        if self.lo > self.hi {
            return self.clone();
        }
        if rhs.lo > rhs.hi {
            return rhs.clone();
        }
        if op == BinaryOp::Remainder && self.lo == self.hi && rhs.lo == rhs.hi {
            if let (End::Finite(a), End::Finite(b)) = (&self.lo, &rhs.lo) {
                if let Some((_, remainder)) = a.magnitude.div_rem(&b.magnitude)
                {
                    return Self::exact(Integer::new(a.negative, remainder));
                }
            }
        }
        match op {
            B::Add => Self {
                lo: self.lo.add(&rhs.lo),
                hi: self.hi.add(&rhs.hi),
            },
            B::Subtract => Self {
                lo: self.lo.add(&rhs.hi.negated()),
                hi: self.hi.add(&rhs.lo.negated()),
            },
            B::Multiply => Self::corners(self, rhs, End::multiply),
            B::Divide => {
                let mut ends = Vec::new();
                let one = End::Finite(1.into());
                if rhs.lo < End::Finite(0.into()) {
                    let part = Self {
                        lo: rhs.lo.clone(),
                        hi: rhs.hi.clone().min(one.negated()),
                    };
                    let r = Self::corners(self, &part, End::divide);
                    ends.extend([r.lo, r.hi]);
                }
                if rhs.hi > End::Finite(0.into()) {
                    let part = Self {
                        lo: rhs.lo.clone().max(one),
                        hi: rhs.hi.clone(),
                    };
                    let r = Self::corners(self, &part, End::divide);
                    ends.extend([r.lo, r.hi]);
                }
                Self::bounds(&ends)
            }
            B::Remainder => {
                let max = rhs
                    .lo
                    .negated()
                    .max(rhs.hi.clone())
                    .add(&End::Finite(Integer::from(1).negated()));
                let zero = End::Finite(0.into());
                Self {
                    lo: if self.lo >= zero {
                        zero.clone()
                    } else {
                        self.lo.clone().max(max.negated())
                    },
                    hi: if self.hi <= zero {
                        zero
                    } else {
                        self.hi.clone().min(max)
                    },
                }
            }
            _ => Self::unknown(),
        }
    }
    fn corners(a: &Self, b: &Self, op: fn(&End, &End) -> End) -> Self {
        Self::bounds(&[
            op(&a.lo, &b.lo),
            op(&a.lo, &b.hi),
            op(&a.hi, &b.lo),
            op(&a.hi, &b.hi),
        ])
    }
    fn bounds(ends: &[End]) -> Self {
        Self {
            lo: ends.iter().min().cloned().unwrap_or(End::Positive),
            hi: ends.iter().max().cloned().unwrap_or(End::Negative),
        }
    }
    #[must_use]
    pub fn compare(&self, op: BinaryOp, rhs: &Self) -> Option<bool> {
        if self.lo > self.hi || rhs.lo > rhs.hi {
            return Some(true);
        }
        let (yes, no) = match op {
            B::Less => (self.hi < rhs.lo, self.lo >= rhs.hi),
            B::LessEqual => (self.hi <= rhs.lo, self.lo > rhs.hi),
            B::Greater => return rhs.compare(B::Less, self),
            B::GreaterEqual => return rhs.compare(B::LessEqual, self),
            B::Equal => (
                self.lo == self.hi && self == rhs,
                self.hi < rhs.lo || rhs.hi < self.lo,
            ),
            B::NotEqual => return self.compare(B::Equal, rhs).map(|b| !b),
            _ => return None,
        };
        if yes {
            Some(true)
        } else if no {
            Some(false)
        } else {
            None
        }
    }
}

impl Range {
    /// The one range of `x` where `x op rhs` holds, given what is
    /// known of `x` (07-checks §What is known 4): `!=` narrows only at
    /// an end of `own`, by an exact `rhs`.
    pub(crate) fn narrowed(
        op: BinaryOp,
        rhs: &Self,
        own: Option<&Self>,
    ) -> Option<Self> {
        let one = End::Finite(1.into());
        Some(match op {
            B::Less => Self {
                lo: End::Negative,
                hi: rhs.hi.add(&one.negated()),
            },
            B::LessEqual => Self {
                lo: End::Negative,
                hi: rhs.hi.clone(),
            },
            B::Greater => Self {
                lo: rhs.lo.add(&one),
                hi: End::Positive,
            },
            B::GreaterEqual => Self {
                lo: rhs.lo.clone(),
                hi: End::Positive,
            },
            B::Equal => rhs.clone(),
            B::NotEqual => {
                let own = own?;
                let n = &rhs.hi;
                if !matches!(n, End::Finite(_))
                    || rhs.lo != *n
                    || (own.lo != *n && own.hi != *n)
                {
                    return None;
                }
                Self {
                    lo: if own.lo == *n {
                        n.add(&one)
                    } else {
                        End::Negative
                    },
                    hi: if own.hi == *n {
                        n.add(&one.negated())
                    } else {
                        End::Positive
                    },
                }
            }
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Fault,
    Dependency,
    /// The value never arrives, as a `fault` call's: a join skips it
    /// (07-checks §What is known 4).
    Never,
}

/// Facts held of a value, independent of its type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub(crate) failure: Option<Failure>,
    pub(crate) meaning: Option<crate::types::TypeName>,
    pub phase: super::Phase,
    pub(crate) pure: bool,
    /// A compile demand awaits preparation by the evaluator.
    pub demanded: bool,
    pub holding: Option<super::Holding>,
    pub range: Option<Range>,
    pub exact: Option<String>,
    pub boolean: Option<bool>,
    pub names: Vec<crate::types::TypeName>,
    pub parts: Vec<(String, Self)>,
    /// What is known of every element alike, when the parts are not
    /// known one by one: the bytes of a text or a bytes literal are `U8`.
    pub element: Option<alloc::boxed::Box<Self>>,
    pub constructor: Option<String>,
    /// What is known of a function value (07-checks §What is known 5).
    pub(crate) function: Option<alloc::boxed::Box<Function>>,
}

/// What is known of a function: its parameters' limits and its result's
/// facts, and, when its result is a type variable, where that variable is
/// fed (07-checks §What is known 5, 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Function {
    /// Each parameter's limits as the type that tests them, `None` when
    /// it has none: a call tests its argument against them (rule 8).
    pub(crate) parameters: Vec<Option<Type>>,
    pub(crate) result: Known,
    /// `None` when the result is no variable of the function's scheme or
    /// something may produce it unseen.
    pub(crate) feeds: Option<Vec<Feed>>,
    /// Each parameter of a function argument that is a variable of the
    /// scheme, and where that variable goes in: a function value passed
    /// there meets what is known at it (07-checks §What is known 8).
    pub(crate) takes: Vec<Take>,
}

/// A function argument's parameter at a scheme variable, and that
/// variable's feeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Take {
    /// The argument's position.
    pub(crate) spot: usize,
    /// The argument's own parameter at the variable.
    pub(crate) parameter: usize,
    pub(crate) feeds: Vec<Feed>,
}

/// A parameter where the result's type variable goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Feed {
    /// The argument at this position is of the variable's type.
    Value(usize),
    /// The argument at this position is a function of this many
    /// parameters returning it.
    Returns(usize, usize),
    /// The argument at this position holds values of the variable's
    /// type as its elements: a sequence, or the receiver of a member
    /// limit that takes them (07-checks §What is known 5).
    Element(usize),
}

impl fmt::Display for Known {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(boolean) = self.boolean {
            writeln!(f, "exact {}", if boolean { "True" } else { "False" })?;
        }
        if let Some(exact) = &self.exact {
            writeln!(f, "exact {exact}")?;
        }
        if let Some(range) =
            self.range.as_ref().filter(|r| **r != Range::unknown())
        {
            match range.lo.cmp(&range.hi) {
                core::cmp::Ordering::Greater => writeln!(f, "range empty")?,
                core::cmp::Ordering::Equal => {
                    writeln!(f, "exact {}", range.lo)?;
                }
                core::cmp::Ordering::Less => {
                    writeln!(f, "range {}..{}", range.lo, range.hi)?;
                }
            }
        }
        if !self.names.is_empty() {
            f.write_str("names ")?;
            for (i, name) in self.names.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(&name.name)?;
            }
            writeln!(f)?;
        }
        if !self.parts.is_empty() {
            f.write_str("parts ")?;
            for (i, (name, value)) in self.parts.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{name}: ")?;
                if let Some(boolean) = value.boolean {
                    f.write_str(if boolean { "True" } else { "False" })?;
                } else if let Some(exact) = &value.exact {
                    f.write_str(exact)?;
                } else if let Some(range) = &value.range {
                    if range.lo == range.hi {
                        write!(f, "{}", range.lo)?;
                    } else {
                        write!(f, "{}..{}", range.lo, range.hi)?;
                    }
                } else {
                    f.write_str("..")?;
                }
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

impl Known {
    /// Transform only type-bearing facts, including nested callable values.
    pub(crate) fn map_types(&mut self, map: &mut impl FnMut(&Type) -> Type) {
        for (_, part) in &mut self.parts {
            part.map_types(map);
        }
        if let Some(element) = &mut self.element {
            element.map_types(map);
        }
        if let Some(function) = &mut self.function {
            for parameter in function.parameters.iter_mut().flatten() {
                *parameter = map(parameter);
            }
            function.result.map_types(map);
        }
    }

    pub(crate) fn has_generic_type_facts(&self) -> bool {
        self.parts
            .iter()
            .any(|(_, part)| part.has_generic_type_facts())
            || self
                .element
                .as_ref()
                .is_some_and(|element| element.has_generic_type_facts())
            || self.function.as_ref().is_some_and(|function| {
                function.parameters.iter().flatten().any(type_has_binder)
                    || function.result.has_generic_type_facts()
            })
    }

    /// What is known after two branches meet (07-checks §What is known
    /// 4, 8): the smallest range that holds both, the names both have,
    /// and of a function, in the value or a part, its parameters met and
    /// its results joined. Nothing else leaves a branch; a dead branch is
    /// skipped.
    #[must_use]
    pub(crate) fn join(&self, other: &Self) -> Self {
        if self.is_dead() {
            return other.branch();
        }
        if other.is_dead() {
            return self.branch();
        }
        Self {
            failure: [self.failure, other.failure]
                .into_iter()
                .flatten()
                .find(|f| *f == Failure::Dependency),
            range: match (&self.range, &other.range) {
                (Some(a), Some(b)) => Some(Range {
                    lo: a.lo.clone().min(b.lo.clone()),
                    hi: a.hi.clone().max(b.hi.clone()),
                }),
                _ => None,
            },
            names: self
                .names
                .iter()
                .filter(|name| other.names.contains(name))
                .cloned()
                .collect(),
            function: match (&self.function, &other.function) {
                (Some(a), Some(b)) => Some(Box::new(a.join(b))),
                // A function not known has no parameter limits and an
                // unknown result.
                (Some(f), None) | (None, Some(f)) => {
                    Some(Box::new(Function::of(f.parameters.clone())))
                }
                (None, None) => None,
            },
            parts: self
                .parts
                .iter()
                .filter_map(|(name, part)| {
                    let (_, other) =
                        other.parts.iter().find(|(n, _)| n == name)?;
                    Some((name.clone(), part.join(other).functions()?))
                })
                .collect(),
            ..Self::default()
        }
    }
    /// The function knowledge of a value and of its parts alone: what
    /// leaves a branch inside a part (rule 8), `None` when there is none.
    fn functions(&self) -> Option<Self> {
        let parts: Vec<_> = self
            .parts
            .iter()
            .filter_map(|(name, part)| Some((name.clone(), part.functions()?)))
            .collect();
        (self.function.is_some() || !parts.is_empty()).then(|| Self {
            function: self.function.clone(),
            parts,
            ..Self::default()
        })
    }
    /// Whether a function this value is or holds has a parameter limit.
    pub(crate) fn carries(&self) -> bool {
        self.function.as_deref().is_some_and(|f| {
            f.parameters.iter().any(Option::is_some) || f.result.carries()
        }) || self.parts.iter().any(|(_, part)| part.carries())
            || self.element.as_deref().is_some_and(Self::carries)
    }
    /// What is known of both a value's sources: the range both hold and
    /// the names of either. A dead side stays dead.
    #[must_use]
    pub(crate) fn meet(mut self, other: &Self) -> Self {
        if self.is_dead() {
            return self;
        }
        if other.is_dead() {
            return other.clone();
        }
        match (&mut self.range, &other.range) {
            (Some(range), Some(other)) => range.intersect(other),
            (range @ None, Some(other)) => *range = Some(other.clone()),
            _ => (),
        }
        for name in &other.names {
            if !self.names.contains(name) {
                self.names.push(name.clone());
            }
        }
        match (&mut self.function, &other.function) {
            (function @ None, Some(other)) => *function = Some(other.clone()),
            (Some(function), Some(other)) => {
                for (mine, theirs) in
                    function.parameters.iter_mut().zip(&other.parameters)
                {
                    *mine = both(mine.take(), theirs.as_ref());
                }
            }
            _ => (),
        }
        self
    }
    /// What a function's result carries to its calls: the value's own
    /// facts, not where or when its body runs.
    #[must_use]
    pub(crate) fn result(&self) -> Self {
        Self {
            failure: self.failure.filter(|f| *f != Failure::Fault),
            phase: super::Phase::default(),
            pure: false,
            demanded: false,
            holding: None,
            ..self.clone()
        }
    }
    /// What of one branch may leave it: its range, its names and the
    /// function knowledge of it and of its parts.
    pub(crate) fn branch(&self) -> Self {
        let functions = self.functions().unwrap_or_default();
        Self {
            failure: self.failure.filter(|f| *f != Failure::Fault),
            range: self.range.clone(),
            names: self.names.clone(),
            ..functions
        }
    }
    pub(crate) fn is_dead(&self) -> bool {
        self.failure == Some(Failure::Never)
            || self.range.as_ref().is_some_and(|r| r.lo > r.hi)
    }
    pub(crate) fn part(&self, name: &str) -> Self {
        if self.failure.is_some() {
            return self.clone();
        }
        self.parts
            .iter()
            .find(|(n, _)| n == name)
            .map_or_else(Self::default, |(_, part)| part.clone())
    }
    pub(crate) fn is_exact(&self) -> bool {
        self.exact.is_some()
            || self.boolean.is_some()
            || self.range.as_ref().is_some_and(|r| r.lo == r.hi)
            || (!self.parts.is_empty()
                && self.parts.iter().any(|(name, _)| name != "len")
                && self.parts.iter().all(|(_, k)| k.is_exact()))
    }
    pub(crate) fn rehome(&mut self, origin: &crate::resolve::Origin) {
        if let Some(name) = &mut self.meaning {
            if name.origin == crate::resolve::Origin::This {
                name.origin = origin.clone();
            }
        }
        for name in &mut self.names {
            if name.origin == crate::resolve::Origin::This {
                name.origin = origin.clone();
            }
        }
        for (_, part) in &mut self.parts {
            part.rehome(origin);
        }
        if let Some(function) = &mut self.function {
            for parameter in function.parameters.iter_mut().flatten() {
                *parameter = parameter.rehome(origin);
            }
            function.result.rehome(origin);
        }
    }
}

fn type_has_binder(ty: &Type) -> bool {
    match ty {
        Type::Var(_) | Type::Rigid(_) => true,
        Type::Named { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Either(arguments)
        | Type::Both(arguments)
        | Type::Sum(arguments) => arguments.iter().any(type_has_binder),
        Type::Array { element, length } => {
            type_has_binder(element) || type_has_binder(length)
        }
        Type::Function { parameter, result } => {
            type_has_binder(parameter) || type_has_binder(result)
        }
        Type::Limited { name, meaning } => {
            type_has_binder(name) || type_has_binder(meaning)
        }
        Type::Not(inner) => type_has_binder(inner),
        Type::Unit | Type::Number(_) => false,
    }
}

impl Function {
    /// A function known by its parameter limits alone.
    pub(crate) fn of(parameters: Vec<Option<Type>>) -> Self {
        Self {
            parameters,
            result: Known::default(),
            feeds: None,
            takes: Vec::new(),
        }
    }
    /// Two branches' functions (07-checks §What is known 8): parameters
    /// meet, results join.
    fn join(&self, other: &Self) -> Self {
        if self == other {
            return self.clone();
        }
        let n = self.parameters.len().min(other.parameters.len());
        let parameters = self.parameters[..n]
            .iter()
            .zip(&other.parameters[..n])
            .map(|(a, b)| both(a.clone(), b.as_ref()))
            .collect();
        Self {
            result: self.after(n).join(&other.after(n)),
            ..Self::of(parameters)
        }
    }
    /// What a call of `n` of its parameters gives: its result, or a
    /// function of the rest.
    pub(crate) fn after(&self, n: usize) -> Known {
        if self.parameters.len() <= n {
            return self.result.clone();
        }
        Known {
            function: Some(Box::new(Self {
                result: self.result.clone(),
                ..Self::of(self.parameters[n..].to_vec())
            })),
            ..Known::default()
        }
    }
}

/// The limits of both parameters: a value passed must meet each.
fn both(a: Option<Type>, b: Option<&Type>) -> Option<Type> {
    match (a, b) {
        (a, None) => a,
        (None, Some(b)) => Some(b.clone()),
        (Some(a), Some(b)) if a == *b => Some(a),
        (Some(a), Some(b)) => Some(Type::Both(alloc::vec![a, b.clone()])),
    }
}

#[cfg(test)]
mod fact_type_mapping_tests {
    use super::*;
    use crate::types::{Type, Var};
    use alloc::vec;

    #[test]
    fn callable_type_mapping_reaches_parts_elements_and_returned_functions() {
        let callable = Known {
            function: Some(Box::new(Function::of(vec![Some(Type::Var(Var(
                7,
            )))]))),
            boolean: Some(true),
            phase: super::super::Phase::Init,
            ..Known::default()
        };
        let mut original = Known {
            parts: vec![(String::from("stored"), callable.clone())],
            element: Some(Box::new(callable.clone())),
            function: Some(Box::new(Function {
                parameters: vec![Some(Type::Var(Var(8)))],
                result: callable,
                feeds: Some(vec![Feed::Value(0)]),
                takes: vec![],
            })),
            exact: Some(String::from("unchanged")),
            ..Known::default()
        };
        let untouched = original.clone();
        original.map_types(&mut |ty| {
            crate::types::replace(ty, &[(Var(7), Type::Unit)])
        });
        for nested in [
            &original.parts[0].1,
            original.element.as_deref().unwrap(),
            &original.function.as_ref().unwrap().result,
        ] {
            assert_eq!(
                nested.function.as_ref().unwrap().parameters,
                [Some(Type::Unit)]
            );
            assert_eq!(nested.boolean, Some(true));
            assert_eq!(nested.phase, super::super::Phase::Init);
        }
        let outer = original.function.as_ref().unwrap();
        assert_eq!(
            outer.parameters,
            [Some(Type::Var(Var(8)))],
            "independent binder stays distinct"
        );
        assert_eq!(outer.feeds, untouched.function.as_ref().unwrap().feeds);
        assert_eq!(outer.takes, untouched.function.as_ref().unwrap().takes);
        assert_eq!(original.exact, untouched.exact);
    }
}
