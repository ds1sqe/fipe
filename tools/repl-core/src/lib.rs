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
//! An entry continues onto the next line, prompted by `....> `, while it
//! has an unclosed bracket or ends in an operator, `=` or a comma; an
//! empty line submits it. A complete entry is submitted at once, and a
//! next line that starts with an operator takes the accepted entry back
//! and continues it (accepting it again if the joined entry is
//! rejected), whether the lines arrive together or one at a time. A
//! command, an empty line or a comment line ends that chance. A `:t` or
//! `:r` query is answered when its entry is complete. A line that starts with `where`
//! continues an accepted `type` item the same way.
//!
//! `:ast` prints the last line that parsed, accepted or not: a tree
//! never depends on checking. Commands: `:ast`, `:r [line]`, `:t [line]`, `:load <stem or path>`,
//! `:checks [name]`, `:host [dir]`, `:q`; any other line goes to the session.
//! `:{` and `:}` on their own lines collect one compilation unit; every
//! intervening line is source text, including commands. EOF submits it.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod render;
mod tree;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use fipe::{
    ast::{Entry, ExprKind},
    decl::{self, Declarations},
    embed::{Outcome, Reply, Request},
    lex::{lex, Lexed},
    session::{Diagnostic, Report, Session},
    token::{Keyword, Kind, Source, Span, Symbol, Token},
};
use render::Context;

const PROMPT: &[u8] = b"fipe> ";
const MORE: &[u8] = b"....> ";
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
    strict: bool,
    input: Vec<u8>,
    /// The entry's lines so far.
    entry: String,
    continuation: Continuation,
    unit: bool,
    wait: Option<Wait>,
    /// Replied file texts by stem, for replays and file diagnostics.
    files: Vec<(String, String)>,
    /// Paths `:load` chose for module names; others are `<name>.fipe`.
    paths: Vec<(String, String)>,
    /// Whether the last output was a prompt, which exit ends with LF.
    prompted: bool,
    /// The last accepted line, for `:t` and `:r`.
    last: Option<(Report, String)>,
    /// The last line that parsed, accepted or not, for `:ast`.
    parsed: Option<Report>,
    /// The session's last line when the entry before this one was it,
    /// which a line starting with an operator continues.
    retractable: Option<String>,
    /// A retracted line, accepted again if its continuation is rejected.
    restore: Option<String>,
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
    /// `:load`'s `use`, accepted like a line but never continued.
    Load,
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
            strict: false,
            input: Vec::new(),
            entry: String::new(),
            continuation: Continuation::default(),
            unit: false,
            wait: None,
            files: Vec::new(),
            paths: Vec::new(),
            prompted: false,
            last: None,
            parsed: None,
            retractable: None,
            restore: None,
            quit: false,
            ended: false,
        }
    }

    /// Reject open implicit checks, including after a host reload.
    pub fn strict(&mut self, enabled: bool) {
        self.strict = enabled;
        self.session.strict(enabled);
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Out> {
        self.input.extend_from_slice(bytes);
        let mut out = Outs::default();
        self.run(&mut out);
        self.finish(out)
    }

    /// End of input: a trailing line without LF and a buffered entry run
    /// as complete, after an LF that ends the prompt's line.
    pub fn flush(&mut self) -> Vec<Out> {
        let mut out = Outs::default();
        if !self.ended && self.prompted {
            out.write(b"\n");
        }
        self.ended = true;
        if !self.input.is_empty() && !self.input.ends_with(b"\n") {
            self.input.push(b'\n');
        }
        self.run(&mut out);
        self.finish(out)
    }

    fn finish(&mut self, out: Outs) -> Vec<Out> {
        if let Some(Out::Write(bytes)) = out.0.last() {
            self.prompted = bytes.ends_with(PROMPT) || bytes.ends_with(MORE);
        }
        out.0
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
        self.finish(out)
    }

    fn run(&mut self, out: &mut Outs) {
        while !self.quit && self.wait.is_none() {
            if !self.unit && !self.entry.is_empty() && !self.continuation.open {
                self.submit(out);
                continue;
            }
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                if self.ended && (self.unit || !self.entry.is_empty()) {
                    self.submit(out);
                    continue;
                }
                break;
            };
            if !self.unit
                && self.continuation.next.is_some_and(|next| {
                    let line = String::from_utf8_lossy(&self.input[..end]);
                    let tokens = lex(&line).tokens;
                    match next {
                        NextLine::Assign => !matches!(
                            tokens[0].kind,
                            Kind::Symbol(Symbol::Assign)
                        ),
                        NextLine::Where => !where_word(&tokens[0].kind),
                    }
                })
            {
                self.submit(out);
                continue;
            }
            if !self.unit
                && !self.entry.is_empty()
                && self.continuation.quote.is_none()
                && self.input[..end].trim_ascii_start().starts_with(b":")
            {
                // A command ends the entry before it.
                self.submit(out);
                continue;
            }
            let bytes: Vec<u8> = self.input.drain(..=end).collect();
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end_matches(['\n', '\r']);
            if self.unit {
                if line.trim() == ":}" {
                    self.submit(out);
                } else {
                    self.entry.push_str(line);
                    self.entry.push('\n');
                }
                continue;
            }
            let blank = line.trim().is_empty();
            if (blank
                && self.continuation.quote.is_none()
                && !self.continuation.doc)
                || (self.entry.is_empty() && comment(line))
            {
                if !self.entry.is_empty() {
                    self.submit(out);
                }
                self.retractable = None;
            } else if !self.entry.is_empty() {
                self.entry.push('\n');
                self.entry.push_str(line);
                if self.continuation.quote != Some('\'') || line.contains('\'')
                {
                    self.continuation =
                        continuation(code(&self.entry), lex(code(&self.entry)));
                }
            } else if continuable(line) {
                self.entry = line.to_string();
                let lexed = lex(code(line));
                let first = lexed.tokens.first();
                let joins = !query(line)
                    && (leading_tokens(&lexed.tokens)
                        || (first
                            .is_some_and(|token| where_word(&token.kind))
                            && self.retractable.as_deref().is_some_and(
                                |text| type_item(&lex(text).tokens),
                            )));
                if joins && self.retractable.take().is_some() {
                    self.continue_last();
                    self.continuation =
                        continuation(code(&self.entry), lex(code(&self.entry)));
                } else {
                    self.continuation = continuation(code(line), lexed);
                }
            } else {
                self.command(line, out);
            }
        }
        if !self.quit
            && !self.ended
            && self.wait.is_none()
            && self.input.is_empty()
        {
            out.write(if self.entry.is_empty() && !self.unit {
                PROMPT
            } else {
                MORE
            });
        }
    }

    fn submit(&mut self, out: &mut Outs) {
        let entry = core::mem::take(&mut self.entry);
        self.continuation = Continuation::default();
        if core::mem::take(&mut self.unit) {
            self.start(&entry, Mode::Accept, out);
        } else {
            self.command(&entry, out);
        }
    }

    fn command(&mut self, line: &str, out: &mut Outs) {
        let Some(command) = line.trim().strip_prefix(':') else {
            if !line.trim().is_empty() {
                self.start(line, Mode::Accept, out);
            }
            return;
        };
        self.retractable = None;
        let (name, arg) = command
            .split_once(' ')
            .map_or((command, ""), |(name, arg)| (name, arg.trim()));
        match (name, arg) {
            ("{", "") => self.unit = true,
            ("q", "") => {
                self.quit = true;
                out.0.push(Out::Exit);
            }
            ("t", "") => match &self.last {
                Some((report, text)) => types(&self.session, report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("t", arg) => self.start(arg, Mode::Types, out),
            ("checks", arg) => {
                if !arg.is_empty() {
                    self.start(arg, Mode::Types, out);
                }
                self.checks((!arg.is_empty()).then_some(arg), out);
            }
            ("ast", "") => self.ast(out),
            ("ast", _) => {
                out.write(b"usage: :ast (the last line that parsed)\n");
            }
            ("r", "") => match &self.last {
                Some((report, text)) => names(&self.session, report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("r", line) => self.start(line, Mode::Resolve, out),
            ("load", "") => out.write(b"usage: :load <stem or path>\n"),
            ("load", arg) => self.load(arg, out),
            ("host", dir) => {
                let dir = if dir.is_empty() { HOST_DIR } else { dir };
                out.0.push(Out::File(host_path(dir, HOST[0])));
                self.wait = Some(Wait::Host {
                    dir: dir.to_string(),
                    texts: Vec::new(),
                });
            }
            _ => out.write(format!("unknown command :{name}\n").as_bytes()),
        }
    }

    fn checks(&self, name: Option<&str>, out: &mut Outs) {
        let checks = self.session.checks(name);
        let lines: Vec<_> = checks
            .iter()
            .map(|check| render::check(check, None))
            .collect();
        for (check, line) in checks.iter().zip(&lines) {
            if lines.iter().filter(|other| *other == line).count() > 1 {
                let (source, prefix) = match &check.place_origin {
                    fipe::resolve::Origin::File(stem) => (
                        self.cached(stem).unwrap_or_default(),
                        format!("{stem}:"),
                    ),
                    _ => (self.session.source(), String::new()),
                };
                let (line, column) =
                    Source::new(source).line_col(check.place.start);
                out.write(
                    render::check(
                        check,
                        Some(&format!("{prefix}{line}:{column}")),
                    )
                    .as_bytes(),
                );
            } else {
                out.write(line.as_bytes());
            }
        }
    }

    /// `use` the module of a stem or path; a file name that is not a
    /// Name, such as `01-files`, is used by its name part, `files`.
    fn load(&mut self, arg: &str, out: &mut Outs) {
        let stem = arg.strip_suffix(".fipe").unwrap_or(arg);
        let file = stem.rsplit('/').next().unwrap_or(stem);
        let name = module_name(file);
        if name.is_empty() {
            let text = format!("{arg}: no module name in the file name\n");
            return out.write(text.as_bytes());
        }
        // The host's modules shadow file modules once `:host` runs.
        if HOST[1..].contains(&name.as_str()) {
            let text = format!("{arg}: `{name}` is a host module\n");
            return out.write(text.as_bytes());
        }
        let path = format!("{stem}.fipe");
        if self.cached(&name).is_some() && self.path(&name) != path {
            let text = format!("{file}: already loaded as `{name}`\n");
            return out.write(text.as_bytes());
        }
        if name != file {
            out.write(format!("{file} as {name}\n").as_bytes());
        }
        self.paths.retain(|(known, _)| *known != name);
        self.paths.push((name.clone(), path));
        self.start(&format!("use {name}"), Mode::Load, out);
    }

    fn path(&self, stem: &str) -> String {
        self.paths
            .iter()
            .find(|(name, _)| name == stem)
            .map_or_else(|| format!("{stem}.fipe"), |(_, path)| path.clone())
    }

    fn start(&mut self, text: &str, mode: Mode, out: &mut Outs) {
        let outcome = match mode {
            Mode::Accept | Mode::Load => self.session.line(text),
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
                out.0.push(Out::File(self.path(&stem)));
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
        if matches!(mode, Mode::Accept) && report.module.is_some() {
            self.parsed = Some(report.clone());
        }
        match mode {
            Mode::Accept | Mode::Load
                if report.diagnostics.iter().all(from_file) =>
            {
                self.restore = None;
                if matches!(mode, Mode::Accept) {
                    self.retractable = Some(text.clone());
                }
                self.last = Some((report, text));
            }
            Mode::Accept | Mode::Load => {
                self.retractable = None;
                if let Some(line) = self.restore.take() {
                    // Nothing is lost: the line before is kept as it was.
                    let report = self.replay(&line);
                    if report.diagnostics.iter().all(from_file) {
                        self.retractable = Some(line.clone());
                        self.last = Some((report, line));
                    }
                }
            }
            Mode::Resolve => names(&self.session, &report, &text, out),
            Mode::Types => types(&self.session, &report, &text, out),
        }
    }

    fn diagnostic(&self, diagnostic: &Diagnostic, line: &str, out: &mut Outs) {
        let at = Context {
            source: self.session.source(),
            line,
        };
        match diagnostic {
            Diagnostic::Syntax(error) => {
                let reason = render::syntax(error.reason);
                located("", line, error.span, &reason, out);
            }
            Diagnostic::Resolve(error) => {
                let reason = render::resolve(&error.reason, &at);
                located("", line, error.span, &reason, out);
            }
            Diagnostic::Type(error) => {
                let reason = render::infer(&error.reason);
                located("", line, error.span, &reason, out);
            }
            Diagnostic::File { stem, error } => {
                let text = self.cached(stem).unwrap_or_default();
                let path = format!("{}:", self.path(stem));
                let reason = render::syntax(error.reason);
                located(&path, text, error.span, &reason, out);
            }
            Diagnostic::FileResolve { stem, error } => {
                let text = self.cached(stem).unwrap_or_default();
                let path = format!("{}:", self.path(stem));
                let at = Context {
                    source: "",
                    line: text,
                };
                let reason = render::resolve(&error.reason, &at);
                located(&path, text, error.span, &reason, out);
            }
            Diagnostic::FileType { stem, error } => {
                let text = self.cached(stem).unwrap_or_default();
                let path = format!("{}:", self.path(stem));
                let reason = render::infer(&error.reason);
                located(&path, text, error.span, &reason, out);
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
        let Some(report) = &self.parsed else {
            return out.write(b"no line has parsed\n");
        };
        let Some(module) = &report.module else { return };
        if let Some(doc) = &module.doc {
            out.write(format!("Module doc {doc:?}\n").as_bytes());
        }
        for entry in report.entries() {
            out.write(tree::entry(module, entry).as_bytes());
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
                    let reason = render::declaration(&reason, text);
                    located(&path, text, span, &reason, out);
                }
            }
        }
        if failed {
            return;
        }
        let lines: Vec<String> =
            self.session.lines().map(String::from).collect();
        self.session = Session::new(declarations);
        self.session.strict(self.strict);
        self.last = None;
        for line in lines {
            let report = self.replay(&line);
            self.done(report, line, Mode::Accept, out);
        }
        // `:host` is a command: the next line starts a fresh entry.
        self.retractable = None;
        out.write(format!("host: {dir}\n").as_bytes());
    }

    /// Accept a line again, answering its `use` requests from the cache.
    fn replay(&mut self, line: &str) -> Report {
        let mut outcome = self.session.line(line);
        loop {
            match outcome {
                Outcome::Request(Request::File { stem }) => {
                    let text = self.cached(&stem).map(String::from);
                    outcome = self.session.reply(Reply::File(text));
                }
                Outcome::Done(report) => return report,
            }
        }
    }

    /// The fresh entry starts with an operator and the entry before it
    /// was accepted: take that line back and continue it.
    fn continue_last(&mut self) {
        let Some(last) = self.session.retract() else {
            return;
        };
        self.entry = format!("{last}\n{}", self.entry);
        self.restore = Some(last);
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
    // Padding by repetition: a format width panics above 65535.
    let mut rendered = format!("{prefix}{line}:{col}: {reason}\n{row}\n");
    rendered.push_str(&" ".repeat(col - 1));
    rendered.push_str(&"^".repeat(width));
    rendered.push('\n');
    out.write(rendered.as_bytes());
}

/// Each name expression of the line: line:col, span and target, with
/// spans in the line's text and a `Local` site in the session's source.
fn names(session: &Session, report: &Report, line: &str, out: &mut Outs) {
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
        let at = Context {
            source: session.source(),
            line,
        };
        let target = render::target(target, &at);
        let text = format!("{l}:{c} {start}..{end} {name}: {target}\n");
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
fn types(session: &Session, report: &Report, line: &str, out: &mut Outs) {
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
            let source = if source.contains(['\'', '"']) {
                source.to_string()
            } else {
                source.lines().map(str::trim).collect::<Vec<_>>().join(" ")
            };
            // A query's line has a generalized scheme; an accepted
            // line's expression only its type.
            if let Some(scheme) = typed.binding(*id) {
                format!("{source}: {scheme}\n")
            } else if let Some(ty) = typed.expr(*id) {
                format!("{source}: {ty}\n")
            } else {
                continue;
            }
        };
        if let Some(doc) = session.documentation(report, *id) {
            out.write(doc.as_bytes());
            out.write(b"\n");
        }
        out.write(rendered.as_bytes());
        let value = if let ExprKind::Bind { value, .. } = expr.kind {
            value
        } else {
            *id
        };
        if let Some(known) = typed.known(value) {
            out.write(format!("{known}").as_bytes());
        }
    }
}

/// A diagnostic of a file the line loaded, which does not reject it.
const fn from_file(diagnostic: &Diagnostic) -> bool {
    match diagnostic {
        Diagnostic::File { .. }
        | Diagnostic::FileResolve { .. }
        | Diagnostic::FileType { .. } => true,
        Diagnostic::Syntax(_)
        | Diagnostic::Resolve(_)
        | Diagnostic::Type(_) => false,
    }
}

/// A comment-only line, which is blank to the grammar.
fn comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
        && (!line.trim_start().starts_with("##")
            || line.trim_start().starts_with("###"))
}

/// A `:t` or `:r` entry, held for a next line that continues it.
fn query(entry: &str) -> bool {
    entry.trim_start().starts_with(':')
}

/// A fresh line may start an entry unless it is a command other than
/// `:t` or `:r` with a line.
fn continuable(line: &str) -> bool {
    line.trim_start().strip_prefix(':').is_none_or(|command| {
        command.split_once(' ').is_some_and(|(name, arg)| {
            matches!(name, "t" | "r") && !arg.trim().is_empty()
        })
    })
}

/// Whether an entry needs another line: an unclosed bracket, a last
/// token that is an operator, `=`, `->`, `=>`, a comma or a type item's
/// `where`, or a bare signature waiting for its `= value`. A `>`
/// touching what precedes it closes type arguments, as in `read<I64>`.
#[derive(Default)]
struct Continuation {
    open: bool,
    quote: Option<char>,
    doc: bool,
    next: Option<NextLine>,
}

#[derive(Clone, Copy)]
enum NextLine {
    Assign,
    Where,
}

fn code(entry: &str) -> &str {
    entry
        .trim_start()
        .strip_prefix(':')
        .map_or(entry, |command| {
            command.split_once(' ').map_or("", |(_, arg)| arg)
        })
}

fn continuation(code: &str, lexed: Lexed) -> Continuation {
    let quote = open_quote(code, &lexed);
    let tokens = &lexed.tokens;
    let doc = pending_doc(tokens);
    let mut result = Continuation {
        open: quote.is_some() || doc,
        quote,
        doc,
        next: None,
    };
    if result.open || !lexed.errors.is_empty() {
        return result;
    }
    let limited = type_item(tokens);
    let mut depth = 0_isize;
    let mut trailing = false;
    let mut assigns = false;
    for token in tokens {
        let Kind::Symbol(symbol) = token.kind else {
            if !matches!(token.kind, Kind::Newline | Kind::Eof) {
                trailing = limited && where_word(&token.kind);
            }
            continue;
        };
        match symbol {
            Symbol::LParen | Symbol::LBracket | Symbol::LBrace => depth += 1,
            Symbol::RParen | Symbol::RBracket | Symbol::RBrace => depth -= 1,
            Symbol::Assign if depth == 0 => assigns = true,
            _ => {}
        }
        let closes = symbol == Symbol::Greater
            && !code[..token.span.start].ends_with([' ', '\t']);
        trailing = (operator(symbol) && !closes)
            || matches!(symbol, Symbol::Assign | Symbol::Arrow | Symbol::Comma);
    }
    result.doc = depth > 0
        && tokens
            .iter()
            .rev()
            .find(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
            .is_some_and(|token| {
                matches!(token.kind, Kind::Doc(_) | Kind::InnerDoc(_))
            });
    // A discarded join occupies the source gap after the last token/trivia.
    let end = tokens
        .iter()
        .filter(|t| !matches!(t.kind, Kind::Eof))
        .map(|t| t.span.end)
        .chain(lexed.comments.iter().map(|s| s.end))
        .max()
        .unwrap_or(0);
    let joined = code[end..].contains('\\');
    let bare_type = limited
        && !tokens.iter().any(|t| {
            matches!(t.kind, Kind::Symbol(Symbol::LBrace | Symbol::Ampersand))
                || where_word(&t.kind)
        });
    if depth == 0 && !trailing && !joined {
        result.next = if !assigns && signature(tokens) {
            Some(NextLine::Assign)
        } else if bare_type {
            Some(NextLine::Where)
        } else {
            None
        };
    }
    result.open = depth > 0 || trailing || joined || result.next.is_some();
    result
}

/// A lexer failure at EOF that another physical line can complete.
fn open_quote(code: &str, lexed: &Lexed) -> Option<char> {
    lexed.errors.iter().find_map(|error| {
        if error.span.end != code.len() {
            return None;
        }
        let written = &code[error.span.start..error.span.end];
        let at = written.find(['\'', '"'])?;
        let quote = written.as_bytes()[at] as char;
        ((quote == '\'' && !written[at + 1..].contains('\''))
            || (quote == '"' && written.ends_with('\\')))
        .then_some(quote)
    })
}

fn pending_doc(tokens: &[Token]) -> bool {
    let Some(last) = tokens
        .iter()
        .rposition(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
    else {
        return false;
    };
    if !matches!(tokens[last].kind, Kind::Doc(_) | Kind::InnerDoc(_)) {
        return false;
    }
    last == 0 || matches!(tokens[last - 1].kind, Kind::Newline)
}

/// Whether the entry starts `[pub] NAME :`, a signature's head. Only the
/// head is scanned; the type after the colon is the parser's.
fn signature(tokens: &[Token]) -> bool {
    let mut kinds = tokens.iter().map(|token| &token.kind).skip_while(|kind| {
        matches!(kind, Kind::Doc(_) | Kind::InnerDoc(_) | Kind::Newline)
    });
    let mut first = kinds.next();
    if matches!(first, Some(Kind::Kw(Keyword::Pub))) {
        first = kinds.next();
    }
    matches!(first, Some(Kind::Name(segments)) if segments.len() == 1)
        && matches!(kinds.next(), Some(Kind::Symbol(Symbol::Colon)))
}

/// Whether an entry is a `type` item, which a `where` line continues.
fn type_item(tokens: &[Token]) -> bool {
    let mut kinds = tokens.iter().map(|token| &token.kind).skip_while(|kind| {
        matches!(kind, Kind::Doc(_) | Kind::InnerDoc(_) | Kind::Newline)
    });
    let mut first = kinds.next();
    if matches!(first, Some(Kind::Kw(Keyword::Pub))) {
        first = kinds.next();
    }
    matches!(first, Some(Kind::Kw(Keyword::Type)))
}

fn where_word(kind: &Kind) -> bool {
    matches!(kind, Kind::Name(parts) if parts.len() == 1 && parts[0] == "where")
}

/// Whether a line starts with an operator that continues the line
/// before it: a pipe, `+ - * %`, a comparison, `=>` or `=`, so a
/// signature may stand above its `= value`.
fn leading_tokens(tokens: &[Token]) -> bool {
    let Some(first) = tokens.first() else {
        return false;
    };
    match first.kind {
        Kind::Symbol(Symbol::Slash) => tokens.get(1).is_some_and(|next| {
            matches!(
                next.kind,
                Kind::Name(_)
                    | Kind::Word(_)
                    | Kind::Path(_)
                    | Kind::Selector(_)
                    | Kind::Int { .. }
                    | Kind::Float(_)
                    | Kind::Str { .. }
                    | Kind::Affixed(_)
                    | Kind::Symbol(
                        Symbol::LParen
                            | Symbol::LBracket
                            | Symbol::LBrace
                            | Symbol::Slash
                    )
            )
        }),
        Kind::Symbol(symbol) => operator(symbol) || symbol == Symbol::Assign,
        _ => false,
    }
}

/// The operators that continue a line from either side; `/` does not,
/// as a lone `/` is the root path.
const fn operator(symbol: Symbol) -> bool {
    symbol.continues_line()
        || matches!(
            symbol,
            Symbol::Plus
                | Symbol::Minus
                | Symbol::Star
                | Symbol::Percent
                | Symbol::Concat
                | Symbol::Eq
                | Symbol::NotEq
                | Symbol::Less
                | Symbol::LessEq
                | Symbol::Greater
                | Symbol::GreaterEq
                | Symbol::Ampersand
                | Symbol::FatArrow
        )
}

/// The Name part of a file stem: from its first letter or `_`, with
/// every other character that a Name cannot hold as `_`.
fn module_name(file: &str) -> String {
    file.trim_start_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '_')
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}
