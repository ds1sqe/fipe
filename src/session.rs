//! A session: a sequence of lines, each checked against the lines
//! accepted before it.
//!
//! An accepted line is frozen in the session's frame; a later definition
//! of a name shadows it for the lines after, never changing it.

use crate::{
    ast::{Entry, ItemKind, Module},
    check::{local, within},
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    frame::Frame,
    infer,
    lex::{lex, Lexed},
    parse::{self, parse},
    resolve::{self, Resolved},
    token::Span,
    typed::Typed,
    types::Subst,
};
use alloc::{string::String, vec::Vec};

/// Accepted lines are joined by a blank line in the session's source,
/// the text their spans are in.
mod docs;

const SEPARATOR: &str = "\n\n";

/// The prelude, the host's declarations, the accepted lines and the file
/// modules loaded through [`Request::File`].
#[derive(Debug)]
pub struct Session {
    declarations: Declarations,
    source: String,
    /// Each accepted line's text and where it starts in `source`.
    lines: Vec<(String, usize)>,
    frame: Frame,
    files: Vec<(String, Module)>,
    absent: Vec<String>,
    pending: Option<Pending>,
    strict: bool,
    index: Option<crate::check::lower::Index>,
    indexed: Vec<(crate::check::lower::Index, Vec<Option<crate::ir::Type>>)>,
}

impl Clone for Session {
    fn clone(&self) -> Self {
        let index = self.index.as_ref().map(crate::check::lower::Index::fork);
        let mut indexed = self.indexed.clone();
        if let Some(index) = &index {
            for (snapshot, _) in &mut indexed {
                snapshot.tables = index.tables.clone();
            }
        }
        Self {
            declarations: self.declarations.clone(),
            source: self.source.clone(),
            lines: self.lines.clone(),
            frame: self.frame.clone(),
            files: self.files.clone(),
            absent: self.absent.clone(),
            pending: self.pending.clone(),
            strict: self.strict,
            index,
            indexed,
        }
    }
}

/// A line waiting for a [`Reply`].
#[derive(Clone, Debug)]
struct Pending {
    text: String,
    accept: bool,
    /// The stem of the outstanding [`Request::File`].
    waiting: Option<String>,
    /// Stems answered as absent or unparsable, for this line only.
    absent: Vec<String>,
    /// Syntax errors of replied files.
    errors: Vec<Diagnostic>,
    /// Stems of the files this line loaded, checked once it resolves.
    loaded: Vec<String>,
}

/// What one line produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Non-fatal diagnostics in the submitted line's source coordinates.
    pub warnings: Vec<crate::typed::Warning>,
    /// The line, parsed; `None` when it does not parse. Its spans are in
    /// the session's source followed by the line.
    pub module: Option<Module>,
    /// Where the line starts in the combined text.
    pub offset: usize,
    /// The diagnostics of the files the line loaded, then the line's
    /// own; the line checks exactly when it has none of its own.
    pub diagnostics: Vec<Diagnostic>,
    /// Present when the combined module resolved.
    pub resolved: Option<Resolved>,
    /// Present when the combined module resolved and typed.
    pub typed: Option<Typed>,
    /// Open entries left by this line.
    pub checks: Vec<crate::typed::Check>,
    /// Indexed backend input, present only for a diagnostic-free line.
    pub unit: Option<crate::ir::Unit>,
}

impl Report {
    fn syntax(errors: Vec<parse::Error>, offset: usize) -> Self {
        let diagnostics = errors
            .into_iter()
            .map(|error| {
                Diagnostic::Syntax(parse::Error {
                    span: local(error.span, offset),
                    ..error
                })
            })
            .collect();
        Self {
            warnings: Vec::new(),
            module: None,
            offset,
            diagnostics,
            resolved: None,
            typed: None,
            checks: Vec::new(),
            unit: None,
        }
    }

    /// The line's own entries of [`Report::module`].
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        let Some(module) = &self.module else {
            return &[];
        };
        let first = module.entries.partition_point(|entry| {
            let start = match entry {
                Entry::Item(item) => item.span.start,
                Entry::Line(id) => module.expr(*id).map_or(0, |e| e.span.start),
            };
            start < self.offset
        });
        &module.entries[first..]
    }
}

pub use crate::check::Error as Diagnostic;

impl Session {
    #[must_use]
    pub const fn new(declarations: Declarations) -> Self {
        Self {
            declarations,
            source: String::new(),
            lines: Vec::new(),
            frame: Frame {
                lines: Vec::new(),
                values: alloc::collections::BTreeMap::new(),
                types: alloc::collections::BTreeMap::new(),
                locals: alloc::collections::BTreeMap::new(),
                facts: alloc::collections::BTreeMap::new(),
                impls: alloc::collections::BTreeMap::new(),
                members: alloc::collections::BTreeMap::new(),
                constructors: alloc::collections::BTreeMap::new(),
                weak: Subst::from_bindings(Vec::new()),
            },
            files: Vec::new(),
            absent: Vec::new(),
            pending: None,
            strict: false,
            index: None,
            indexed: Vec::new(),
        }
    }

    /// Reject open implicit entries in subsequent units; explicit gates
    /// remain allowed. Accepted lines retain their earlier checks.
    pub const fn strict(&mut self, enabled: bool) {
        self.strict = enabled;
    }

    /// Open checks of accepted lines, or of the latest binding named `name`.
    #[must_use]
    pub fn checks(&self, name: Option<&str>) -> Vec<crate::typed::Check> {
        let local_site = name
            .and_then(|name| self.frame.values.get(name))
            .and_then(|target| {
                if let resolve::Target::Local { site } = target {
                    Some(*site)
                } else {
                    None
                }
            });
        let file_binding = name.and_then(|name| {
            let (head, member) = name.split_once('.')?;
            let resolve::Target::Module(origin @ resolve::Origin::File(stem)) =
                self.frame.values.get(head)?
            else {
                return None;
            };
            let (_, module) = self.files.iter().find(|(n, _)| n == stem)?;
            module.entries.iter().find_map(|entry| {
                let Entry::Line(id) = entry else { return None };
                let expr = module.expr(*id)?;
                let crate::ast::ExprKind::Bind { pattern, .. } = expr.kind
                else {
                    return None;
                };
                let crate::ast::PatternKind::Name(n) =
                    &module.pattern(pattern)?.kind
                else {
                    return None;
                };
                (n.segments.last().is_some_and(|n| n == member))
                    .then_some((origin, expr.span))
            })
        });
        let mut seen = alloc::collections::BTreeSet::new();
        self.frame
            .lines
            .iter()
            .flat_map(|line| {
                let span = local_site.and_then(|site| {
                    line.module.entries.iter().find_map(|entry| {
                        let Entry::Line(id) = entry else { return None };
                        let expr = line.module.expr(*id)?;
                        let crate::ast::ExprKind::Bind { pattern, .. } =
                            expr.kind
                        else {
                            return None;
                        };
                        (line.module.pattern(pattern)?.span == site)
                            .then_some(expr.span)
                    })
                });
                line.typed.checks.iter().filter(move |check| {
                    name.is_none()
                        || (check.place_origin == resolve::Origin::This
                            && span
                                .is_some_and(|span| within(check.place, span)))
                        || file_binding.is_some_and(|(origin, span)| {
                            &check.place_origin == origin
                                && within(check.place, span)
                        })
                })
            })
            .filter(|check| {
                seen.insert((
                    check.place_origin.clone(),
                    check.place,
                    check.place_kind.clone(),
                    check.limit_origin.clone(),
                    check.limit,
                ))
            })
            .cloned()
            .collect()
    }

    /// The accepted lines, each followed by a blank line: the text
    /// before a new line's [`Report::offset`].
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Accepted compilation units in order, preserving embedded blank lines.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|(text, _)| text.as_str())
    }

    /// Remove the last accepted line and what it defined, and return its
    /// text. A line awaiting a reply is abandoned.
    pub fn retract(&mut self) -> Option<String> {
        self.pending = None;
        let (text, offset) = self.lines.pop()?;
        self.frame.retract();
        if let Some((index, meanings)) = self.indexed.pop() {
            index.restore_meanings(&meanings);
            self.index = Some(index);
        }
        self.source.truncate(offset);
        Some(text)
    }

    /// Check a line and accept it when it has no diagnostic. A line still
    /// awaiting a reply is abandoned.
    pub fn line(&mut self, text: &str) -> Outcome<Report> {
        self.start(text, true)
    }

    /// Check a line without accepting it. An expression line is
    /// generalized, not defaulted: its scheme is the line's
    /// [`Typed::binding`].
    pub fn query(&mut self, text: &str) -> Outcome<Report> {
        self.start(text, false)
    }

    /// Answer the last request and resume the line that made it.
    ///
    /// # Panics
    /// If no line awaits a reply.
    pub fn reply(&mut self, reply: Reply) -> Outcome<Report> {
        let mut pending = self.pending.take().expect("no line awaits a reply");
        let stem = pending.waiting.take().expect("no request is pending");
        match reply {
            Reply::File(Some(text)) => match parse(lex(&text)) {
                Ok(module) => {
                    self.absent.retain(|name| *name != stem);
                    self.files.push((stem.clone(), module));
                    pending.loaded.push(stem);
                }
                Err(errors) => {
                    pending.errors.extend(errors.into_iter().map(|error| {
                        Diagnostic::File {
                            stem: stem.clone(),
                            error,
                        }
                    }));
                    pending.absent.push(stem);
                }
            },
            Reply::File(None) => pending.absent.push(stem),
        }
        for stem in &pending.absent {
            if !self.absent.contains(stem) {
                self.absent.push(stem.clone());
            }
        }
        self.step(pending)
    }

    fn start(&mut self, text: &str, accept: bool) -> Outcome<Report> {
        self.pending = None;
        self.step(Pending {
            text: String::from(text),
            accept,
            waiting: None,
            absent: Vec::new(),
            errors: Vec::new(),
            loaded: Vec::new(),
        })
    }

    fn step(&mut self, mut pending: Pending) -> Outcome<Report> {
        let offset = self.source.len();
        let module = match parse(shifted(lex(&pending.text), offset)) {
            Ok(module) => module,
            Err(errors) => {
                return Outcome::Done(Report::syntax(errors, offset))
            }
        };
        if let Some(stem) = self.missing(&module, &pending.absent) {
            pending.waiting = Some(stem.clone());
            self.pending = Some(pending);
            return Outcome::Request(Request::File { stem });
        }
        let cache = infer::files::FileCache::default();
        let mut diagnostics = pending.errors;
        for stem in &pending.loaded {
            diagnostics.extend(crate::check::file_errors(
                stem,
                &self.declarations,
                &self.files,
                &cache,
            ));
        }
        let files = diagnostics.len();
        let checked = crate::check::stages(
            &module,
            &self.declarations,
            &self.files,
            Some(&self.frame),
            &cache,
            crate::check::Options {
                strict: self.strict,
            },
            (if pending.accept { usize::MAX } else { offset }, offset),
        );
        diagnostics.extend(checked.diagnostics);
        let resolved = checked.resolved;
        let typed = checked.typed;
        let checks = typed
            .as_ref()
            .map_or_else(Vec::new, |typed| typed.checks.clone());
        if pending.accept && diagnostics.len() == files {
            let index = self.index.get_or_insert_with(Default::default);
            self.indexed.push((
                index.checkpoint(),
                index.tables.borrow().session_meanings.clone(),
            ));
        }
        let files_valid = cache.files.borrow().values().all(|file| {
            file.resolve_errors.is_empty() && file.type_errors.is_empty()
        });
        let unit = if diagnostics.is_empty() && files_valid {
            if let (Some(resolved), Some(typed)) = (&resolved, &typed) {
                let index = self.index.get_or_insert_with(Default::default);
                let mut query;
                let index = if pending.accept {
                    index
                } else {
                    query = index.fork();
                    &mut query
                };
                index.session(self.frame.lines.len(), typed);
                Some(index.emit(
                    &module,
                    resolved.clone(),
                    typed.clone(),
                    &self.declarations,
                    &self.files,
                    &cache,
                ))
            } else {
                None
            }
        } else {
            None
        };
        if pending.accept && diagnostics.len() == files {
            if let (Some(resolved), Some(typed)) = (&resolved, &typed) {
                self.accept(&pending.text, &module, resolved, typed);
            }
        }
        Outcome::Done(Report {
            warnings: report_warnings(typed.as_ref(), offset),
            module: Some(module),
            offset,
            diagnostics,
            resolved,
            typed,
            checks,
            unit,
        })
    }

    fn accept(
        &mut self,
        text: &str,
        module: &Module,
        resolved: &Resolved,
        typed: &Typed,
    ) {
        self.frame.accept(module.clone(), resolved.clone(), typed);
        self.lines.push((String::from(text), self.source.len()));
        self.source.push_str(text);
        self.source.push_str(SEPARATOR);
    }

    /// The first `use`d module that is neither a host module, nor loaded,
    /// nor answered absent. A use written in this line retries its imports.
    fn missing(&self, module: &Module, absent: &[String]) -> Option<String> {
        let retry = self.imports(module);
        core::iter::once(module)
            .chain(self.files.iter().map(|(_, file)| file))
            .flat_map(|module| &module.entries)
            .filter_map(|entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Use { names, .. } => Some(names),
                    _ => None,
                },
                Entry::Line(_) => None,
            })
            .flatten()
            .filter_map(|name| name.segments.first())
            .find(|stem| {
                self.declarations.get_module(stem).is_none()
                    && !self.files.iter().any(|(file, _)| file == *stem)
                    && !absent.contains(stem)
                    && (!self.absent.contains(stem) || retry.contains(*stem))
            })
            .cloned()
    }
    fn imports(&self, module: &Module) -> alloc::collections::BTreeSet<String> {
        let mut stems = alloc::collections::BTreeSet::new();
        let mut pending = alloc::vec![module];
        while let Some(module) = pending.pop() {
            for entry in &module.entries {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Use { names, .. } = &item.kind else {
                    continue;
                };
                for name in names {
                    let Some(stem) = name.segments.first() else {
                        continue;
                    };
                    if stems.insert(stem.clone()) {
                        if let Some((_, file)) =
                            self.files.iter().find(|(name, _)| name == stem)
                        {
                            pending.push(file);
                        }
                    }
                }
            }
        }
        stems
    }
}

/// Tokens of a line placed at `offset` in the session's text, so every
/// site in the session is distinct.
fn shifted(mut lexed: Lexed, offset: usize) -> Lexed {
    let shift = |span: &mut Span| {
        span.start += offset;
        span.end += offset;
    };
    for comment in &mut lexed.comments {
        shift(comment);
    }
    for token in &mut lexed.tokens {
        shift(&mut token.span);
    }
    for error in &mut lexed.errors {
        shift(&mut error.span);
    }
    lexed
}

fn report_warnings(
    typed: Option<&Typed>,
    offset: usize,
) -> Vec<crate::typed::Warning> {
    typed.map_or_else(Vec::new, |typed| {
        typed
            .warnings
            .iter()
            .map(|warning| crate::typed::Warning {
                span: local(warning.span, offset),
                style: warning.style,
            })
            .collect()
    })
}
