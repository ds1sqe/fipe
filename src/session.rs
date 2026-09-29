//! A session: lines checked one at a time against the lines accepted
//! before them, as if they were one file.

use crate::{
    ast::{Entry, ItemKind, Module},
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    infer::{self, infer},
    lex::lex,
    parse::{self, parse},
    resolve::{self, resolve, Resolved},
    token::Span,
    typed::Typed,
};
use alloc::{string::String, vec::Vec};

/// Accepted lines are joined by a blank line, which ends any
/// continuation, so a new line never extends an accepted one.
const SEPARATOR: &str = "\n\n";

/// The prelude, the host's declarations, the accepted lines and the file
/// modules loaded through [`Request::File`].
#[derive(Clone, Debug)]
pub struct Session {
    declarations: Declarations,
    source: String,
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
}

/// What one line produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The session's source followed by the line, parsed; `None` when
    /// the line does not parse. Its spans are in that combined text.
    pub module: Option<Module>,
    /// Where the line starts in the combined text.
    pub offset: usize,
    /// Every diagnostic of the line; empty exactly when the line checks.
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

/// A located diagnostic. Its span is in the line's text, except for
/// [`Diagnostic::File`]; a span inside a reason, such as a duplicate's
/// first site, stays in the combined text.
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
}

impl Session {
    #[must_use]
    pub const fn new(declarations: Declarations) -> Self {
        Self {
            declarations,
            source: String::new(),
            files: Vec::new(),
            pending: None,
        }
    }

    /// The accepted lines, each followed by a blank line.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Check a line and accept it when it has no diagnostic. A line still
    /// awaiting a reply is abandoned.
    pub fn line(&mut self, text: &str) -> Outcome<Report> {
        self.start(text, true)
    }

    /// Check a line without accepting it.
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
                Ok(module) => self.files.push((stem, module)),
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
        })
    }

    fn step(&mut self, mut pending: Pending) -> Outcome<Report> {
        let offset = self.source.len();
        let mut text = self.source.clone();
        text.push_str(&pending.text);
        let module = match parse(lex(&text)) {
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
        let resolved = match resolve(&module, &self.declarations, &self.files) {
            Ok(resolved) => Some(resolved),
            Err(errors) => {
                diagnostics.extend(errors.into_iter().map(|error| {
                    Diagnostic::Resolve(resolve::Error {
                        span: local(error.span, offset),
                        ..error
                    })
                }));
                None
            }
        };
        let typed = resolved.as_ref().and_then(|resolved| {
            match infer(&module, resolved, &self.declarations, &self.files) {
                Ok(typed) => Some(typed),
                Err(errors) => {
                    diagnostics.extend(errors.into_iter().map(|error| {
                        Diagnostic::Type(infer::Error {
                            span: local(error.span, offset),
                            ..error
                        })
                    }));
                    None
                }
            }
        });
        if pending.accept && diagnostics.is_empty() {
            self.source = text;
            self.source.push_str(SEPARATOR);
        }
        Outcome::Done(Report {
            module: Some(module),
            offset,
            diagnostics,
            resolved,
            typed,
        })
    }

    /// The first `use`d module that is neither a host module, nor loaded,
    /// nor answered absent for this line.
    fn missing(&self, module: &Module, absent: &[String]) -> Option<String> {
        module
            .entries
            .iter()
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

/// A span of the combined text moved into the line's text; one that
/// starts before the line, which only the line can have caused, is
/// placed at its start.
const fn local(span: Span, offset: usize) -> Span {
    Span {
        start: span.start.saturating_sub(offset),
        end: span.end.saturating_sub(offset),
    }
}
