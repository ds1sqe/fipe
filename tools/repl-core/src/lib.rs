//! The Fipe REPL without I/O: bytes in, bytes and file requests out.
//!
//! A driver passes input to [`Repl::feed`] and acts on each [`Out`] in
//! order: it writes [`Out::Write`], answers [`Out::File`] with [`Repl::file`]
//! using its request identity and path, answers a host call ([`Out::Call`],
//! when it hosts the session) with [`Repl::answer`] using its request identity,
//! acknowledges [`Out::Start`], [`Out::End`] and [`Out::Reset`] with [`Repl::resume`] after its
//! boundary actions, and stops at [`Out::Exit`]. Lines end at LF; at end of input the driver calls
//! [`Repl::flush`] so a last line without LF runs, drains it, then calls
//! [`Repl::close`] and handles its gates through Exit before disposal.
//! The prompt comes out whenever the input holds no partial line and
//! nothing is awaited, so `feed(&[])` yields the first one; none comes
//! after `flush`. At a terminal a driver edits each line with
//! [`edit`] before feeding it, and [`Repl::prompt`] names the prompt
//! the line is drawn after.
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
//! `:ir [line]`, `:mir [line]`, `:load <stem or path>`, `:checks [name]`,
//! `:host [dir]`, `:help [name]`, `:q`; any other line goes to the
//! session. `:help` lists them, one line each.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod complete;
pub mod edit;
pub mod highlight;
mod ir;
mod mir;
mod render;
mod tree;

use alloc::{
    boxed::Box,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;
pub use fipe::check::COMPILE_BUDGET;
use fipe::{
    ast::{Entry, ExprKind},
    decl::{self, Declarations},
    embed::{Outcome, Reply, Request},
    ir::Tables,
    lex::{lex, Lexed},
    mir::Program,
    session::{Diagnostic, Report, Session},
    token::{Kind, Source, Span, Symbol, Token},
    value::Value,
    vm::{Fault, Machine},
};
use render::Context;

const PROMPT: &str = "fipe> ";
/// The REPL's own budget per line when the driver names none: a surface
/// constant, not the language's.
pub const BUDGET: u64 = 1 << 20;
const MORE: &str = "....> ";
const HOST_DIR: &str = "tests/fixtures/host";
/// The commands: how each is written, and what it does.
const COMMANDS: [(&str, &str); 10] = [
    (":ast", "the syntax tree of the last line that parsed"),
    (
        ":r [line]",
        "what each name of the line, or the last one, refers to",
    ),
    (
        ":t [line]",
        "the types of the line, or of the last accepted one",
    ),
    (
        ":ir [line]",
        "the checked unit of the line, or of the last one",
    ),
    (
        ":mir [line]",
        "the machine program of the line, or of the last one",
    ),
    (":load <stem or path>", "use the module of a file"),
    (
        ":checks [name]",
        "the session's checks, or those of a binding",
    ),
    (
        ":host [dir]",
        "load host declarations, tests/fixtures/host if none",
    ),
    (":help [name]", "this list, or a name's doc and type"),
    (":q", "quit"),
];
/// The command namespace first, then the host modules by name.
const HOST: [&str; 6] = ["commands", "fs", "sys", "cli", "text", "config"];

/// The identity of one pending file request or host call.
///
/// Cloning preserves identity; only the REPL can create a fresh request.
#[derive(Clone)]
pub struct RequestId(Rc<()>);

impl RequestId {
    fn new() -> Self {
        Self(Rc::new(()))
    }
}

impl PartialEq for RequestId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RequestId {}

impl core::fmt::Debug for RequestId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RequestId(..)")
    }
}

/// The identity of one ordinary executable unit.
///
/// Cloning preserves identity; only the REPL creates a fresh identity.
#[derive(Clone)]
pub struct RunId(Rc<()>);

impl RunId {
    fn new() -> Self {
        Self(Rc::new(()))
    }
}
impl PartialEq for RunId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RunId {}
impl core::fmt::Debug for RunId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RunId(..)")
    }
}

/// The identity of one pending execution boundary.
///
/// Cloning preserves identity; only the REPL creates a fresh identity.
#[derive(Clone)]
pub struct ResumeId(Rc<()>);

impl ResumeId {
    fn new() -> Self {
        Self(Rc::new(()))
    }
}
impl PartialEq for ResumeId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ResumeId {}
impl core::fmt::Debug for ResumeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResumeId(..)")
    }
}

/// The actual VM terminal, before presentation and language withdrawal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEnd {
    /// VM success, including a language-level `Err` value.
    Success,
    /// A source-located VM failure.
    Fault(Fault),
    /// The driver stopped an unfinished run; prior effects are not rolled back.
    Abandoned,
}

/// Why the driver must dispose its session owner before resuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    HostReplacement,
    Exit,
}

/// A driver operation refused without changing the pending request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveError {
    /// No request of this kind, identity and path is pending.
    WrongRequest,
    /// No boundary with this continuation identity is pending.
    WrongContinuation,
    /// A request or boundary must be handled before this operation.
    Busy,
    /// The instance has completed its exit protocol.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Write(Vec<u8>),
    /// Runtime execution waits for the driver to acknowledge this start.
    Start {
        run: RunId,
        resume: ResumeId,
    },
    /// The driver handles this terminal before acknowledging presentation/next input.
    End {
        run: RunId,
        result: RunEnd,
        resume: ResumeId,
    },
    /// The driver reads this path and answers with [`Repl::file`].
    File {
        id: RequestId,
        path: String,
    },
    /// A host call of the line's run, which the driver answers with
    /// [`Repl::answer`]; only when it hosts the session
    /// ([`Repl::hosted`]).
    Call {
        id: RequestId,
    },
    /// Dispose the old session owner before acknowledging this reset.
    Reset {
        reason: ResetReason,
        resume: ResumeId,
    },
    Exit,
}

// The flags are independent session facts, not one state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct Repl {
    session: Session,
    /// The session's declarations, for completion.
    declarations: Declarations,
    strict: bool,
    /// The host's compile budget, kept across a host reload.
    compile_budget: u64,
    input: Vec<u8>,
    /// The entry's lines so far.
    entry: String,
    continuation: Continuation,
    wait: Option<Wait>,
    /// Whether the driver answers host calls; otherwise each Faults.
    hosted: bool,
    /// Replied file texts by stem, for replays and file diagnostics.
    files: Vec<(String, String)>,
    /// Declaration sources by namespace, for diagnostics and docs.
    host_texts: Vec<(String, String)>,
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
    replay: Option<Replay>,
    closing: bool,
    quit: bool,
    ended: bool,
}

struct Replacement {
    declarations: Declarations,
    texts: Vec<(String, String)>,
    dir: String,
    lines: Vec<String>,
}
struct Replay {
    lines: Vec<String>,
    next: usize,
}
enum Reset {
    Host(Box<Replacement>),
    Exit,
}
enum Terminal {
    Vm(Result<Value, Fault>),
    Abandoned,
}
enum Wait {
    Reset {
        resume: ResumeId,
        action: Reset,
    },
    Start {
        resume: ResumeId,
        execution: Box<Execution>,
    },
    End {
        resume: ResumeId,
        execution: Box<Execution>,
        result: Terminal,
    },
    Line {
        id: RequestId,
        stem: String,
        text: String,
        mode: Mode,
    },
    Host {
        id: RequestId,
        dir: String,
        texts: Vec<String>,
    },
    /// A line's run waiting on the driver's answer to a host call.
    Call(Box<Pending>),
}

/// A line's run stopped at a host call.
struct Pending {
    id: RequestId,
    request: Request,
    execution: Execution,
}

/// One execution moves through its boundaries and host calls without copying source.
struct Execution {
    run: RunId,
    program: Program,
    report: Report,
    text: String,
}

#[derive(Clone, Copy)]
enum Mode {
    Accept,
    /// `:load`'s `use`, accepted like a line but never continued.
    Load,
    Resolve,
    Types,
    /// `:help name`: as `:t`, but an unknown name is one line.
    Help,
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
            declarations: Declarations::new(),
            strict: false,
            compile_budget: COMPILE_BUDGET,
            input: Vec::new(),
            entry: String::new(),
            continuation: Continuation::default(),
            wait: None,
            hosted: false,
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
            replay: None,
            closing: false,
            quit: false,
            ended: false,
        }
    }

    /// The charge budget each accepted line runs under.
    pub const fn budget(&mut self, budget: u64) {
        self.machine.set_budget(Some(budget));
    }

    /// Whether the driver answers host calls: each comes out as
    /// [`Out::Call`] and waits for [`Repl::answer`]. Otherwise, and
    /// while accepted lines replay after a host reload, a call Faults
    /// `no host for <namespace>.<name>`.
    pub const fn hosted(&mut self, hosted: bool) {
        self.hosted = hosted;
    }

    /// Reject open implicit checks, including after a host reload.
    pub const fn strict(&mut self, enabled: bool) {
        self.strict = enabled;
        self.session.strict(enabled);
    }

    /// The charge units each compile-time evaluation may spend,
    /// including after a host reload.
    pub const fn compile_budget(&mut self, units: u64) {
        self.compile_budget = units;
        self.session.compile_budget(units);
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Out> {
        if self.quit {
            return Vec::new();
        }
        self.input.extend_from_slice(bytes);
        let mut out = Outs::default();
        self.run(&mut out);
        self.finish(out)
    }

    /// Submit trailing input, after an LF that ends the prompt's line.
    ///
    /// This drains input, not ownership: drain the outputs then call close at EOF.
    pub fn flush(&mut self) -> Vec<Out> {
        if self.quit {
            return Vec::new();
        }
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

    /// The prompt for the next line: `....> ` while an entry is held.
    #[must_use]
    pub const fn prompt(&self) -> &'static str {
        if self.entry.is_empty() {
            PROMPT
        } else {
            MORE
        }
    }

    /// Drop input and abandon an unfinished run, preserving computed terminals.
    ///
    /// Withdrawal follows acknowledgement of the resulting Abandoned End.
    ///
    /// # Panics
    /// If an internal pending Call has no active machine execution.
    pub fn interrupt(&mut self) -> Vec<Out> {
        if self.quit {
            return Vec::new();
        }
        self.clear_input();
        let mut out = Outs::default();
        self.abandon(&mut out);
        self.run(&mut out);
        self.finish(out)
    }

    /// Dispose explicitly: acknowledge any returned End, then Reset, before drop.
    ///
    /// # Errors
    /// Busy preserves an already computed End or Reset, including its input/intent.
    /// Drain that boundary and retry. Repeated close after Exit is inert.
    ///
    /// # Panics
    /// If an internal pending Call has no active machine execution.
    pub fn close(&mut self) -> Result<Vec<Out>, DriveError> {
        if self.quit {
            return Ok(Vec::new());
        }
        if matches!(self.wait, Some(Wait::End { .. } | Wait::Reset { .. })) {
            return Err(DriveError::Busy);
        }
        self.clear_input();
        self.closing = true;
        let mut out = Outs::default();
        self.abandon(&mut out);
        self.run(&mut out);
        Ok(self.finish(out))
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.entry.clear();
        self.continuation = Continuation::default();
    }

    fn abandon(&mut self, out: &mut Outs) {
        // Taking the Wait invalidates the driver's association before VM work.
        let execution = match self.wait.take() {
            Some(Wait::Start { execution, .. }) => Some(*execution),
            Some(Wait::Call(pending)) => {
                assert!(
                    self.machine.abandon(),
                    "a pending call has active execution"
                );
                Some(pending.execution)
            }
            Some(wait @ (Wait::End { .. } | Wait::Reset { .. })) => {
                self.wait = Some(wait);
                return;
            }
            Some(Wait::Line { .. } | Wait::Host { .. }) | None => None,
        };
        if let Some(replay) = &mut self.replay {
            replay.lines.truncate(replay.next);
        }
        if let Some(execution) = execution {
            self.terminal(execution, Terminal::Abandoned, out);
        }
    }

    fn reset(&mut self, action: Reset, out: &mut Outs) {
        let reason = match &action {
            Reset::Host(_) => ResetReason::HostReplacement,
            Reset::Exit => ResetReason::Exit,
        };
        let resume = ResumeId::new();
        out.0.push(Out::Reset {
            reason,
            resume: resume.clone(),
        });
        self.wait = Some(Wait::Reset { resume, action });
    }

    fn finish(&mut self, out: Outs) -> Vec<Out> {
        if let Some(Out::Write(bytes)) = out.0.last() {
            self.prompted = bytes.ends_with(PROMPT.as_bytes())
                || bytes.ends_with(MORE.as_bytes());
        }
        out.0
    }

    /// The answer to the last [`Out::File`]: its text, or `None` when
    /// the path does not exist.
    ///
    /// # Errors
    /// [`DriveError::WrongRequest`] unless this identity and path are pending.
    pub fn file(
        &mut self,
        id: &RequestId,
        path: &str,
        text: Option<&str>,
    ) -> Result<Vec<Out>, DriveError> {
        let current = match &self.wait {
            Some(Wait::Line {
                id: pending, stem, ..
            }) => pending == id && self.path(stem) == path,
            Some(Wait::Host {
                id: pending,
                dir,
                texts,
            }) => {
                pending == id
                    && HOST
                        .get(texts.len())
                        .is_some_and(|name| host_path(dir, name) == path)
            }
            _ => false,
        };
        if !current {
            return Err(DriveError::WrongRequest);
        }
        let mut out = Outs::default();
        match self.wait.take() {
            Some(Wait::Line {
                stem,
                text: line,
                mode,
                ..
            }) => {
                if let Some(text) = text {
                    self.withdrawn_loads.retain(|(name, _)| *name != stem);
                    self.files.push((stem, text.to_string()));
                } else if let Some((name, path)) =
                    self.withdrawn_loads.iter().find(|(name, _)| *name == stem)
                {
                    out.write(
                        format!(
                            "`{name}`: its `:load` of {path} was withdrawn; load it again with `:load {path}`\n"
                        )
                        .as_bytes(),
                    );
                }
                let reply = Reply::File(text.map(String::from));
                let outcome = self.session.reply(reply);
                self.step(line, mode, outcome, &mut out);
            }
            Some(Wait::Host { dir, mut texts, .. }) => match text {
                None => out.write(format!("{path}: not found\n").as_bytes()),
                Some(text) => {
                    texts.push(text.to_string());
                    if let Some(name) = HOST.get(texts.len()) {
                        let id = RequestId::new();
                        out.0.push(Out::File {
                            id: id.clone(),
                            path: host_path(&dir, name),
                        });
                        self.wait = Some(Wait::Host { id, dir, texts });
                    } else {
                        let sources: Vec<_> = HOST
                            .iter()
                            .zip(&texts)
                            .map(|(name, text)| (*name, text.as_str()))
                            .collect();
                        self.host(&dir, &sources, &mut out);
                    }
                }
            },
            _ => unreachable!("the file request was checked before taking it"),
        }
        self.run(&mut out);
        Ok(self.finish(out))
    }

    /// The answer to the last [`Out::Call`]: `answer` gets the request
    /// and the run's tables and gives the reply, after which the run
    /// goes on.
    ///
    /// # Errors
    /// [`DriveError::WrongRequest`] unless this host call is pending. The
    /// callback is never invoked for a stale or wrong-kind identity.
    pub fn answer(
        &mut self,
        id: &RequestId,
        answer: impl FnOnce(&Request, &Tables) -> Reply,
    ) -> Result<Vec<Out>, DriveError> {
        if !matches!(&self.wait, Some(Wait::Call(pending)) if pending.id == *id)
        {
            return Err(DriveError::WrongRequest);
        }
        let mut out = Outs::default();
        match self.wait.take() {
            Some(Wait::Call(pending)) => {
                let Pending {
                    request, execution, ..
                } = *pending;
                let reply =
                    answer(&request, &execution.program.tables.borrow());
                let outcome = self.machine.reply(&execution.program, reply);
                self.settle(execution, outcome, &mut out);
            }
            _ => unreachable!("the host call was checked before taking it"),
        }
        self.run(&mut out);
        Ok(self.finish(out))
    }

    /// Acknowledge a runtime boundary or session reset before the pump advances.
    ///
    /// # Errors
    /// [`DriveError::WrongContinuation`] for a stale, foreign or wrong-stage ID;
    /// the pending boundary is unchanged.
    pub fn resume(&mut self, id: &ResumeId) -> Result<Vec<Out>, DriveError> {
        if !matches!(&self.wait,
            Some(Wait::Start { resume, .. } | Wait::End { resume, .. } | Wait::Reset { resume, .. }) if resume == id)
        {
            return Err(DriveError::WrongContinuation);
        }
        let mut out = Outs::default();
        match self.wait.take() {
            Some(Wait::Start { execution, .. }) => {
                let outcome = self.machine.run(&execution.program);
                self.settle(*execution, outcome, &mut out);
            }
            Some(Wait::End {
                execution, result, ..
            }) => {
                self.complete_execution(*execution, result, &mut out);
            }
            Some(Wait::Reset { action, .. }) => match action {
                Reset::Host(replacement) => self.install(*replacement),
                Reset::Exit => {
                    // Owner disposal was acknowledged; release all old language aliases.
                    *self = Self::new();
                    self.quit = true;
                    out.0.push(Out::Exit);
                }
            },
            _ => unreachable!("the continuation was checked before taking it"),
        }
        self.run(&mut out);
        Ok(self.finish(out))
    }

    fn run(&mut self, out: &mut Outs) {
        while !self.quit && self.wait.is_none() {
            if self.closing {
                self.reset(Reset::Exit, out);
                break;
            }
            if let Some(replay) = &mut self.replay {
                if let Some(line) = replay.lines.get(replay.next).cloned() {
                    replay.next += 1;
                    let report = self.replay(&line);
                    self.done(report, line, Mode::Accept, out);
                } else {
                    self.replay = None;
                    out.write(format!("host: {}\n", self.host_dir).as_bytes());
                }
                continue;
            }
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
                self.continuation = continuation(code(line), &lex(code(line)));
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
            out.write(self.prompt().as_bytes());
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
            self.continuation = continuation(line, &lex(line));
            self.continuation.depth += depth;
            self.continuation.open |= self.continuation.depth > 0;
        } else if self.continuation.quote != Some('\'') || line.contains('\'') {
            self.continuation =
                continuation(code(&self.entry), &lex(code(&self.entry)));
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
                self.clear_input();
                self.closing = true;
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
            ("help", arg) => self.help(arg, out),
            _ => out.write(format!("unknown command :{name}\n").as_bytes()),
        }
    }

    /// `:help`: the commands, one line each; `:help :cmd` that command's
    /// line; `:help name` the name's doc and type.
    fn help(&mut self, arg: &str, out: &mut Outs) {
        let width = COMMANDS.iter().map(|(usage, _)| usage.len()).max();
        let line = |(usage, what): &(&str, &str)| {
            let padding = " ".repeat(width.unwrap_or_default() - usage.len());
            format!("{usage}{padding}  {what}\n")
        };
        if arg.is_empty() {
            for command in &COMMANDS {
                out.write(line(command).as_bytes());
            }
        } else if let Some(name) = arg.strip_prefix(':') {
            let found = COMMANDS
                .iter()
                .find(|(usage, _)| usage[1..].split(' ').next() == Some(name));
            match found {
                Some(command) => out.write(line(command).as_bytes()),
                None => {
                    out.write(format!("unknown command :{name}\n").as_bytes());
                }
            }
        } else if is_name(arg) {
            self.start(arg, Mode::Help, out);
        } else {
            out.write(b"usage: :help [name or :command]\n");
        }
    }

    /// Load host declarations by namespace, with `commands` first when
    /// present. Dotted module names create submodules. `dir` names where
    /// the sources come from, as a Fault in one is located.
    ///
    /// # Errors
    /// [`DriveError::Busy`] while a request, boundary or replay is pending;
    /// [`DriveError::Closed`] after Exit.
    pub fn host_sources(
        &mut self,
        dir: &str,
        sources: &[(&str, &str)],
    ) -> Result<Vec<Out>, DriveError> {
        if self.quit {
            return Err(DriveError::Closed);
        }
        if self.wait.is_some() || self.replay.is_some() {
            return Err(DriveError::Busy);
        }
        let mut out = Outs::default();
        self.host(dir, sources, &mut out);
        self.run(&mut out);
        Ok(self.finish(out))
    }

    /// Load host declarations from a directory without interpreting it as input.
    ///
    /// # Errors
    /// [`DriveError::Busy`] while a request, boundary or replay is pending;
    /// [`DriveError::Closed`] after Exit.
    pub fn host_directory(
        &mut self,
        dir: &str,
    ) -> Result<Vec<Out>, DriveError> {
        if self.quit {
            return Err(DriveError::Closed);
        }
        if self.wait.is_some() || self.replay.is_some() {
            return Err(DriveError::Busy);
        }
        let mut out = Outs::default();
        self.request_host(dir, &mut out);
        Ok(self.finish(out))
    }

    fn request_host(&mut self, dir: &str, out: &mut Outs) {
        let dir = if dir.is_empty() { HOST_DIR } else { dir };
        if dir.contains('\0') {
            out.write(b"host path contains a NUL character\n");
            return;
        }
        let id = RequestId::new();
        out.0.push(Out::File {
            id: id.clone(),
            path: host_path(dir, HOST[0]),
        });
        self.wait = Some(Wait::Host {
            id,
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
                    _ => sources
                        .range(..=check.place.start)
                        .next_back()
                        .map_or_else(
                            || (self.session.source(), String::new(), 0),
                            |(offset, text)| (*text, String::new(), *offset),
                        ),
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
            Origin::Prelude => fipe::decl::prelude_source(),
            Origin::Commands => self
                .host_texts
                .iter()
                .find(|(name, _)| name == "commands")?
                .1
                .as_str(),
            Origin::Host(name) => {
                self.host_texts.iter().find(|(n, _)| n == name)?.1.as_str()
            }
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
            Mode::Resolve | Mode::Types | Mode::Help | Mode::Ir | Mode::Mir => {
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
                let id = RequestId::new();
                out.0.push(Out::File {
                    id: id.clone(),
                    path: self.path(&stem),
                });
                self.wait = Some(Wait::Line {
                    id,
                    stem,
                    text,
                    mode,
                });
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
        if matches!(mode, Mode::Help) && report.diagnostics.iter().any(unknown)
        {
            let text = format!("no name `{text}` in scope\n");
            return out.write(text.as_bytes());
        }
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
                if report.diagnostics.iter().all(from_file) {
                    self.execute(report, text, out);
                }
            }
            Mode::Resolve => names(&self.session, &report, &text, out),
            Mode::Types | Mode::Help => {
                types(&self.session, &report, &text, out);
            }
            Mode::Ir => ir::render(report.unit.as_ref(), out),
            Mode::Mir => mir::render(report.unit.as_ref(), out),
        }
    }

    /// Run an accepted line on the machine: its value in the language's
    /// spelling, nothing for `()`, or the Fault where it happened, after
    /// which the line is withdrawn. A line that stands is the last.
    fn execute(&mut self, report: Report, text: String, out: &mut Outs) {
        let Some(unit) = &report.unit else {
            self.last = Some((report, text));
            return;
        };
        let program = fipe::compile::compile(unit);
        let execution = Execution {
            run: RunId::new(),
            program,
            report,
            text,
        };
        let resume = ResumeId::new();
        out.0.push(Out::Start {
            run: execution.run.clone(),
            resume: resume.clone(),
        });
        self.wait = Some(Wait::Start {
            resume,
            execution: Box::new(execution),
        });
    }

    /// Carry a line's run to its end: each host call goes to the driver
    /// when it hosts the session, the run then waiting on its answer;
    /// otherwise the call fails, naming its declaration, and the run
    /// Faults there.
    fn settle(
        &mut self,
        execution: Execution,
        mut outcome: Outcome<Result<Value, Fault>>,
        out: &mut Outs,
    ) {
        loop {
            let request = match outcome {
                Outcome::Request(request) => request,
                Outcome::Done(result) => {
                    self.terminal(execution, Terminal::Vm(result), out);
                    return;
                }
            };
            let Request::Call { declaration, .. } = &request else {
                unreachable!("a run asks only for host calls")
            };
            if self.hosted && self.replay.is_none() {
                let id = RequestId::new();
                out.0.push(Out::Call { id: id.clone() });
                self.wait = Some(Wait::Call(Box::new(Pending {
                    id,
                    request,
                    execution,
                })));
                return;
            }
            let message = {
                let tables = execution.program.tables.borrow();
                let declaration = &tables.declarations[*declaration];
                format!(
                    "no host for {}.{}",
                    tables.namespaces[declaration.namespace], declaration.label
                )
            };
            outcome = self
                .machine
                .reply(&execution.program, Reply::Value(Err(message)));
        }
    }

    fn terminal(
        &mut self,
        execution: Execution,
        result: Terminal,
        out: &mut Outs,
    ) {
        let terminal = match &result {
            Terminal::Vm(Ok(_)) => RunEnd::Success,
            Terminal::Vm(Err(fault)) => RunEnd::Fault(fault.clone()),
            Terminal::Abandoned => RunEnd::Abandoned,
        };
        let resume = ResumeId::new();
        out.0.push(Out::End {
            run: execution.run.clone(),
            result: terminal,
            resume: resume.clone(),
        });
        self.wait = Some(Wait::End {
            resume,
            execution: Box::new(execution),
            result,
        });
    }

    fn complete_execution(
        &mut self,
        execution: Execution,
        result: Terminal,
        out: &mut Outs,
    ) {
        match result {
            Terminal::Vm(result) => {
                if self.ended(
                    &execution.program,
                    &execution.report,
                    &execution.text,
                    result,
                    out,
                ) {
                    self.last = Some((execution.report, execution.text));
                }
            }
            Terminal::Abandoned => self.withdraw(&execution.report),
        }
    }

    /// A run's end: the value written, or the Fault and the line
    /// withdrawn. Whether the line stands.
    fn ended(
        &mut self,
        program: &Program,
        report: &Report,
        text: &str,
        result: Result<Value, Fault>,
        out: &mut Outs,
    ) -> bool {
        match result {
            Ok(value) => {
                if self.replay.is_none() && !matches!(value, Value::Unit) {
                    let shown = render::value(&value, &program.tables.borrow());
                    out.write(format!("{}\n", visible(&shown)).as_bytes());
                }
                true
            }
            Err(fault) => {
                if self.replay.is_none() {
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
                self.withdraw(report);
                false
            }
        }
    }

    /// `fault at L:C: reason` in the line, or in the earlier accepted
    /// line the span falls in, with its source line and a caret.
    fn withdraw(&mut self, report: &Report) {
        if let Some(withdrawn) =
            report.accepted.and_then(|line| self.session.withdraw(line))
        {
            self.machine.withdraw(&withdrawn);
            let (files, paths) = core::mem::take(&mut self.before);
            self.files.truncate(files);
            for load in &self.paths {
                if !paths.contains(load) {
                    self.withdrawn_loads.retain(|(name, _)| *name != load.0);
                    self.withdrawn_loads.push(load.clone());
                }
            }
            self.paths = paths;
        }
    }

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
                        "{reason}: the line that bound `{word}` was withdrawn; give it again"
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
                    let _ = write!(
                        reason,
                        "; `{word}` is text here: no name `{word}` is bound"
                    );
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
        if let Some((_, text)) =
            self.host_texts.iter().find(|(name, _)| name == namespace)
        {
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
    fn host(&mut self, dir: &str, texts: &[(&str, &str)], out: &mut Outs) {
        let mut declarations = Declarations::new();
        let mut failed = false;
        for (name, text) in texts {
            let result = if *name == "commands" {
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
        let replacement = Replacement {
            declarations,
            texts: texts
                .iter()
                .map(|(name, text)| (String::from(*name), String::from(*text)))
                .collect(),
            dir: String::from(dir),
            lines: self.session.lines().map(String::from).collect(),
        };
        self.reset(Reset::Host(Box::new(replacement)), out);
    }

    fn install(&mut self, replacement: Replacement) {
        self.host_texts = replacement.texts;
        self.host_dir = replacement.dir;
        self.declarations = replacement.declarations.clone();
        self.session = Session::new(replacement.declarations);
        self.session.strict(self.strict);
        self.session.compile_budget(self.compile_budget);
        self.last = None;
        self.parsed = None;
        self.machine = Machine::new(self.machine.budget());
        self.replay = Some(Replay {
            lines: replacement.lines,
            next: 0,
        });
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

/// An unknown name or type, which `:help` says in one line.
const fn unknown(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic,
        Diagnostic::Resolve(fipe::resolve::Error {
            reason: fipe::resolve::Reason::UnknownName
                | fipe::resolve::Reason::UnknownNameIn { .. }
                | fipe::resolve::Reason::UnknownType,
            ..
        })
    )
}

/// A qualified name: Name segments joined by dots.
fn is_name(text: &str) -> bool {
    text.split('.').all(|segment| {
        segment.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
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

fn continuation(code: &str, lexed: &Lexed) -> Continuation {
    let quote = open_quote(code, lexed);
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
