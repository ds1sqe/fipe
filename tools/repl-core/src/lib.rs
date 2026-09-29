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
//! has an unclosed bracket or ends in an operator, `=` or a comma, and
//! when the next line starts with an operator, which also takes back and
//! continues the entry before it once that was accepted (and accepts it
//! again if the joined entry is rejected); an empty line submits it.
//!
//! Commands: `:ast`, `:r [line]`, `:t [line]`, `:load <stem or path>`,
//! `:host [dir]`, `:q`; any other line goes to the session.

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
    lex::lex,
    session::{Diagnostic, Report, Session},
    token::{Kind, Source, Span, Symbol},
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
    input: Vec<u8>,
    /// The entry's lines so far.
    entry: String,
    wait: Option<Wait>,
    /// Replied file texts by stem, for replays and file diagnostics.
    files: Vec<(String, String)>,
    /// Paths `:load` chose for module names; others are `<name>.fipe`.
    paths: Vec<(String, String)>,
    /// Whether the last output was a prompt, which exit ends with LF.
    prompted: bool,
    /// The last accepted line, for `:ast` and `:r`.
    last: Option<(Report, String)>,
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
            entry: String::new(),
            wait: None,
            files: Vec::new(),
            paths: Vec::new(),
            prompted: false,
            last: None,
            retractable: None,
            restore: None,
            quit: false,
            ended: false,
        }
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
            if !self.entry.is_empty() && !opens(&self.entry) {
                match self.leads() {
                    Some(true) => {}
                    Some(false) => {
                        self.submit(out);
                        continue;
                    }
                    None => break,
                }
            }
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                if self.ended && !self.entry.is_empty() {
                    self.submit(out);
                    continue;
                }
                break;
            };
            if !self.entry.is_empty()
                && self.input[..end].trim_ascii_start().starts_with(b":")
            {
                // A command ends the entry before it.
                self.submit(out);
                continue;
            }
            let bytes: Vec<u8> = self.input.drain(..=end).collect();
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end_matches(['\n', '\r']);
            let blank = line.trim().is_empty();
            if blank || (self.entry.is_empty() && comment(line)) {
                if !self.entry.is_empty() {
                    self.submit(out);
                }
            } else if !self.entry.is_empty() {
                self.entry.push('\n');
                self.entry.push_str(line);
            } else if continuable(line) {
                self.entry = line.to_string();
                if leading(line) && self.retractable.take().is_some() {
                    self.continue_last();
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
            out.write(if self.entry.is_empty() { PROMPT } else { MORE });
        }
    }

    /// Whether the next line starts with an operator that continues a
    /// complete entry; `None` while that line is still arriving.
    fn leads(&self) -> Option<bool> {
        if self.input.is_empty() {
            return Some(false);
        }
        let end = self.input.iter().position(|&b| b == b'\n');
        if end.is_none() && !self.ended {
            return None;
        }
        let next = &self.input[..end.unwrap_or(self.input.len())];
        Some(leading(&String::from_utf8_lossy(next)))
    }

    fn submit(&mut self, out: &mut Outs) {
        let entry = core::mem::take(&mut self.entry);
        self.command(&entry, out);
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
            ("ast", _) => out.write(b"usage: :ast (the last accepted line)\n"),
            ("r", "") => match &self.last {
                Some((report, text)) => names(report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("r", line) => self.start(line, Mode::Resolve, out),
            ("load", "") => out.write(b"usage: :load <stem>\n"),
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
        if name != file {
            out.write(format!("{file} as {name}\n").as_bytes());
        }
        self.paths.retain(|(known, _)| *known != name);
        self.paths.push((name.clone(), format!("{stem}.fipe")));
        self.start(&format!("use {name}"), Mode::Accept, out);
    }

    fn path(&self, stem: &str) -> String {
        self.paths
            .iter()
            .find(|(name, _)| name == stem)
            .map_or_else(|| format!("{stem}.fipe"), |(_, path)| path.clone())
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
        match mode {
            Mode::Accept if report.diagnostics.iter().all(from_file) => {
                self.restore = None;
                self.retractable = Some(text.clone());
                self.last = Some((report, text));
            }
            Mode::Accept => {
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
            Mode::Resolve => names(&report, &text, out),
            Mode::Types => types(&report, &text, out),
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
        let Some((report, _)) = &self.last else {
            return out.write(b"no accepted line\n");
        };
        let Some(module) = &report.module else { return };
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
            let report = self.replay(&line);
            self.done(report, line, Mode::Accept, out);
        }
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
            let source: Vec<&str> = source.lines().map(str::trim).collect();
            let source = source.join(" ");
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
        out.write(rendered.as_bytes());
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

/// Whether an entry needs another line: an unclosed bracket, or a last
/// token that is an operator, `=`, `->`, `=>` or a comma. A `>` touching
/// what precedes it closes type arguments, as in `read<i64>`.
fn opens(entry: &str) -> bool {
    let code = entry
        .trim_start()
        .strip_prefix(':')
        .map_or(entry, |command| {
            command.split_once(' ').map_or("", |(_, arg)| arg)
        });
    let mut depth = 0_isize;
    let mut trailing = false;
    for token in lex(code).tokens {
        let Kind::Symbol(symbol) = token.kind else {
            if !matches!(token.kind, Kind::Newline | Kind::Eof) {
                trailing = false;
            }
            continue;
        };
        match symbol {
            Symbol::LParen | Symbol::LBracket | Symbol::LBrace => depth += 1,
            Symbol::RParen | Symbol::RBracket | Symbol::RBrace => depth -= 1,
            _ => {}
        }
        let closes = symbol == Symbol::Greater
            && !code[..token.span.start].ends_with([' ', '\t']);
        trailing = (operator(symbol) && !closes)
            || matches!(symbol, Symbol::Assign | Symbol::Arrow | Symbol::Comma);
    }
    depth > 0 || trailing
}

/// Whether a line starts with an operator that continues the line
/// before it: a pipe, `+ - * %`, a comparison or `=>`.
fn leading(line: &str) -> bool {
    lex(line).tokens.first().is_some_and(
        |token| matches!(token.kind, Kind::Symbol(symbol) if operator(symbol)),
    )
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
                | Symbol::Eq
                | Symbol::NotEq
                | Symbol::Less
                | Symbol::LessEq
                | Symbol::Greater
                | Symbol::GreaterEq
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
