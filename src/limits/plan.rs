use super::known::Known;
use crate::{resolve::Origin, token::Span};
use alloc::string::String;

/// When an expression is ready or an open test runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Compile,
    Init,
    #[default]
    Eval,
}
/// Why the static rules could not settle a test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    UnknownInput,
    NoRule,
    CompileTimeEvaluation,
    Narrowing,
}
/// How often an entry occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Once,
    PerCall,
    PerElement,
}
/// The destination of a checked value, independent of its source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Binding(String),
    Parameter(Option<String>),
    Result,
    Field { owner: String, name: String },
    Element,
    Part(usize),
    Payload,
}
/// One open entry, with source identity for each span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub place: Span,
    pub place_kind: Place,
    pub limit: Span,
    pub phase: Phase,
    pub reason: Reason,
    pub frequency: Frequency,
    pub place_origin: Origin,
    pub limit_origin: Origin,
    pub name: String,
    /// Number of written tests on the owning type, excluding its bases.
    /// Renderers can distinguish multiple tests using `limit`'s source text.
    pub test_count: usize,
    pub known: Known,
    pub gate: bool,
}

impl core::fmt::Display for Frequency {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Once => "once",
            Self::PerCall => "per call",
            Self::PerElement => "per element",
        })
    }
}
impl core::fmt::Display for Reason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnknownInput => "unknown input",
            Self::NoRule => "no rule",
            Self::CompileTimeEvaluation => "compile-time evaluation",
            Self::Narrowing => "narrowing",
        })
    }
}

impl core::fmt::Display for Phase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Compile => "compile",
            Self::Init => "init",
            Self::Eval => "eval",
        })
    }
}
