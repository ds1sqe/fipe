//! The Fipe REPL without I/O: bytes in, bytes and file requests out.
//!
//! A driver passes input to [`Repl::feed`] and acts on each [`Out`] in
//! order: it writes [`Out::Write`], answers [`Out::File`] with
//! [`Repl::file`] and stops at [`Out::Exit`]. Lines end at LF; at end of
//! input the driver calls [`Repl::flush`] so a last line without LF runs.
//! The prompt comes out whenever the input holds no partial line and
//! nothing is awaited, so `feed(&[])` yields the first one; none comes
//! after `flush`.
//!
//! Commands: `:ast`, `:r [line]`, `:t [line]`, `:load <stem>`,
//! `:host [dir]`, `:q`; any other line goes to the session.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use fipe::{
    ast::{Entry, ExprKind},
    decl::{self, Declarations},
    embed::{Outcome, Reply, Request},
    infer::Reason,
    session::{Diagnostic, Report, Session},
    token::{Source, Span},
};

const PROMPT: &[u8] = b"fipe> ";
const HOST_DIR: &str = "tests/fixtures/host";
/// The command namespace first, then the host modules by name.
const HOST: [&str; 5] = ["commands", "fs", "sys", "cli", "text"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Write(Vec<u8>),
    /// The driver reads this path and answers with [`Repl::file`].
    File(String),
    Exit,
}

pub struct Repl {
    session: Session,
    input: Vec<u8>,
    wait: Option<Wait>,
    /// Replied file texts by stem, for replays and file diagnostics.
    files: Vec<(String, String)>,
    /// The last accepted line, for `:ast` and `:r`.
    last: Option<(Report, String)>,
    quit: bool,
    ended: bool,
}

enum Wait {
    Line {
        stem: String,
        text: String,
        mode: Mode,
    },
    Host {
        dir: String,
        texts: Vec<String>,
    },
}

#[derive(Clone, Copy)]
enum Mode {
    Accept,
    Resolve,
    Types,
}

/// Output with adjacent writes merged.
#[derive(Default)]
struct Outs(Vec<Out>);

impl Outs {
    fn write(&mut self, bytes: &[u8]) {
        if let Some(Out::Write(last)) = self.0.last_mut() {
            last.extend_from_slice(bytes);
        } else {
            self.0.push(Out::Write(bytes.to_vec()));
        }
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

impl Repl {
    /// A session over the prelude alone.
    #[must_use]
    pub fn new() -> Self {
        Self {
            session: Session::new(Declarations::new()),
            input: Vec::new(),
            wait: None,
            files: Vec::new(),
            last: None,
            quit: false,
            ended: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Out> {
        self.input.extend_from_slice(bytes);
        let mut out = Outs::default();
        self.run(&mut out);
        out.0
    }

    /// End of input: a trailing line without LF runs as complete.
    pub fn flush(&mut self) -> Vec<Out> {
        self.ended = true;
        if !self.input.is_empty() && !self.input.ends_with(b"\n") {
            self.input.push(b'\n');
        }
        self.feed(&[])
    }

    /// The answer to the last [`Out::File`]: its text, or `None` when
    /// the path does not exist.
    pub fn file(&mut self, path: &str, text: Option<&str>) -> Vec<Out> {
        let mut out = Outs::default();
        match self.wait.take() {
            Some(Wait::Line {
                stem,
                text: line,
                mode,
            }) => {
                if let Some(text) = text {
                    self.files.push((stem, text.to_string()));
                }
                let reply = Reply::File(text.map(String::from));
                let outcome = self.session.reply(reply);
                self.step(line, mode, outcome, &mut out);
            }
            Some(Wait::Host { dir, mut texts }) => match text {
                None => out.write(format!("{path}: not found\n").as_bytes()),
                Some(text) => {
                    texts.push(text.to_string());
                    if let Some(name) = HOST.get(texts.len()) {
                        out.0.push(Out::File(host_path(&dir, name)));
                        self.wait = Some(Wait::Host { dir, texts });
                    } else {
                        self.host(&dir, &texts, &mut out);
                    }
                }
            },
            None => out.write(format!("{path}: not requested\n").as_bytes()),
        }
        self.run(&mut out);
        out.0
    }

    fn run(&mut self, out: &mut Outs) {
        while !self.quit && self.wait.is_none() {
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                break;
            };
            let bytes: Vec<u8> = self.input.drain(..=end).collect();
            let line = String::from_utf8_lossy(&bytes);
            self.command(line.trim_end_matches(['\n', '\r']), out);
        }
        if !self.quit
            && !self.ended
            && self.wait.is_none()
            && self.input.is_empty()
        {
            out.write(PROMPT);
        }
    }

    fn command(&mut self, line: &str, out: &mut Outs) {
        let Some(command) = line.trim().strip_prefix(':') else {
            if !line.trim().is_empty() {
                self.start(line, Mode::Accept, out);
            }
            return;
        };
        let (name, arg) = command
            .split_once(' ')
            .map_or((command, ""), |(name, arg)| (name, arg.trim()));
        match (name, arg) {
            ("q", "") => {
                self.quit = true;
                out.0.push(Out::Exit);
            }
            ("t", "") => match &self.last {
                Some((report, text)) => types(report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("t", arg) => self.start(arg, Mode::Types, out),
            ("ast", "") => self.ast(out),
            ("r", "") => match &self.last {
                Some((report, text)) => names(report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("r", line) => self.start(line, Mode::Resolve, out),
            ("load", stem) if !stem.is_empty() => {
                self.start(&format!("use {stem}"), Mode::Accept, out);
            }
            ("host", dir) => {
                let dir = if dir.is_empty() { HOST_DIR } else { dir };
                out.0.push(Out::File(host_path(dir, HOST[0])));
                self.wait = Some(Wait::Host {
                    dir: dir.to_string(),
                    texts: Vec::new(),
                });
            }
            _ => out.write(format!("unknown command :{command}\n").as_bytes()),
        }
    }

    fn start(&mut self, text: &str, mode: Mode, out: &mut Outs) {
        let outcome = match mode {
            Mode::Accept => self.session.line(text),
            Mode::Resolve | Mode::Types => self.session.query(text),
        };
        self.step(text.to_string(), mode, outcome, out);
    }

    fn step(
        &mut self,
        text: String,
        mode: Mode,
        outcome: Outcome<Report>,
        out: &mut Outs,
    ) {
        match outcome {
            Outcome::Request(Request::File { stem }) => {
                out.0.push(Out::File(format!("{stem}.fipe")));
                self.wait = Some(Wait::Line { stem, text, mode });
            }
            Outcome::Done(report) => self.done(report, text, mode, out),
        }
    }

    fn done(
        &mut self,
        report: Report,
        text: String,
        mode: Mode,
        out: &mut Outs,
    ) {
        for diagnostic in &report.diagnostics {
            self.diagnostic(diagnostic, &text, out);
        }
        match mode {
            Mode::Accept if report.diagnostics.is_empty() => {
                self.last = Some((report, text));
            }
            Mode::Accept => {}
            Mode::Resolve => names(&report, &text, out),
            Mode::Types => types(&report, &text, out),
        }
    }

    fn diagnostic(&self, diagnostic: &Diagnostic, line: &str, out: &mut Outs) {
        match diagnostic {
            Diagnostic::Syntax(error) => {
                located("", line, error.span, &error.reason.to_string(), out);
            }
            Diagnostic::Resolve(error) => {
                let reason = format!("{:?}", error.reason);
                located("", line, error.span, &reason, out);
            }
            Diagnostic::Type(error) => {
                let reason = match &error.reason {
                    Reason::Mismatch {
                        expected, actual, ..
                    } => format!("expected {expected}, found {actual}"),
                    reason => format!("{reason:?}"),
                };
                located("", line, error.span, &reason, out);
            }
            Diagnostic::File { stem, error } => {
                let text = self.cached(stem).unwrap_or_default();
                let path = format!("{stem}.fipe:");
                located(
                    &path,
                    text,
                    error.span,
                    &error.reason.to_string(),
                    out,
                );
            }
        }
    }

    fn cached(&self, stem: &str) -> Option<&str> {
        self.files
            .iter()
            .rev()
            .find(|(cached, _)| cached == stem)
            .map(|(_, text)| text.as_str())
    }

    fn ast(&self, out: &mut Outs) {
        let Some((report, _)) = &self.last else {
            return out.write(b"no accepted line\n");
        };
        let Some(module) = &report.module else { return };
        for entry in report.entries() {
            let text = match entry {
                Entry::Item(item) => format!("{item:#?}\n"),
                Entry::Line(id) => module
                    .expr(*id)
                    .map_or_else(String::new, |expr| format!("{expr:#?}\n")),
            };
            out.write(text.as_bytes());
        }
    }

    /// Rebuild the session over the host's declarations and replay the
    /// accepted lines, answering their `use` requests from the cache.
    fn host(&mut self, dir: &str, texts: &[String], out: &mut Outs) {
        let mut declarations = Declarations::new();
        let mut failed = false;
        for (index, (name, text)) in HOST.iter().zip(texts).enumerate() {
            let result = if index == 0 {
                declarations.commands(text)
            } else {
                declarations.module(name, text)
            };
            if let Err(errors) = result {
                failed = true;
                let path = format!("{}:", host_path(dir, name));
                for decl::Error { span, reason } in errors {
                    located(&path, text, span, &format!("{reason:?}"), out);
                }
            }
        }
        if failed {
            return;
        }
        let lines: Vec<String> = self
            .session
            .source()
            .split("\n\n")
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect();
        self.session = Session::new(declarations);
        self.last = None;
        for line in lines {
            let mut outcome = self.session.line(&line);
            let report = loop {
                match outcome {
                    Outcome::Request(Request::File { stem }) => {
                        let text = self.cached(&stem).map(String::from);
                        outcome = self.session.reply(Reply::File(text));
                    }
                    Outcome::Done(report) => break report,
                }
            };
            self.done(report, line, Mode::Accept, out);
        }
    }
}

fn host_path(dir: &str, name: &str) -> String {
    format!("{}/{name}.fipe", dir.trim_end_matches('/'))
}

/// `line:col: reason`, the source line and a caret under the span.
fn located(prefix: &str, text: &str, span: Span, reason: &str, out: &mut Outs) {
    let (line, col) = Source::new(text).line_col(span.start);
    let at = span.start.min(text.len());
    let start = text
        .get(..at)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1);
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let width = text
        .get(at..span.end.clamp(at, end))
        .map_or(0, |s| s.chars().count())
        .max(1);
    let row = &text[start..end];
    let rendered = format!(
        "{prefix}{line}:{col}: {reason}\n{row}\n{:pad$}{:^<width$}\n",
        "",
        "",
        pad = col - 1,
    );
    out.write(rendered.as_bytes());
}

/// Each name expression of the line: line:col, span and target, with
/// spans in the line's text and a `Local` site in the session's source.
fn names(report: &Report, line: &str, out: &mut Outs) {
    let (Some(module), Some(resolved)) = (&report.module, &report.resolved)
    else {
        return;
    };
    let mut names: Vec<_> = resolved
        .expressions()
        .filter_map(|(id, target)| {
            let expr = module.expr(id)?;
            let ExprKind::Name(name) = &expr.kind else {
                return None;
            };
            let start = expr.span.start.checked_sub(report.offset)?;
            let end = expr.span.end - report.offset;
            Some((start, end, name.segments.join("."), target))
        })
        .collect();
    names.sort_by_key(|&(start, ..)| start);
    for (start, end, name, target) in names {
        let (l, c) = Source::new(line).line_col(start);
        let text = format!("{l}:{c} {start}..{end} {name}: {target:?}\n");
        out.write(text.as_bytes());
    }
}

/// The exact spelling of a combined-text span, in the line's own text.
fn text_of(line: &str, offset: usize, span: Span) -> Option<&str> {
    let start = span.start.checked_sub(offset)?;
    let end = span.end.checked_sub(offset)?;
    Source::new(line).slice(Span { start, end })
}

/// Each top-level entry of the line: a binding's name and its scheme, or
/// the entry's own source text and its type.
fn types(report: &Report, line: &str, out: &mut Outs) {
    let (Some(module), Some(typed)) = (&report.module, &report.typed) else {
        return;
    };
    for entry in report.entries() {
        let Entry::Line(id) = entry else { continue };
        let Some(expr) = module.expr(*id) else {
            continue;
        };
        let rendered = if let ExprKind::Bind { pattern, .. } = &expr.kind {
            let name = module
                .pattern(*pattern)
                .and_then(|pattern| text_of(line, report.offset, pattern.span))
                .unwrap_or_default();
            let Some(scheme) = typed.binding(*id) else {
                continue;
            };
            format!("{name}: {scheme}\n")
        } else {
            let source =
                text_of(line, report.offset, expr.span).unwrap_or_default();
            let Some(ty) = typed.expr(*id) else {
                continue;
            };
            format!("{source}: {ty}\n")
        };
        out.write(rendered.as_bytes());
    }
}
