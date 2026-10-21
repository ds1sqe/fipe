use crate::ast::BinaryOp as B;
use crate::{ast::BinaryOp, token::Magnitude};
use alloc::{string::String, vec::Vec};
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
    pub narrowing: bool,
}

/// What is known of a function: its parameters' and its result's facts,
/// and, when its result is a type variable, where that variable is fed
/// (07-checks §What is known 5, 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Function {
    pub(crate) parameters: Vec<Known>,
    pub(crate) result: Known,
    /// `None` when the result is no variable of the function's scheme or
    /// something may produce it unseen.
    pub(crate) feeds: Option<Vec<Feed>>,
}

/// A parameter where the result's type variable goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Feed {
    /// The argument at this position is of the variable's type.
    Value(usize),
    /// The argument at this position is a function of this many
    /// parameters returning it.
    Returns(usize, usize),
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
    /// What is known after two branches meet (07-checks §What is known
    /// 4): the smallest range that holds both and the names both have.
    /// Nothing else leaves a branch; a dead branch is skipped.
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
            narrowing: self.narrowing || other.narrowing,
            ..Self::default()
        }
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
    /// What of one branch may leave it: its range and names.
    pub(crate) fn branch(&self) -> Self {
        Self {
            failure: self.failure.filter(|f| *f != Failure::Fault),
            range: self.range.clone(),
            names: self.names.clone(),
            narrowing: self.narrowing,
            ..Self::default()
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
        self.parts.iter().find(|(n, _)| n == name).map_or_else(
            || Self {
                narrowing: self.narrowing,
                ..Self::default()
            },
            |(_, part)| part.clone(),
        )
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
            for parameter in &mut function.parameters {
                parameter.rehome(origin);
            }
            function.result.rehome(origin);
        }
    }
}
