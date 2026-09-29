//! A session: a sequence of lines, each checked against the lines
//! accepted before it.
//!
//! An accepted line is frozen in the session's frame; a later definition
//! of a name shadows it for the lines after, never changing it.

use crate::{
    ast::{Entry, ItemKind, Module},
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    frame::Frame,
    infer::{self, infer_in, infer_partial},
    lex::{lex, Lexed},
    parse::{self, parse},
    resolve::{self, resolve_in, resolve_used, Resolved},
    token::Span,
    typed::Typed,
};
use alloc::{string::String, vec::Vec};

/// Accepted lines are joined by a blank line in the session's source,
/// the text their spans are in.
const SEPARATOR: &str = "\n\n";

/// The prelude, the host's declarations, the accepted lines and the file
/// modules loaded through [`Request::File`].
#[derive(Clone, Debug)]
pub struct Session {
    declarations: Declarations,
    source: String,
    /// Each accepted line's text and where it starts in `source`.
    lines: Vec<(String, usize)>,
    frame: Frame,
    files: Vec<(String, Module)>,
    pending: Option<Pending>,
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
}

impl Report {
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

/// A located diagnostic.
///
/// Its span is in the line's text, except for a file module's, which is
/// in that file's text; a span inside a reason, such as a duplicate's
/// first site, stays in the text it was found in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    /// A lexical or syntax error.
    Syntax(parse::Error),
    Resolve(resolve::Error),
    Type(infer::Error),
    /// A replied file module that does not parse, at a span of its text.
    File {
        stem: String,
        error: parse::Error,
    },
    /// A resolve error of a file module the line loaded.
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
                impls: alloc::collections::BTreeMap::new(),
                members: alloc::collections::BTreeMap::new(),
                constructors: alloc::collections::BTreeMap::new(),
            },
            files: Vec::new(),
            pending: None,
        }
    }

    /// The accepted lines, each followed by a blank line: the text
    /// before a new line's [`Report::offset`].
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Remove the last accepted line and what it defined, and return its
    /// text. A line awaiting a reply is abandoned.
    pub fn retract(&mut self) -> Option<String> {
        self.pending = None;
        let (text, offset) = self.lines.pop()?;
        self.frame.retract();
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
                let diagnostics = errors
                    .into_iter()
                    .map(|error| {
                        Diagnostic::Syntax(parse::Error {
                            span: local(error.span, offset),
                            ..error
                        })
                    })
                    .collect();
                return Outcome::Done(Report {
                    module: None,
                    offset,
                    diagnostics,
                    resolved: None,
                    typed: None,
                });
            }
        };
        if let Some(stem) = self.missing(&module, &pending.absent) {
            pending.waiting = Some(stem.clone());
            self.pending = Some(pending);
            return Outcome::Request(Request::File { stem });
        }
        let mut diagnostics = pending.errors;
        for stem in &pending.loaded {
            diagnostics.extend(self.check_file(stem));
        }
        let files = diagnostics.len();
        let (resolved, errors) = resolve_in(
            &module,
            &self.declarations,
            &self.files,
            Some(&self.frame),
        );
        let resolved = if errors.is_empty() {
            Some(resolved)
        } else {
            diagnostics.extend(errors.into_iter().map(|error| {
                Diagnostic::Resolve(resolve::Error {
                    span: local(error.span, offset),
                    ..error
                })
            }));
            None
        };
        let typed = resolved.as_ref().and_then(|resolved| {
            let query = if pending.accept { usize::MAX } else { offset };
            let (typed, errors) = infer_in(
                &module,
                resolved,
                &self.declarations,
                &self.files,
                &self.frame,
                query,
            );
            if errors.is_empty() {
                return Some(typed);
            }
            diagnostics.extend(errors.into_iter().map(|error| {
                Diagnostic::Type(infer::Error {
                    span: local(error.span, offset),
                    ..error
                })
            }));
            None
        });
        if pending.accept && diagnostics.len() == files {
            if let (Some(resolved), Some(typed)) = (&resolved, &typed) {
                self.frame.accept(module.clone(), resolved.clone(), typed);
                self.lines.push((pending.text.clone(), offset));
                self.source.push_str(&pending.text);
                self.source.push_str(SEPARATOR);
            }
        }
        Outcome::Done(Report {
            module: Some(module),
            offset,
            diagnostics,
            resolved,
            typed,
        })
    }

    /// A loaded file module's own diagnostics. A type error inside an
    /// entry that failed to resolve is a consequence and is dropped.
    fn check_file(&self, stem: &str) -> Vec<Diagnostic> {
        let Some((_, module)) = self.files.iter().find(|(f, _)| f == stem)
        else {
            return Vec::new();
        };
        let (resolved, errors) =
            resolve_used(module, &self.declarations, &self.files, stem);
        let (_, types) =
            infer_partial(module, &resolved, &self.declarations, &self.files);
        let failed: Vec<Span> = module
            .entries
            .iter()
            .map(|entry| entry_span(module, entry))
            .filter(|span| errors.iter().any(|e| within(e.span, *span)))
            .collect();
        let resolve = errors.into_iter().map(|error| Diagnostic::FileResolve {
            stem: String::from(stem),
            error,
        });
        let types = types
            .into_iter()
            .filter(|error| {
                !failed.iter().any(|span| within(error.span, *span))
            })
            .map(|error| Diagnostic::FileType {
                stem: String::from(stem),
                error,
            });
        resolve.chain(types).collect()
    }

    /// The first `use`d module that is neither a host module, nor loaded,
    /// nor answered absent for this line; the loaded files' `use` lines
    /// count too.
    fn missing(&self, module: &Module, absent: &[String]) -> Option<String> {
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
            })
            .cloned()
    }
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
const fn within(inner: Span, outer: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// Tokens of a line placed at `offset` in the session's text, so every
/// site in the session is distinct.
fn shifted(mut lexed: Lexed, offset: usize) -> Lexed {
    let shift = |span: &mut Span| {
        span.start += offset;
        span.end += offset;
    };
    for token in &mut lexed.tokens {
        shift(&mut token.span);
    }
    for error in &mut lexed.errors {
        shift(&mut error.span);
    }
    lexed
}

/// A span of the combined text moved into the line's text; one that
/// starts before the line, which only the line can have caused, is
/// placed at its start.
const fn local(span: Span, offset: usize) -> Span {
    Span {
        start: span.start.saturating_sub(offset),
        end: span.end.saturating_sub(offset),
    }
}
