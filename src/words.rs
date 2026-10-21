//! The words a reader reads for each diagnostic reason and Fault, one
//! `Display` per reason type. A location belongs to whoever renders the
//! source: a `Duplicate` says what is duplicated, not where the first
//! one stands.

use crate::{
    check::Evaluated, decl, infer, ir, parse::Expected, resolve, types::Type,
    value, vm,
};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expression => f.write_str("an expression"),
            Self::Pattern => f.write_str("a pattern"),
            Self::Type => f.write_str("a type"),
            Self::Name => f.write_str("a name"),
            Self::UnqualifiedName => f.write_str("a name without dots"),
            Self::NonnegativeInteger => f.write_str("a non-negative integer"),
            Self::LineEnd => f.write_str("the end of the line"),
            Self::Symbol(symbol) => symbol.fmt(f),
        }
    }
}

impl fmt::Display for resolve::Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultScope => f.write_str(
                "a default may name earlier spots and module-level names, \
                 not an enclosing function’s locals",
            ),
            Self::SelfDefault => {
                f.write_str("a default may not name the binding it belongs to")
            }
            Self::UnknownName => f.write_str("unknown name"),
            Self::NoGate { name } => write!(
                f,
                "`{name}` has no test, so it is no gate: annotate the \
                 place, `x: {name} = …`, or write `to<{name}>`"
            ),
            Self::UnknownType => f.write_str("unknown type"),
            Self::ImportCycle => f.write_str("import cycle"),
            Self::NoMain => f.write_str("a used module has no main"),
            Self::CaseRule { uppercase: true } => {
                f.write_str("a type or constructor name starts uppercase")
            }
            Self::CaseRule { uppercase: false } => f.write_str(
                "a binding, field or parameter name starts lowercase",
            ),
            Self::Duplicate { .. } => f.write_str("already defined"),
            Self::DeclarationForm => {
                f.write_str("a declaration form, allowed only in host files")
            }
            Self::SelfOutsideBody => {
                f.write_str("`Self` outside a type or impl body")
            }
            Self::UnknownModule => f.write_str("unknown module"),
            Self::NotExported => f.write_str("the module exports no such name"),
            Self::UnknownMember => f.write_str("no such member"),
            Self::LimitedImpl => f.write_str("a limited type has no impl"),
            Self::ImplOutsideModule => {
                f.write_str("impl of a type from another module")
            }
            Self::UnknownField => {
                f.write_str("the type declares no such field")
            }
            Self::PrivateField { field } => {
                write!(f, "field `{field}` is private")
            }
            Self::PrivateMember { member } => {
                write!(f, "member `{member}` is private")
            }
            Self::NotYetChecked { form } => {
                write!(f, "{form} is not yet checked")
            }
            Self::TimingForm => f.write_str(
                "a timing form takes exactly one operand; its name is not \
                 a value or pattern",
            ),
            Self::NumericName { upper } => {
                write!(f, "numeric types are uppercase: write `{upper}`")
            }
        }
    }
}

impl fmt::Display for decl::SpotProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(name) => {
                write!(f, "spot `{name}` is declared twice")
            }
            Self::AmbiguousFlag(letter) => {
                write!(f, "more than one Bool spot starts with `{letter}`")
            }
            Self::SelfNotLast => {
                f.write_str("a supplied member must take `self` last")
            }
        }
    }
}

impl fmt::Display for decl::Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(reason) => reason.fmt(f),
            Self::ClaimBlock { problem } => {
                write!(f, "invalid claim block: {problem}")
            }
            Self::Spots { problem } => problem.fmt(f),
            Self::UnknownDocSpot { name } => {
                write!(f, "doc names unknown spot `{name}`")
            }
            Self::NotDeclaration => {
                f.write_str("not a declaration or `name = value`")
            }
            Self::Duplicate { .. } => f.write_str("already declared"),
            Self::NotYetChecked { form } => {
                write!(f, "{form} is not yet checked")
            }
        }
    }
}

/// Whether a type's meaning is `Integer`, through its limits.
fn integer_meaning(ty: &Type) -> bool {
    match ty {
        Type::Limited { meaning, .. } => integer_meaning(meaning),
        Type::Named { name, .. } => {
            name.origin == resolve::Origin::Prelude && name.name == "Integer"
        }
        _ => false,
    }
}

/// Whether a type is a variable, so nothing names its meaning yet.
const fn unknown(ty: &Type) -> bool {
    matches!(ty, Type::Var(_) | Type::Rigid(_))
}

/// A conversion failure: the target, the source when known, and the
/// candidates written beside each other and both sides, so names of
/// different declarations show their modules.
fn conversion(
    f: &mut fmt::Formatter<'_>,
    source: Option<&Type>,
    target: &Type,
    candidates: &[Type],
) -> fmt::Result {
    if unknown(target) {
        return f.write_str("conversion needs a known target meaning");
    }
    let choices = if candidates.is_empty() {
        String::new()
    } else {
        let context = Type::Tuple(
            candidates
                .iter()
                .chain(source)
                .chain([target])
                .cloned()
                .collect(),
        );
        let choices = candidates
            .iter()
            .map(|candidate| candidate.beside(&context).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("; candidates: {choices}")
    };
    match source {
        Some(source) if !unknown(source) => write!(
            f,
            "no conversion or member of {target} accepts {source}{choices}"
        ),
        _ => write!(f, "{target} needs a known argument meaning{choices}"),
    }
}

impl fmt::Display for infer::Reason {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch {
                expected,
                actual: Type::Array { .. },
                ..
            } if matches!(expected, Type::Named { name, .. }
                if name.origin == resolve::Origin::Prelude
                    && name.name == "Vec") =>
            {
                write!(
                    f,
                    "a bound array cannot enter {expected}; use to<{expected}>"
                )
            }
            Self::Mismatch {
                expected, actual, ..
            } => write!(
                f,
                "expected {}, found {}",
                expected.beside(actual),
                actual.beside(expected)
            ),
            Self::CallReading { style } => match style {
                crate::ast::CallStyle::Space => f.write_str(
                    "written as one tuple argument; the argument-list \
                     reading binds",
                ),
                crate::ast::CallStyle::Parenthesized => f.write_str(
                    "written as an argument list; the tuple reading binds",
                ),
            },
            Self::TypeDepth => {
                f.write_str("inferred type nesting limit exceeded")
            }
            Self::Occurs => f.write_str("a type that would contain itself"),
            Self::NothingPayload => f.write_str("Nothing has no payload"),
            Self::OutcomeText { family } => {
                write!(f, "{family} cannot enter a text place; use || or |:")
            }
            Self::UnknownFamily => {
                f.write_str("no outcome family is known here")
            }
            Self::ClaimBlock { claim, problem } => {
                write!(f, "claim {claim} {problem}")
            }
            Self::MissingClaim { ty, claim } => {
                write!(f, "{ty} does not have a ready claim for {claim}")
            }
            Self::MissingMember { ty, name } => {
                write!(f, "missing member {ty}.{name}")
            }
            Self::NoField { ty, .. } => {
                write!(f, "no such field or member on {ty}")
            }
            Self::Conversion {
                source,
                target,
                candidates,
            } => conversion(f, source.as_ref(), target, candidates),
            Self::ConflictingConversion => f.write_str(
                "a conversion for this source meaning is already declared",
            ),
            Self::NoCommonMeaning { forms } => write!(
                f,
                "no meaning reads all literal forms: {}",
                forms.join(", ")
            ),
            Self::NotAFunction => f.write_str("not a function"),
            Self::ExtraArgument => f.write_str("one argument too many"),
            Self::IfWithoutElse => f.write_str(
                "an `if` without `else` is `()`, so its body may have no \
                 value: add an `else`",
            ),
            Self::LiteralApplied => f.write_str("a literal takes no operands"),
            Self::PositionalBool => f.write_str(
                "a Bool spot takes no positional operand: pipe the value \
                 in, `value | f`, or write the spot's flag",
            ),
            Self::UnknownSpot { name } => write!(f, "no spot named `{name}`"),
            Self::FlagOperand { name } => {
                write!(f, "flag `{name}` needs an operand: `-{name} value`")
            }
            Self::DuplicateSupply { name } => {
                write!(f, "duplicate supply: spot `{name}` is already filled")
            }
            Self::Spots { problem } => problem.fmt(f),
            Self::DeclarationValue { name, .. } => {
                write!(f, "declared value `{name}` does not type check")
            }
            Self::Default { name, cause } => match cause {
                Some(cause) => write!(
                    f,
                    "default for spot `{name}` does not meet its type: {cause}"
                ),
                None => write!(
                    f,
                    "default for spot `{name}` does not meet its type: a \
                     name does not resolve"
                ),
            },
            Self::NonBoolChaining => {
                f.write_str("a chained flag needs only Bool spots and no value")
            }
            Self::FilledSpot => f.write_str("no open spot to fill"),
            Self::OpenSpot => {
                f.write_str("`|!` on Maybe needs a value, not an open spot")
            }
            Self::NeedsMaybeOrResult => f.write_str(
                "`||` and `&&` need Bool, Maybe or Result on their left",
            ),
            Self::NeedsOutcome => f.write_str(
                "`|:` and `|!` need Maybe, Result or Either on their left",
            ),
            Self::NotYetChecked { form } => {
                write!(f, "not yet checked: {form}")
            }
            Self::Discarded => f.write_str("a line before the last must be ()"),
            Self::MissingField { name } => {
                write!(f, "missing field `{name}`")
            }
            Self::Limit { message } => f.write_str(message),
            Self::OutOfRange => {
                f.write_str("integer out of range for its type")
            }
            Self::NotExact { ty } if integer_meaning(ty) => f.write_str(
                "not an integer: a literal at an integer place must be exact",
            ),
            Self::NotExact { ty } => {
                write!(f, "not exact: {ty} cannot hold this literal exactly")
            }
            Self::NoBracketForm { ty } => write!(
                f,
                "`[:…]` and `[!…]` need a Maybe or Result place; {ty} has \
                 no bracket form"
            ),
            Self::FamilyClash { left, right } => {
                write!(f, "`||` mixes two outcome families: {left} and {right}")
            }
            Self::NoPayload { name } => write!(f, "`{name}` takes no payload"),
            Self::MissingPayload { good, ty } => write!(
                f,
                "`[{}]` needs a payload of {ty}",
                if *good { ':' } else { '!' }
            ),
            Self::Ambiguous { class } => {
                write!(f, "ambiguous: `{class}` on an unknown type")
            }
            Self::ImportCycle => f.write_str("import cycle"),
            Self::PrivateMember { member } => {
                write!(f, "member `{member}` is private")
            }
            Self::PrivateField { field } => {
                write!(f, "field `{field}` is private")
            }
            Self::NoConstructor { field } => write!(
                f,
                "no constructor outside the module: private field \
                 `{field}` has no default"
            ),
        }
    }
}

impl fmt::Display for vm::InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "host input enters a last line that is a function of one open \
             spot; ",
        )?;
        match self.spots {
            0 => f.write_str("this one is not a function"),
            spots => write!(f, "this one has {spots}"),
        }
    }
}

impl value::Reason {
    /// The reason in words, with what only the run's tables know: a
    /// refuted entry names its limit, and a refuted divisor test reads
    /// as a division by zero. Other reasons read as their `Display`.
    #[must_use]
    pub fn describe(&self, tables: &ir::Tables) -> String {
        match self {
            Self::Failed { entry, value } => {
                let entry = &tables.entries[*entry];
                if matches!(entry.predicate, ir::Predicate::Nonzero { .. }) {
                    format!("division by zero: divisor is {value}")
                } else {
                    format!("refuted entry against {}: {value}", entry.label)
                }
            }
            other => other.to_string(),
        }
    }
}

impl Evaluated {
    /// What went wrong in a compile-time evaluation that refuted its test
    /// or Faulted with `reason`, in words.
    #[must_use]
    pub fn describe(&self, reason: &value::Reason) -> String {
        match (self, reason) {
            (
                Self::Entry {
                    limit,
                    value: Some(value),
                },
                value::Reason::Failed { .. },
            ) => format!("refuted entry against {limit}: {value}"),
            (
                Self::Entry {
                    limit,
                    value: Some(value),
                },
                reason,
            ) => format!(
                "refuted entry against {limit}: the test Faults on {value}: \
                 {reason}"
            ),
            (Self::Entry { limit, value: None }, value::Reason::Budget) => {
                format!(
                    "the compile-time test of {limit} is not settled: the \
                     compile budget is exhausted"
                )
            }
            (Self::Entry { limit, value: None }, reason) => format!(
                "the value entering {limit} Faults at compile time: {reason}"
            ),
            (Self::Literal { meaning }, reason) => {
                format!("the literal Faults reading {meaning}: {reason}")
            }
            (Self::Demand, value::Reason::Budget) => String::from(
                "the value demanded at compile time is not ready: the \
                 compile budget is exhausted",
            ),
            (Self::Demand, reason) => {
                format!("the value demanded at compile time Faults: {reason}")
            }
            (Self::Sequence { meaning }, reason) => format!(
                "the sequence of {meaning} Faults on this bracket: {reason}"
            ),
        }
    }
}
