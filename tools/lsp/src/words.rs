//! Diagnostics in words a reader understands: the crate's typed reasons
//! with Core Display wording; source snapshots use `fipe::analysis` stable records.

use fipe::check::Error;
use fipe::{infer, parse, resolve};

/// A syntax error's reason.
#[must_use]
pub fn syntax(reason: &parse::Reason) -> String {
    reason.to_string()
}

/// A resolution error's reason; a duplicate's first site is the
/// related location, not part of the words.
#[must_use]
pub fn resolve(reason: &resolve::Reason) -> String {
    reason.to_string()
}

/// A type error's reason.
#[must_use]
pub fn infer(reason: &infer::Reason) -> String {
    reason.to_string()
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
        } => Placed::Here(span(*at), what.describe(reason)),
    }
}
