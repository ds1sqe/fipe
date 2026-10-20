//! Shared checking diagnostics and policies for files and sessions.
pub(crate) mod evaluate;
pub(crate) mod lower;

use crate::{
    ast::{Entry, Module},
    decl::Declarations,
    infer, parse, resolve,
    token::Span,
};
use alloc::{string::String, vec::Vec};

/// File checking policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Reject open implicit entries; explicit gates remain allowed.
    pub strict: bool,
    /// The checker's steps each compile-time evaluation may spend, set
    /// by the host (`09-host.md` §Budget); exhausted means open.
    pub compile_budget: u64,
}

/// The compile budget when the host names none (`09-host.md` §Budget).
pub const COMPILE_BUDGET: u64 = 1 << 20;

impl Default for Options {
    fn default() -> Self {
        Self {
            strict: false,
            compile_budget: COMPILE_BUDGET,
        }
    }
}

/// What a compile-time evaluation that Faulted was settling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Evaluated {
    /// An entry against the limit named `limit`; `value` is the value
    /// tested, in the language's spelling, when computing it did not
    /// Fault.
    Entry {
        limit: String,
        value: Option<String>,
    },
    /// A literal read by a reader that runs, a program's `literal` or a
    /// read cast, at the meaning labelled `meaning`.
    Literal { meaning: String },
    /// A value demanded at compile time, by a place or `Compile`.
    Demand,
    /// A bracket of exact elements handed to the `sequence` of the
    /// meaning labelled `meaning`.
    Sequence { meaning: String },
}

/// An entry the compile phase proved, with the value it proved it on in
/// the language's spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settled {
    pub check: crate::typed::Check,
    pub value: String,
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
    /// A compile-time evaluation that refuted its test or Faulted
    /// (`07-checks.md` §What is known 1), at the place it settled.
    Compile {
        span: Span,
        what: Evaluated,
        reason: crate::value::Reason,
    },
}

/// The strict-session errors of open entries: before the compile phase
/// those it does not decide, after it (`phase`) those it left open.
pub(crate) fn open_errors(
    checks: &[crate::typed::Check],
    strict: bool,
    offset: usize,
    phase: bool,
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
                        && phase == compile_phase(check)
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

/// The values a binding's place demands at compile time, by span: the
/// compile phase prepares each.
pub(crate) fn demands(
    module: &Module,
    typed: &crate::typed::Typed,
) -> Vec<Span> {
    module
        .expressions
        .iter()
        .filter_map(|expression| match expression.kind {
            crate::ast::ExprKind::Bind {
                annotation: Some(_),
                value,
                ..
            } if typed.known.get(value.0).is_some_and(|k| k.demanded) => {
                Some(module.expressions[value.0].span)
            }
            _ => None,
        })
        .collect()
}

/// The checks the compile phase decided, applied to a line's or a
/// file's typing: a proved entry leaves its checks and is returned
/// settled; an exhausted one stays open with that reason; a prepared
/// value is exact from then on, in its expression and its binding.
pub(crate) fn settle(
    typed: &mut crate::typed::Typed,
    module: &Module,
    phase: &evaluate::Phase,
) -> Vec<Settled> {
    let same = |check: &crate::typed::Check, entry: &crate::ir::Entry| {
        check.place_origin == resolve::Origin::This
            && check.place == entry.span
            && check.name == entry.label
            && check.path == entry.path
    };
    let mut settled = Vec::new();
    for proved in &phase.proved {
        let mut index = 0;
        while index < typed.checks.len() {
            if same(&typed.checks[index], &proved.entry) {
                settled.push(Settled {
                    check: typed.checks.remove(index),
                    value: proved.value.clone(),
                });
            } else {
                index += 1;
            }
        }
    }
    for entry in &phase.exhausted {
        for check in &mut typed.checks {
            if same(check, entry) {
                check.reason = crate::typed::CheckReason::CompileBudget;
            }
        }
    }
    for (span, known) in &phase.prepared {
        exact_from_now(typed, module, *span, known);
    }
    settled
}

/// What is known of a prepared value, given to the expression that
/// demanded it and to the name a binding gives it.
fn exact_from_now(
    typed: &mut crate::typed::Typed,
    module: &Module,
    span: Span,
    known: &crate::limits::known::Known,
) {
    use crate::ast::{ExprKind, PatternKind};
    let exact = |fact: &mut crate::limits::known::Known| {
        if known.range.is_some() {
            fact.range.clone_from(&known.range);
        }
        if known.boolean.is_some() {
            fact.boolean = known.boolean;
        }
        if known.exact.is_some() {
            fact.exact.clone_from(&known.exact);
        }
    };
    for (index, expression) in module.expressions.iter().enumerate() {
        if expression.span == span {
            if let Some(fact) = typed.known.get_mut(index) {
                exact(fact);
            }
        }
        let ExprKind::Bind { pattern, value, .. } = expression.kind else {
            continue;
        };
        if module.expressions[value.0].span != span {
            continue;
        }
        let PatternKind::Name(name) = &module.patterns[pattern.0].kind else {
            continue;
        };
        let site = (name.span.start, name.span.end);
        for (at, fact) in &mut typed.facts {
            if *at == site {
                exact(fact);
            }
        }
    }
}

/// The entries the compile budget left open whose test is written
/// `Compile (test)`: such a test is settled at compile time, else it is
/// a check error (`07-checks.md` §Modes). `timed` tells whether the
/// test at a limit's span, in its module, is written so.
pub(crate) fn unsettled(
    typed: &crate::typed::Typed,
    timed: impl Fn(&resolve::Origin, Span) -> bool,
    offset: usize,
) -> Vec<Error> {
    typed
        .checks
        .iter()
        .filter(|check| {
            check.reason == crate::typed::CheckReason::CompileBudget
                && timed(&check.limit_origin, check.limit)
        })
        .map(|check| Error::Compile {
            span: local(check.place, offset),
            what: Evaluated::Entry {
                limit: check.name.clone(),
                value: None,
            },
            reason: crate::value::Reason::Budget,
        })
        .collect()
}

/// Whether the expression at `span` is written `Compile (…)`.
pub(crate) fn timed_compile(
    module: &Module,
    resolved: &resolve::Resolved,
    span: Span,
) -> bool {
    module.expressions.iter().any(|expression| {
        expression.span == span
            && matches!(
                &expression.kind,
                crate::ast::ExprKind::Call { callee, .. }
                    if resolve::timing(resolved.expr(*callee))
                        == Some(crate::typed::Phase::Compile)
            )
    })
}

/// Whether the compile phase decides a check.
const fn compile_phase(check: &crate::typed::Check) -> bool {
    matches!(
        check.reason,
        crate::typed::CheckReason::CompileTimeEvaluation
            | crate::typed::CheckReason::CompileBudget
    )
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
        diagnostics.extend(open_errors(
            &typed.checks,
            options.strict,
            offset,
            false,
        ));
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
    // A module the parser could not read whole is no unit: its syntax
    // errors lead, the rest of it is still checked for its diagnostics.
    let syntax = module.errors.iter().cloned().map(Error::Syntax);
    let file_syntax = files.iter().flat_map(|(stem, file)| {
        file.errors.iter().map(|error| Error::File {
            stem: stem.clone(),
            error: error.clone(),
        })
    });
    checked.diagnostics = syntax
        .chain(file_syntax)
        .chain(core::mem::take(&mut checked.diagnostics))
        .collect();
    if !checked.diagnostics.is_empty() {
        return Err(checked.diagnostics);
    }
    let mut typed = checked.typed.expect("successful typing");
    let resolved = checked.resolved.expect("successful resolution");
    let mut unit = lower::Index::default().emit(
        module,
        resolved.clone(),
        typed.clone(),
        declarations,
        files,
        &cache,
    );
    let demands = demands(module, &typed);
    let phase = evaluate::run(&mut unit, 0, options.compile_budget, &demands)?;
    settle(&mut typed, module, &phase);
    let mut errors = unsettled(
        &typed,
        |origin, span| {
            *origin == resolve::Origin::This
                && timed_compile(module, &resolved, span)
        },
        0,
    );
    errors.extend(open_errors(&typed.checks, options.strict, 0, true));
    if errors.is_empty() {
        Ok(unit)
    } else {
        Err(errors)
    }
}
