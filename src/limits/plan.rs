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
    /// The compile phase ran out of its budget: the entry stays open.
    CompileBudget,
}
/// How often an entry occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Once,
    PerCall,
    PerElement,
}
/// The destination of a checked value, independent of its source span.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Place {
    Binding(String),
    Parameter(Option<String>),
    Result,
    Field { owner: String, name: String },
    Element,
    Part(usize),
    Payload,
}
/// Whether this check executes here or reports a default's source-owned check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckRole {
    /// The value and its expression identifier belong to this checked source.
    Local,
    /// Caller-attributed diagnostics; the recorded default owns execution.
    CallerReport,
}

/// One open entry, with source identity for each span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub role: CheckRole,
    /// The receiving call in `place_origin` source, when this checks a supply.
    /// Nested projections retain the original call; diagnostic copies may omit it.
    pub receiving_call: Option<Span>,
    /// Expression producing the checked value; separate from its diagnostic place.
    pub value: Option<crate::ast::ExprId>,
    pub path: alloc::vec::Vec<crate::ir::Projection>,
    /// Checked destination of this entry; None denotes the nonzero guard.
    pub target: Option<crate::types::Type>,
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
    /// A generic divisor check runs only for an Integer instance.
    pub when_integer: bool,
    /// The binding that owns this entry, when one exists.
    pub owner: Option<String>,
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
            Self::CompileBudget => "compile budget exhausted",
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
