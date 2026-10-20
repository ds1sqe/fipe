//! Diagnostics in words a reader understands: the crate's typed reasons
//! with this server's wording.

use fipe::check::{Error, Evaluated};
use fipe::{infer, parse, resolve};

/// A variant's name from its debug form, as lowercase words:
/// `ChainedComparison { .. }` is "chained comparison".
fn variant(debug: &str) -> String {
    let name: String = debug
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let mut words = String::new();
    for (index, letter) in name.chars().enumerate() {
        if letter.is_ascii_uppercase() && index > 0 {
            words.push(' ');
        }
        words.push(letter.to_ascii_lowercase());
    }
    words
}

/// A syntax error's reason.
#[must_use]
pub fn syntax(reason: &parse::Reason) -> String {
    match reason {
        parse::Reason::Expected(expected) => {
            format!("expected {}", variant(&format!("{expected:?}")))
        }
        parse::Reason::OperatorInArguments => {
            String::from("group an operator expression used as an argument")
        }
        parse::Reason::ApplicationInOperator => {
            String::from("group an application used as an operand")
        }
        parse::Reason::DepthLimit => {
            String::from("expression too deep: bind part of it to a name first")
        }
        parse::Reason::Lexical(reason) => {
            format!("lexical error: {}", variant(&format!("{reason:?}")))
        }
        other => variant(&format!("{other:?}")),
    }
}

/// A resolution error's reason.
#[must_use]
pub fn resolve(reason: &resolve::Reason) -> String {
    match reason {
        resolve::Reason::UnknownName => String::from("unknown name"),
        resolve::Reason::UnknownType => String::from("unknown type"),
        resolve::Reason::UnknownModule => String::from("unknown module"),
        other => variant(&format!("{other:?}")),
    }
}

/// A type error's reason.
#[must_use]
pub fn infer(reason: &infer::Reason) -> String {
    match reason {
        infer::Reason::Mismatch {
            expected, actual, ..
        } => format!("expected {expected}, found {actual}"),
        infer::Reason::NoField { ty, .. } => {
            format!("no field or member here on {ty}")
        }
        infer::Reason::MissingClaim { ty, claim } => {
            format!("{ty} does not claim {claim}")
        }
        infer::Reason::MissingMember { ty, name } => {
            format!("{ty} has no member {name}")
        }
        infer::Reason::Conversion { target, .. } => {
            format!("no conversion to {target}")
        }
        infer::Reason::NotAFunction => String::from("not a function"),
        infer::Reason::ExtraArgument => String::from("one argument too many"),
        infer::Reason::IfWithoutElse => {
            String::from("an if without else is (): add an else")
        }
        other => variant(&format!("{other:?}")),
    }
}

/// A compile-time evaluation's subject.
fn evaluated(what: &Evaluated) -> String {
    match what {
        Evaluated::Entry {
            limit,
            value: Some(value),
        } => format!("{value} against {limit}"),
        Evaluated::Entry { limit, value: None } => {
            format!("the value tested against {limit}")
        }
        Evaluated::Literal { meaning } => format!("a literal at {meaning}"),
        other => variant(&format!("{other:?}")),
    }
}

/// Where a check error belongs and what it says.
#[derive(Debug, PartialEq, Eq)]
pub enum Placed {
    /// In the document, at a byte span.
    Here((usize, usize), String),
    /// In the file module `stem`, at a byte span of its text.
    File(String, (usize, usize), String),
}

/// A check error, placed.
#[must_use]
pub fn check(error: &Error) -> Placed {
    let span = |span: fipe::token::Span| (span.start, span.end);
    match error {
        Error::Syntax(error) => {
            Placed::Here(span(error.span), syntax(&error.reason))
        }
        Error::Resolve(error) => {
            Placed::Here(span(error.span), resolve(&error.reason))
        }
        Error::Type(error) => {
            Placed::Here(span(error.span), infer(&error.reason))
        }
        Error::File { stem, error } => {
            Placed::File(stem.clone(), span(error.span), syntax(&error.reason))
        }
        Error::FileResolve { stem, error } => {
            Placed::File(stem.clone(), span(error.span), resolve(&error.reason))
        }
        Error::FileType { stem, error } => {
            Placed::File(stem.clone(), span(error.span), infer(&error.reason))
        }
        Error::Compile {
            span: at,
            what,
            reason,
        } => Placed::Here(
            span(*at),
            format!("at compile time, {}: {reason}", evaluated(what)),
        ),
    }
}
