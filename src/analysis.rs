//! Offline source snapshots and transport-neutral checking diagnostics.
mod diagnostics;

use crate::{
    ast::Module, check, decl::Declarations, resolve, token::Span, typed,
};
use alloc::{string::String, vec::Vec};
use core::fmt;

/// An index in the caller's source-text slice; not a path or wire identity.
pub type SourceId = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub source: SourceId,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Related {
    pub location: Location,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub location: Location,
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub related: Vec<Related>,
}

/// Exact texts corresponding to the already parsed modules and declarations.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub texts: &'a [&'a str],
    pub current: SourceId,
    pub origins: &'a [(resolve::Origin, SourceId)],
    /// The current document's file-module stem when an import reaches it again.
    pub current_file: Option<&'a str>,
}

/// Opt-in input admission bounds; these do not bound inference work or time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub max_source_bytes: Option<usize>,
    pub max_total_bytes: Option<usize>,
    pub max_sources: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub checking: check::Options,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    InvalidSourceMap,
    SourceLimit { source: SourceId },
    TotalLimit,
    SourceCountLimit,
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceMap => {
                formatter.write_str("invalid analysis source map")
            }
            Self::SourceLimit { source } => write!(
                formatter,
                "analysis source {source} exceeds its byte limit"
            ),
            Self::TotalLimit => formatter
                .write_str("analysis sources exceed the total byte limit"),
            Self::SourceCountLimit => {
                formatter.write_str("too many analysis sources")
            }
        }
    }
}

impl core::error::Error for InputError {}

/// Partial stages survive ordinary checking errors; no executable unit is issued.
pub struct Snapshot {
    pub resolved: resolve::Resolved,
    pub typed: typed::Typed,
    pub diagnostics: Vec<Diagnostic>,
}

impl Sources<'_> {
    fn source(&self, origin: &resolve::Origin) -> Result<SourceId, InputError> {
        if *origin == resolve::Origin::This {
            return Ok(self.current);
        }
        if matches!(origin, resolve::Origin::File(stem)
            if self.current_file == Some(stem.as_str()))
        {
            return Ok(self.current);
        }
        self.origins
            .iter()
            .find(|(known, _)| known == origin)
            .map(|(_, source)| *source)
            .ok_or(InputError::InvalidSourceMap)
    }

    fn location(
        &self,
        origin: &resolve::Origin,
        span: Span,
    ) -> Result<Location, InputError> {
        let source = self.source(origin)?;
        let text =
            self.texts.get(source).ok_or(InputError::InvalidSourceMap)?;
        if span.start > span.end || text.get(span.start..span.end).is_none() {
            return Err(InputError::InvalidSourceMap);
        }
        Ok(Location { source, span })
    }

    fn validate(
        &self,
        files: &[(String, Module)],
        limits: Limits,
    ) -> Result<(), InputError> {
        if limits
            .max_sources
            .is_some_and(|limit| self.texts.len() > limit)
        {
            return Err(InputError::SourceCountLimit);
        }
        let mut total = 0usize;
        for (source, text) in self.texts.iter().enumerate() {
            if limits
                .max_source_bytes
                .is_some_and(|limit| text.len() > limit)
            {
                return Err(InputError::SourceLimit { source });
            }
            total = total
                .checked_add(text.len())
                .ok_or(InputError::TotalLimit)?;
        }
        if limits.max_total_bytes.is_some_and(|limit| total > limit) {
            return Err(InputError::TotalLimit);
        }
        if self.current >= self.texts.len() {
            return Err(InputError::InvalidSourceMap);
        }
        for (index, (origin, source)) in self.origins.iter().enumerate() {
            if *source >= self.texts.len()
                || self.origins[..index]
                    .iter()
                    .any(|(known, _)| known == origin)
                || matches!(origin, resolve::Origin::This)
                    && *source != self.current
            {
                return Err(InputError::InvalidSourceMap);
            }
            if matches!(origin, resolve::Origin::File(stem)
                if self.current_file == Some(stem.as_str()))
                && self.texts[*source] != self.texts[self.current]
            {
                return Err(InputError::InvalidSourceMap);
            }
        }
        for (index, (stem, _)) in files.iter().enumerate() {
            if files[..index].iter().any(|(known, _)| known == stem) {
                return Err(InputError::InvalidSourceMap);
            }
            self.source(&resolve::Origin::File(stem.clone()))?;
        }
        Ok(())
    }
}

/// Analyze exact caller-supplied modules without loading files or invoking a Host.
///
/// This retains the existing tooling pipeline: partial resolution/inference for
/// useful stages, followed by one authoritative check. Limits apply to supplied
/// text, not allocations already made by the caller or total checker work.
/// An input error is not an empty successful diagnostic set. Callers must keep
/// revision identities and never present an older snapshot as current.
///
/// # Errors
/// Invalid source mappings, non-byte-boundary locations or input admission limits.
pub fn analyze(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    sources: Sources<'_>,
    options: Options,
) -> Result<Snapshot, InputError> {
    sources.validate(files, options.limits)?;
    sources.location(&resolve::Origin::This, module.span)?;
    let (resolved, _) = resolve::resolve_partial(module, declarations, files);
    let (typed, _) =
        crate::infer::infer_partial(module, &resolved, declarations, files);
    let mut diagnostics = Vec::new();
    if let Err(errors) =
        check::check(module, declarations, files, options.checking)
    {
        for error in &errors {
            let mut diagnostic = diagnostics::checked(error, sources)?;
            diagnostics::imports(&mut diagnostic, module, files, sources)?;
            diagnostics.push(diagnostic);
        }
    }
    for warning in &typed.warnings {
        diagnostics.push(diagnostics::warning(warning, sources)?);
    }
    Ok(Snapshot {
        resolved,
        typed,
        diagnostics,
    })
}
