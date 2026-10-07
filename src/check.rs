//! Shared checking diagnostics and policies for files and sessions.
pub(crate) mod lower;

use crate::{
    ast::{Entry, Module},
    decl::Declarations,
    infer, parse, resolve,
    token::Span,
};
use alloc::{string::String, vec::Vec};

/// File checking policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Reject open implicit entries; explicit gates remain allowed.
    pub strict: bool,
}

/// A located diagnostic.
///
/// Its span is in the checked source, except for a file module's, which is
/// in that file's text; a span inside a reason, such as a duplicate's
/// first site, stays in the text it was found in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A lexical or syntax error.
    Syntax(parse::Error),
    Resolve(resolve::Error),
    Type(infer::Error),
    /// A replied file module that does not parse, at a span of its text.
    File {
        stem: String,
        error: parse::Error,
    },
    /// A resolve error of a supplied file module.
    FileResolve {
        stem: String,
        error: resolve::Error,
    },
    /// A type error of a file module the line loaded, outside any of
    /// its entries that failed to resolve.
    FileType {
        stem: String,
        error: infer::Error,
    },
}

pub(crate) fn open_errors(
    checks: &[crate::typed::Check],
    strict: bool,
    offset: usize,
) -> Vec<Error> {
    let mut diagnostics = Vec::new();
    if strict {
        let mut seen = alloc::collections::BTreeSet::new();
        diagnostics.extend(
            checks
                .iter()
                .filter(|check| {
                    !check.gate
                        && check.name != "nonzero divisor"
                        && seen.insert((
                            check.place_origin.clone(),
                            check.place,
                            check.name.clone(),
                        ))
                })
                .map(|check| match &check.place_origin {
                    resolve::Origin::File(stem) => Error::FileType {
                        stem: stem.clone(),
                        error: strict_error(check, 0),
                    },
                    _ => Error::Type(strict_error(check, offset)),
                }),
        );
    }
    diagnostics
}

/// A loaded file module's own diagnostics. A type error inside an
/// entry that failed to resolve is a consequence and is dropped.
pub(crate) fn file_errors(
    stem: &str,
    declarations: &Declarations,
    files: &[(String, Module)],
    cache: &infer::files::FileCache,
) -> Vec<Error> {
    let Some((_, module)) = files.iter().find(|(f, _)| f == stem) else {
        return Vec::new();
    };
    let Some(checked) =
        infer::files::check_file(stem, declarations, files, cache, &[])
    else {
        return Vec::new();
    };
    let errors = checked.resolve_errors;
    let types = checked.type_errors;
    let failed: Vec<Span> = module
        .entries
        .iter()
        .map(|entry| entry_span(module, entry))
        .filter(|span| errors.iter().any(|e| within(e.span, *span)))
        .collect();
    let resolve = errors.into_iter().map(|error| Error::FileResolve {
        stem: String::from(stem),
        error,
    });
    let types = types
        .into_iter()
        .filter(|error| !failed.iter().any(|span| within(error.span, *span)))
        .map(|error| Error::FileType {
            stem: String::from(stem),
            error,
        });
    let mut diagnostics: Vec<_> = resolve.chain(types).collect();
    diagnostics.sort_by_key(|d| match d {
        Error::FileResolve { error, .. } => error.span.start,
        Error::FileType { error, .. } => error.span.start,
        _ => 0,
    });
    diagnostics
}

fn entry_span(module: &Module, entry: &Entry) -> Span {
    match entry {
        Entry::Item(item) => item.span,
        Entry::Line(id) => {
            module.expr(*id).map_or_else(Span::default, |e| e.span)
        }
    }
}

/// Whether `inner` lies inside `outer`.
pub(crate) const fn within(inner: Span, outer: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// A span of the combined text moved into the line's text; one that
/// starts before the line, which only the line can have caused, is
/// placed at its start.
pub(crate) const fn local(span: Span, offset: usize) -> Span {
    Span {
        start: span.start.saturating_sub(offset),
        end: span.end.saturating_sub(offset),
    }
}

fn strict_error(check: &crate::typed::Check, offset: usize) -> infer::Error {
    infer::Error {
        span: local(check.place, offset),
        reason: infer::Reason::Limit {
            message: alloc::format!(
                "open entry against {} in strict session: {}",
                check.name,
                check.reason
            ),
        },
    }
}

/// Checked source retained for tools; an IR unit is emitted only without errors.
pub(crate) struct Checked {
    pub resolved: Option<resolve::Resolved>,
    pub typed: Option<crate::typed::Typed>,
    pub diagnostics: Vec<Error>,
}

pub(crate) fn stages(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: Option<&crate::frame::Frame>,
    cache: &infer::files::FileCache,
    options: Options,
    (query, offset): (usize, usize),
) -> Checked {
    let (resolved, errors) =
        resolve::resolve_in(module, declarations, files, frame);
    if !errors.is_empty() {
        return Checked {
            resolved: None,
            typed: None,
            diagnostics: errors
                .into_iter()
                .map(|error| {
                    Error::Resolve(resolve::Error {
                        span: local(error.span, offset),
                        ..error
                    })
                })
                .collect(),
        };
    }
    for stem in lower::file_order(module, files) {
        infer::files::check_file(&stem, declarations, files, cache, &[]);
    }
    let (typed, errors) = infer::run(
        module,
        &resolved,
        declarations,
        files,
        frame,
        (&[], cache),
        query,
    );
    let mut diagnostics: Vec<_> = errors
        .into_iter()
        .map(|error| {
            Error::Type(infer::Error {
                span: local(error.span, offset),
                ..error
            })
        })
        .collect();
    let typed = diagnostics.is_empty().then_some(typed);
    if let Some(typed) = &typed {
        diagnostics.extend(open_errors(&typed.checks, options.strict, offset));
    }
    Checked {
        resolved: Some(resolved),
        typed,
        diagnostics,
    }
}

/// Check a parsed file and lower all executable choices to indexed tables.
///
/// # Errors
/// Returns resolution, typing, coverage, or entry-policy diagnostics.
///
/// # Panics
/// Panics only if a checker invariant is broken, a crate defect.
pub fn check(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    options: Options,
) -> Result<crate::ir::Unit, Vec<Error>> {
    let cache = infer::files::FileCache::default();
    let mut checked = stages(
        module,
        declarations,
        files,
        None,
        &cache,
        options,
        (usize::MAX, 0),
    );
    let loaded: Vec<_> = cache.files.borrow().keys().cloned().collect();
    for stem in loaded {
        checked.diagnostics.extend(file_errors(
            &stem,
            declarations,
            files,
            &cache,
        ));
    }
    if !checked.diagnostics.is_empty() {
        return Err(checked.diagnostics);
    }
    Ok(lower::Index::default().emit(
        module,
        checked.resolved.expect("successful resolution"),
        checked.typed.expect("successful typing"),
        declarations,
        files,
        &cache,
    ))
}
