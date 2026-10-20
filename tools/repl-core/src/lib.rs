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
//! An entry is the input up to a blank line: the lines before it are
//! one compilation unit, checked and accepted once, as a file's lines
//! are, whether they arrive together or one at a time. A blank line
//! inside an open bracket, in a quote across lines or after a doc
//! comment waiting for its item stays inside the entry, which is
//! prompted by `....> `. End of input submits what is held. A command
//! line, `:` first, runs on its own line after submitting the entry
//! before it; a bracket or quote it opens is closed first. A comment
//! line between entries is skipped.
//!
//! `:ast` prints the last line that parsed, accepted or not: a tree
//! never depends on checking. Commands: `:ast`, `:r [line]`, `:t [line]`,
//! `:ir [line]`, `:load <stem or path>`, `:checks [name]`, `:host [dir]`,
//! `:q`; any other line goes to the session.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod ir;
mod mir;
mod render;
mod tree;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
pub use fipe::check::COMPILE_BUDGET;
use fipe::{
    ast::{Entry, ExprKind},
    decl::{self, Declarations},
    embed::{Outcome, Reply, Request},
    lex::{lex, Lexed},
    session::{Diagnostic, Report, Session},
    token::{Kind, Source, Span, Symbol, Token},
    value::Value,
    vm::{Fault, Machine},
};
use render::Context;

const PROMPT: &[u8] = b"fipe> ";
/// The REPL's own budget per line when the driver names none: a surface
/// constant, not the language's.
pub const BUDGET: u64 = 1 << 20;
const MORE: &[u8] = b"....> ";
const HOST_DIR: &str = "tests/fixtures/host";
/// The command namespace first, then the host modules by name.
const HOST: [&str; 6] = ["commands", "fs", "sys", "cli", "text", "config"];

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
    /// The host's compile budget, kept across a host reload.
    compile_budget: u64,
    input: Vec<u8>,
    /// The entry's lines so far.
    entry: String,
    continuation: Continuation,
    wait: Option<Wait>,
    /// Replied file texts by stem, for replays and file diagnostics.
    files: Vec<(String, String)>,
    /// Declaration sources in HOST order, for written-test descriptions.
    host_texts: Vec<String>,
    /// The directory the declaration sources came from.
    host_dir: String,
    /// Paths `:load` chose for module names; others are `<name>.fipe`.
    paths: Vec<(String, String)>,
    /// How many file texts, and which paths, stood before the line in
    /// progress: what a withdrawn line leaves.
    before: (usize, Vec<(String, String)>),
    /// Paths `:load` chose on lines later withdrawn, by module name: a
    /// later `use` of the name says why its file is not found.
    withdrawn_loads: Vec<(String, String)>,
    /// Whether the last output was a prompt, which exit ends with LF.
    prompted: bool,
    /// The last accepted line, for `:t` and `:r`.
    last: Option<(Report, String)>,
    /// The last line that parsed, accepted or not, for `:ast`.
    parsed: Option<Report>,
    /// The machine every accepted line runs on; its globals are the
    /// session's values.
    machine: Machine,
    /// Accepted lines replayed after a host reload run without output.
    replaying: bool,
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
    Ir,
    Mir,
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
            compile_budget: COMPILE_BUDGET,
            input: Vec::new(),
            entry: String::new(),
            continuation: Continuation::default(),
            wait: None,
            files: Vec::new(),
            host_texts: Vec::new(),
            host_dir: String::new(),
            paths: Vec::new(),
            withdrawn_loads: Vec::new(),
            before: (0, Vec::new()),
            prompted: false,
            last: None,
            parsed: None,
            machine: Machine::new(Some(BUDGET)),
            replaying: false,
            quit: false,
            ended: false,
        }
    }

    /// The charge budget each accepted line runs under.
    pub const fn budget(&mut self, budget: u64) {
        self.machine.set_budget(Some(budget));
    }

    /// Reject open implicit checks, including after a host reload.
    pub fn strict(&mut self, enabled: bool) {
        self.strict = enabled;
        self.session.strict(enabled);
    }

    /// The charge units each compile-time evaluation may spend,
    /// including after a host reload.
    pub fn compile_budget(&mut self, units: u64) {
        self.compile_budget = units;
        self.session.compile_budget(units);
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
                    self.withdrawn_loads.retain(|(name, _)| *name != stem);
                    self.files.push((stem, text.to_string()));
                } else if let Some((name, path)) =
                    self.withdrawn_loads.iter().find(|(name, _)| *name == stem)
                {
                    out.write(
                        format!(
                            "`{name}`: its `:load` of {path} was withdrawn when its run Faulted; load it again with `:load {path}`\n"
                        )
                        .as_bytes(),
                    );
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
            let Some(end) = self.input.iter().position(|&b| b == b'\n') else {
                if self.ended && !self.entry.is_empty() {
                    self.submit(out);
                    continue;
                }
                break;
            };
            if !self.entry.is_empty()
                && self.continuation.quote.is_none()
                && self.continuation.depth == 0
                && self.input[..end].trim_ascii_start().starts_with(b":")
            {
                // A command submits the entry before it and stands alone.
                self.submit(out);
                continue;
            }
            let bytes: Vec<u8> = self.input.drain(..=end).collect();
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end_matches(['\n', '\r']);
            let blank = line.trim().is_empty();
            if self.entry.is_empty() {
                if blank || comment(line) {
                    continue;
                }
                self.entry = line.to_string();
                self.continuation = continuation(code(line), lex(code(line)));
            } else if blank && !self.continuation.open {
                self.submit(out);
                continue;
            } else {
                self.entry.push('\n');
                self.entry.push_str(line);
                self.extend(line);
            }
            if command_entry(&self.entry) && !self.continuation.open {
                // A command runs once what it opened is closed.
                self.submit(out);
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

    /// Carry the continuation over a line added to the entry. Outside a
    /// quote and a pending doc only the new line is lexed, whatever the
    /// bracket depth, so a long entry costs its own length once; an open
    /// raw quote waits for a line holding `'`; otherwise the whole entry
    /// is read again, since the quote changes how its lines lex.
    fn extend(&mut self, line: &str) {
        if self.continuation.quote.is_none() && !self.continuation.doc {
            let depth = self.continuation.depth;
            self.continuation = continuation(line, lex(line));
            self.continuation.depth += depth;
            self.continuation.open |= self.continuation.depth > 0;
        } else if self.continuation.quote != Some('\'') || line.contains('\'') {
            self.continuation =
                continuation(code(&self.entry), lex(code(&self.entry)));
        }
    }

    fn submit(&mut self, out: &mut Outs) {
        let entry = core::mem::take(&mut self.entry);
        self.continuation = Continuation::default();
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
            .split_once(char::is_whitespace)
            .map_or((command, ""), |(name, arg)| (name, arg.trim()));
        match (name, arg) {
            ("q", "") => {
                self.quit = true;
                out.0.push(Out::Exit);
            }
            ("t", "") => match &self.last {
                Some((report, text)) => types(&self.session, report, text, out),
                None => out.write(b"no accepted line\n"),
            },
            ("t", arg) => self.start(arg, Mode::Types, out),
            ("ir", "") => match &self.last {
                Some((report, _)) => ir::render(report.unit.as_ref(), out),
                None => out.write(b"no accepted line\n"),
            },
            ("ir", arg) => self.start(arg, Mode::Ir, out),
            ("mir", "") => match &self.last {
                Some((report, _)) => mir::render(report.unit.as_ref(), out),
                None => out.write(b"no accepted line\n"),
            },
            ("mir", arg) => self.start(arg, Mode::Mir, out),
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
            ("host", dir) => self.request_host(dir, out),
            _ => out.write(format!("unknown command :{name}\n").as_bytes()),
        }
    }

    /// Load host declarations from a directory without interpreting it as input.
    pub fn host_directory(&mut self, dir: &str) -> Vec<Out> {
        let mut out = Outs::default();
        self.request_host(dir, &mut out);
        self.finish(out)
    }

    fn request_host(&mut self, dir: &str, out: &mut Outs) {
        let dir = if dir.is_empty() { HOST_DIR } else { dir };
        if dir.contains('\0') {
            out.write(b"host path contains a NUL character\n");
            return;
        }
        out.0.push(Out::File(host_path(dir, HOST[0])));
        self.wait = Some(Wait::Host {
            dir: dir.to_string(),
            texts: Vec::new(),
        });
    }

    fn checks(&self, name: Option<&str>, out: &mut Outs) {
        for settled in self.session.settled(name) {
            if name.is_none() {
                if let Some(owner) = &settled.check.owner {
                    out.write(format!("{owner}: ").as_bytes());
                }
            }
            let test = self.check_test(&settled.check);
            out.write(render::settled(&settled, test).as_bytes());
        }
        let checks = self.session.checks(name);
        let lines: Vec<_> = checks
            .iter()
            .map(|check| render::check(check, None, self.check_test(check)))
            .collect();
        let mut counts = alloc::collections::BTreeMap::new();
        for line in &lines {
            *counts.entry(line).or_insert(0) += 1;
        }
        let mut offset = 0;
        let sources: alloc::collections::BTreeMap<_, _> = self
            .session
            .lines()
            .map(|text| {
                let start = offset;
                offset += text.len() + 2;
                (start, text)
            })
            .collect();
        for (check, line) in checks.iter().zip(&lines) {
            if name.is_none() {
                if let Some(owner) = &check.owner {
                    out.write(format!("{owner}: ").as_bytes());
                }
            }
            if counts.get(line).is_some_and(|count| *count > 1) {
                let (source, prefix, offset) = match &check.place_origin {
                    fipe::resolve::Origin::File(stem) => (
                        self.cached(stem).unwrap_or_default(),
                        format!("{stem}:"),
                        0,
                    ),
                    _ => {
                        sources.range(..=check.place.start).next_back().map_or(
                            (self.session.source(), String::new(), 0),
                            |(offset, text)| (*text, String::new(), *offset),
                        )
                    }
                };
                let (line, column) =
                    Source::new(source).line_col(check.place.start - offset);
                out.write(
                    render::check(
                        check,
                        Some(&format!("{prefix}{line}:{column}")),
                        self.check_test(check),
                    )
                    .as_bytes(),
                );
            } else {
                out.write(line.as_bytes());
            }
        }
    }

    fn check_test(&self, check: &fipe::typed::Check) -> Option<&str> {
        use fipe::resolve::Origin;
        if check.test_count <= 1 {
            return None;
        }
        let source = match &check.limit_origin {
            Origin::This | Origin::Line(_) => self.session.source(),
            Origin::File(stem) => self.cached(stem)?,
            Origin::Prelude => include_str!("../../../prelude/prelude.fipe"),
            Origin::Commands => self.host_texts.first()?.as_str(),
            Origin::Host(name) => self
                .host_texts
                .get(HOST.iter().position(|n| *n == name)?)?
                .as_str(),
        };
        Source::new(source).slice(check.limit)
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
        self.before = (self.files.len(), self.paths.clone());
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
        // `load` noted what stood before it changed `paths`.
        if matches!(mode, Mode::Accept) {
            self.before = (self.files.len(), self.paths.clone());
        }
        let outcome = match mode {
            Mode::Accept | Mode::Load => self.session.line(text),
            Mode::Resolve | Mode::Types | Mode::Ir | Mode::Mir => {
                self.session.query(text)
            }
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
            Outcome::Request(Request::Call { .. }) => {
                unreachable!("a session asks only for files")
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
        for warning in &report.warnings {
            let reading = match warning.style {
                fipe::ast::CallStyle::Space => "one tuple argument",
                fipe::ast::CallStyle::Parenthesized => "an argument list",
            };
            located("", &text, warning.span, &format!("warning: written as {reading}; both readings bind; write double parentheses to keep the tuple"), out);
        }
        for diagnostic in &report.diagnostics {
            self.diagnostic(diagnostic, &report, &text, out);
        }
        if matches!(mode, Mode::Accept) && report.module.is_some() {
            self.parsed = Some(report.clone());
        }
        match mode {
            Mode::Accept | Mode::Load => {
                if report.diagnostics.iter().all(from_file)
                    && self.execute(&report, &text, out)
                {
                    self.last = Some((report, text));
                }
            }
            Mode::Resolve => names(&self.session, &report, &text, out),
            Mode::Types => types(&self.session, &report, &text, out),
            Mode::Ir => ir::render(report.unit.as_ref(), out),
            Mode::Mir => mir::render(report.unit.as_ref(), out),
        }
    }

    /// Run an accepted line on the machine: its value in the language's
    /// spelling, nothing for `()`, or the Fault where it happened, after
    /// which the line is withdrawn. Whether the line stands.
    fn execute(&mut self, report: &Report, text: &str, out: &mut Outs) -> bool {
        let Some(unit) = &report.unit else {
            return true;
        };
        let program = fipe::compile::compile(unit);
        let mut outcome = self.machine.run(&program);
        // No host is wired yet: each host call fails, naming its
        // declaration, and the run Faults there.
        while let Outcome::Request(request) = outcome {
            let Request::Call { declaration, .. } = request else {
                unreachable!("a run asks only for host calls")
            };
            let message = {
                let tables = program.tables.borrow();
                let declaration = &tables.declarations[declaration];
                format!(
                    "no host for {}.{}",
                    tables.namespaces[declaration.namespace], declaration.label
                )
            };
            outcome = self.machine.reply(&program, Reply::Value(Err(message)));
        }
        match outcome {
            Outcome::Done(Ok(value)) => {
                if !self.replaying && !matches!(value, Value::Unit) {
                    let shown = render::value(&value, &program.tables.borrow());
                    out.write(format!("{}\n", visible(&shown)).as_bytes());
                }
                true
            }
            Outcome::Done(Err(fault)) => {
                if !self.replaying {
                    let tables = program.tables.borrow();
                    let reason = render::fault(&fault.reason, &tables);
                    let namespace = &tables.namespaces[fault.namespace];
                    if let Some((path, source)) = self.source_of(namespace) {
                        let prefix = format!("fault at {path}:");
                        located(&prefix, source, fault.span, &reason, out);
                    } else {
                        self.fault(&fault, &reason, report.offset, text, out);
                    }
                }
                if let Some(withdrawn) =
                    report.accepted.and_then(|line| self.session.withdraw(line))
                {
                    self.machine.withdraw(&withdrawn);
                    let (files, paths) = core::mem::take(&mut self.before);
                    self.files.truncate(files);
                    for load in &self.paths {
                        if !paths.contains(load) {
                            self.withdrawn_loads
                                .retain(|(name, _)| *name != load.0);
                            self.withdrawn_loads.push(load.clone());
                        }
                    }
                    self.paths = paths;
                }
                false
            }
            Outcome::Request(_) => unreachable!("every request was answered"),
        }
    }

    /// `fault at L:C: reason` in the line, or in the earlier accepted
    /// line the span falls in, with its source line and a caret.
    fn fault(
        &self,
        fault: &Fault,
        reason: &str,
        offset: usize,
        text: &str,
        out: &mut Outs,
    ) {
        let Span { start, end } = fault.span;
        if start >= offset && end - offset <= text.len() {
            let span = Span {
                start: start - offset,
                end: end - offset,
            };
            return located("fault at ", text, span, reason, out);
        }
        let source = self.session.source();
        if end > source.len() {
            return out
                .write(format!("fault: {}\n", visible(reason)).as_bytes());
        }
        let before = &source[..start];
        let number = before.matches("\n\n").count() + 1;
        let first = before.rfind("\n\n").map_or(0, |at| at + 2);
        let last = source[first..]
            .find("\n\n")
            .map_or(source.len(), |at| first + at);
        let span = Span {
            start: start - first,
            end: end.min(last) - first,
        };
        let prefix = format!("fault at line {number}, ");
        located(&prefix, &source[first..last], span, reason, out);
    }

    fn diagnostic(
        &self,
        diagnostic: &Diagnostic,
        report: &Report,
        line: &str,
        out: &mut Outs,
    ) {
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
                let mut reason = render::resolve(&error.reason, &at);
                let word = line.get(error.span.start..error.span.end);
                if matches!(
                    error.reason,
                    fipe::resolve::Reason::UnknownName
                        | fipe::resolve::Reason::UnknownType
                ) && word.is_some_and(|word| self.session.withdrawn(word))
                {
                    let word = word.unwrap_or_default();
                    reason = format!(
                        "{reason}: the line that bound `{word}` was withdrawn when its run Faulted; give it again"
                    );
                }
                located("", line, error.span, &reason, out);
            }
            Diagnostic::Type(error) => {
                let mut reason = render::infer(&error.reason);
                if matches!(error.reason, fipe::infer::Reason::Mismatch { .. })
                    && unbound_word(report, error.span)
                {
                    let word = line
                        .get(error.span.start..error.span.end)
                        .unwrap_or_default();
                    reason.push_str(&format!(
                        "; `{word}` is text here: no name `{word}` is bound"
                    ));
                }
                if error.reason == fipe::infer::Reason::LiteralApplied {
                    let word = line
                        .get(error.span.start..error.span.end)
                        .unwrap_or_default();
                    reason = format!("{reason}: `{word}` is an operand here");
                    if !word.is_empty()
                        && !word.chars().any(char::is_alphanumeric)
                    {
                        reason.push_str(
                            ", not an operator; the arithmetic operators are `+ - * / %`",
                        );
                    }
                }
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
            Diagnostic::Compile { span, what, reason } => {
                located("", line, *span, &render::compile(what, reason), out);
            }
        }
    }

    /// The path and text of a source other than the session's, by the
    /// namespace a Fault names: the prelude, a host declaration file or
    /// a loaded file module. `None` for the session's lines.
    fn source_of(&self, namespace: &str) -> Option<(String, &str)> {
        if namespace == "main" || namespace.starts_with("line ") {
            return None;
        }
        if namespace == "prelude" {
            return Some((
                String::from("prelude.fipe"),
                fipe::decl::prelude_source(),
            ));
        }
        if let Some(index) = HOST.iter().position(|name| *name == namespace) {
            let text = self.host_texts.get(index)?;
            return Some((host_path(&self.host_dir, namespace), text));
        }
        let text = self.cached(namespace)?;
        Some((self.path(namespace), text))
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
        self.host_texts = texts.to_vec();
        self.host_dir = String::from(dir);
        self.session = Session::new(declarations);
        self.session.strict(self.strict);
        self.session.compile_budget(self.compile_budget);
        self.last = None;
        // New tables, so a new machine; the replayed lines rebuild its
        // globals without printing again.
        self.machine = Machine::new(self.machine.budget());
        self.replaying = true;
        for line in lines {
            let report = self.replay(&line);
            self.done(report, line, Mode::Accept, out);
        }
        self.replaying = false;
        out.write(format!("host: {dir}\n").as_bytes());
    }

    /// Accept a line again, answering its `use` requests from the cache.
    fn replay(&mut self, line: &str) -> Report {
        self.before = (self.files.len(), self.paths.clone());
        let mut outcome = self.session.line(line);
        loop {
            match outcome {
                Outcome::Request(Request::File { stem }) => {
                    let text = self.cached(&stem).map(String::from);
                    outcome = self.session.reply(Reply::File(text));
                }
                Outcome::Request(Request::Call { .. }) => {
                    unreachable!("a session asks only for files")
                }
                Outcome::Done(report) => return report,
            }
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
    let marked = text.get(at..span.end.clamp(at, end)).unwrap_or_default();
    let width = visible(marked).chars().count().max(1);
    let row = visible(&text[start..end]);
    let padding = text
        .get(start..at)
        .map_or(0, |text| visible(text).chars().count());
    let prefix = visible(prefix);
    let reason = visible(reason);
    // Padding by repetition: a format width panics above 65535.
    let mut rendered = format!("{prefix}{line}:{col}: {reason}\n{row}\n");
    rendered.push_str(&" ".repeat(padding));
    rendered.push_str(&"^".repeat(width));
    rendered.push('\n');
    out.write(rendered.as_bytes());
}

/// Keep source line breaks; escape controls that can alter a terminal.
fn visible(text: &str) -> String {
    let mut rendered = String::new();
    for ch in text.chars() {
        if ch.is_control() && ch != '\n' {
            rendered.extend(ch.escape_default());
        } else {
            rendered.push(ch);
        }
    }
    rendered
}

/// Each name expression of the line: line:col and target, with
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
    if names.is_empty() {
        out.write(
            b"no names to resolve: operators and literals name nothing\n",
        );
    }
    for (start, _, name, target) in names {
        let (l, c) = Source::new(line).line_col(start);
        let at = Context {
            source: session.source(),
            line,
        };
        let target = render::target(target, &at);
        let text = format!("{l}:{c} {name}: {target}\n");
        out.write(text.as_bytes());
    }
}

/// Whether the line's expression at `span` (in the line's coordinates)
/// is an unbound name in an argument position, which is text.
fn unbound_word(report: &Report, span: Span) -> bool {
    let (Some(module), Some(resolved)) = (&report.module, &report.resolved)
    else {
        return false;
    };
    resolved.expressions().any(|(id, target)| {
        matches!(target, fipe::resolve::Target::Word)
            && module.expr(id).is_some_and(|expr| {
                expr.span.start.checked_sub(report.offset) == Some(span.start)
                    && expr.span.end - report.offset == span.end
            })
    })
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
                visible(source)
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
            out.write(visible(doc).as_bytes());
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
            if known.phase != fipe::typed::Phase::Eval {
                out.write(format!("known at {}\n", known.phase).as_bytes());
            }
            if let Some(holding) = &known.holding {
                out.write(format!("{holding}\n").as_bytes());
            }
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
        | Diagnostic::Type(_)
        | Diagnostic::Compile { .. } => false,
    }
}

/// A comment-only line, which is blank to the grammar.
fn comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
        && (!line.trim_start().starts_with("##")
            || line.trim_start().starts_with("###"))
}

/// A command entry, which runs once what it opened is closed.
fn command_entry(entry: &str) -> bool {
    entry.trim_start().starts_with(':')
}

/// What keeps a blank line inside an entry: an unclosed bracket, a
/// quote across lines, or a doc comment waiting for its item.
#[derive(Default)]
struct Continuation {
    open: bool,
    depth: isize,
    quote: Option<char>,
    doc: bool,
}

fn code(entry: &str) -> &str {
    entry
        .trim_start()
        .strip_prefix(':')
        .map_or(entry, |command| {
            command
                .split_once(char::is_whitespace)
                .map_or("", |(_, arg)| arg)
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
        depth: 0,
    };
    if result.open || !lexed.errors.is_empty() {
        return result;
    }
    let mut depth = 0_isize;
    for token in tokens {
        match token.kind {
            Kind::Symbol(
                Symbol::LParen | Symbol::LBracket | Symbol::LBrace,
            ) => {
                depth += 1;
            }
            Kind::Symbol(
                Symbol::RParen | Symbol::RBracket | Symbol::RBrace,
            ) => {
                depth -= 1;
            }
            _ => {}
        }
    }
    result.doc = depth > 0
        && tokens
            .iter()
            .rev()
            .find(|token| !matches!(token.kind, Kind::Newline | Kind::Eof))
            .is_some_and(|token| {
                matches!(token.kind, Kind::Doc(_) | Kind::InnerDoc(_))
            });
    result.depth = depth;
    result.open = depth > 0;
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

/// The Name part of a file stem: from its first letter or `_`, with
/// every other character that a Name cannot hold as `_`.
fn module_name(file: &str) -> String {
    file.trim_start_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '_')
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}
