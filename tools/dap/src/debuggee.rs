//! A launched program under the adapter: its machine, the stops it
//! takes, breakpoints, stepping and what each stop shows.
//!
//! Every stop is one the VM took (`Machine::next_stop`); the adapter
//! only decides which of them the client sees.
use crate::{
    inspect::{self, Item, References},
    lines::{self, Files, Pipe},
    protocol::Outgoing,
    replay::Replay,
};
use fipe::{
    embed::{Reply, Request},
    ir::{Kind, Tables},
    mir::Program,
    value::{structural::show, Value},
    vm::{Fault, Frame, Machine, Stop},
};
use serde_json::{json, Value as Json};
use std::{collections::BTreeMap, fmt::Write as _, path::Path};

/// The one thread a run has.
pub const THREAD: u64 = 1;

/// How the run goes on when resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// To a breakpoint or the end; `entry` stops at the first stop.
    Continue { entry: bool },
    /// To the next stage or line in this frame or a caller.
    Next(Place),
    /// As `Next`, or into a callee's body.
    StepIn(Place),
    /// Back into the caller.
    StepOut { depth: usize },
}

/// Where a step started: its frame depth, line and pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    depth: usize,
    line: Option<(usize, usize)>,
    pipe: Option<usize>,
}

/// Where the program is.
#[derive(Debug)]
pub enum Phase {
    /// Launched, not yet configured.
    Idle,
    Running,
    /// At a stop the client sees.
    Stopped,
    /// On a Fault; the machine has ended the run.
    Faulted(Fault),
    Ended,
}

/// Named items a scope shows.
type Named = Vec<(String, Item)>;

/// A breakpoint: on a line, or on the mark of one pipe.
#[derive(Clone, Debug)]
struct Breakpoint {
    id: u64,
    namespace: String,
    line: usize,
    pipe: Option<usize>,
}

/// What the run last did at each pipe: the value it fed the pipe, the
/// value the pipe gave, and whether that value is the latest.
#[derive(Debug, Default)]
struct PipeState {
    input: Option<Value>,
    output: Option<Value>,
    locked: bool,
    done: bool,
}

/// A checked program and its run.
#[derive(Debug)]
pub struct Debuggee {
    program: Program,
    files: Files,
    replay: Replay,
    machine: Machine,
    pipes: Vec<Pipe>,
    pub phase: Phase,
    mode: Mode,
    pause: bool,
    breakpoints: Vec<Breakpoint>,
    next_id: u64,
    /// The trace so far: one line per host call with its reply.
    effects: Vec<String>,
    /// The last value written under each node.
    seen: BTreeMap<usize, Value>,
    states: BTreeMap<usize, PipeState>,
    /// The line and pipe each body frame depth is at, outermost first.
    lines: Vec<Option<(usize, usize)>>,
    at_pipes: Vec<Option<usize>>,
    /// The node of the stop the client sees.
    current: Option<usize>,
    references: References,
}

impl Debuggee {
    /// A program ready to run under a budget, its host calls replayed.
    #[must_use]
    pub fn new(
        program: Program,
        files: Files,
        replay: Replay,
        budget: Option<u64>,
    ) -> Self {
        let pipes = lines::pipes(&program.tables.borrow(), &files);
        Self {
            program,
            files,
            replay,
            machine: Machine::new(budget),
            pipes,
            phase: Phase::Idle,
            mode: Mode::Continue { entry: false },
            pause: false,
            breakpoints: Vec::new(),
            next_id: 0,
            effects: Vec::new(),
            seen: BTreeMap::new(),
            states: BTreeMap::new(),
            lines: Vec::new(),
            at_pipes: Vec::new(),
            current: None,
            references: References::default(),
        }
    }

    /// Start the run: to a breakpoint, or to the first stop when
    /// `entry`.
    pub fn start(&mut self, entry: bool) {
        self.machine.begin(&self.program);
        self.mode = Mode::Continue { entry };
        self.phase = Phase::Running;
    }

    /// Whether the run goes on without the client.
    #[must_use]
    pub const fn running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    /// Ask a running program to stop at its next stop.
    pub const fn pause(&mut self) {
        self.pause = true;
    }

    /// Resume a stopped run by `command`: `continue`, `next`, `stepIn`
    /// or `stepOut`.
    ///
    /// # Errors
    /// When the run is not stopped at a stop it can leave.
    pub fn resume(&mut self, command: &str) -> Result<(), String> {
        if !matches!(self.phase, Phase::Stopped) {
            return Err(String::from("the program is not stopped"));
        }
        let place = self.place();
        self.mode = match command {
            "continue" => Mode::Continue { entry: false },
            "next" => Mode::Next(place),
            "stepIn" => Mode::StepIn(place),
            _ => Mode::StepOut { depth: place.depth },
        };
        self.references.clear();
        self.phase = Phase::Running;
        Ok(())
    }

    /// Where the client sees the run: its depth, line and pipe.
    fn place(&self) -> Place {
        let depth = self.depth();
        Place {
            depth,
            line: depth
                .checked_sub(1)
                .and_then(|at| self.lines.get(at).copied().flatten()),
            pipe: depth
                .checked_sub(1)
                .and_then(|at| self.at_pipes.get(at).copied().flatten()),
        }
    }

    /// How many body frames the run has.
    fn depth(&self) -> usize {
        self.machine
            .frames()
            .iter()
            .filter(|frame| matches!(frame, Frame::Body(_)))
            .count()
    }

    /// Run on for at most `stops` stops, sending what the client sees.
    pub fn advance(&mut self, stops: usize, out: &mut Outgoing) {
        for _ in 0..stops {
            if !self.running() {
                return;
            }
            if self.pause {
                self.pause = false;
                self.stopped("pause", &[], out);
                return;
            }
            match self.machine.next_stop() {
                Stop::Before(node) => self.at(node, None, out),
                Stop::After(node, value) => self.at(node, Some(value), out),
                Stop::Request(request) => self.call(&request, out),
                Stop::Done(Ok(value)) => {
                    // The run's value, written as the trace's first line
                    // writes it, so a session can be compared with a run.
                    let line = format!(
                        "value {}\n",
                        show(&value, &self.program.tables.borrow())
                    );
                    out.event(
                        "output",
                        json!({ "category": "console", "output": line }),
                    );
                    out.event("exited", json!({ "exitCode": 0 }));
                    out.event("terminated", Json::Null);
                    self.phase = Phase::Ended;
                }
                Stop::Done(Err(fault)) => {
                    let description = self.located(&fault);
                    out.event(
                        "stopped",
                        json!({
                            "reason": "exception",
                            "description": description,
                            "text": fault.reason.describe(&self.program.tables.borrow()),
                            "threadId": THREAD,
                            "allThreadsStopped": true,
                        }),
                    );
                    self.phase = Phase::Faulted(fault);
                }
            }
        }
    }

    /// Answer a host call from the replay and send its trace line.
    fn call(&mut self, request: &Request, out: &mut Outgoing) {
        let tables = self.program.tables.borrow();
        let (callee, mut line) = call_line(request, &tables);
        let reply = self.replay.answer(&callee, &tables);
        match &reply {
            Ok(value) => write!(line, " -> {}", show(value, &tables)).unwrap(),
            Err(message) => write!(line, " -> failure {message:?}").unwrap(),
        }
        drop(tables);
        out.event(
            "output",
            json!({ "category": "stdout", "output": format!("{line}\n") }),
        );
        self.effects.push(line);
        self.machine.answer(Reply::Value(reply));
    }

    /// Record a stop before or after `node` and stop there when a
    /// breakpoint or the step asks to.
    fn at(&mut self, node: usize, value: Option<Value>, out: &mut Outgoing) {
        let depth = self.depth();
        self.lines.resize(depth, None);
        self.at_pipes.resize(depth, None);
        let before = value.is_none();
        if let Some(value) = value {
            self.wrote(node, value);
        } else {
            self.entered(node);
        }
        let tables = self.program.tables.borrow();
        let entry = &tables.nodes[node];
        let name = &tables.namespaces[entry.namespace];
        let line = Some((
            entry.namespace,
            self.files.position(name, entry.span.start).0,
        ));
        let pipe =
            lines::pipe_of(node, &tables, &self.pipes).map(|pipe| pipe.node);
        drop(tables);
        let index = depth.saturating_sub(1);
        let (line_entered, pipe_entered) = if depth == 0 {
            (false, false)
        } else {
            (
                before && self.lines[index] != line,
                pipe.is_some() && self.at_pipes[index] != pipe,
            )
        };
        if depth > 0 {
            // A node outside every pipe (a stage's piped value, say)
            // leaves the frame at its pipe; a new line leaves it.
            if line_entered {
                self.lines[index] = line;
                self.at_pipes[index] = pipe;
            } else if pipe.is_some() {
                self.at_pipes[index] = pipe;
            }
        }
        let hits: Vec<u64> = self
            .breakpoints
            .iter()
            .filter(|breakpoint| {
                breakpoint.pipe.map_or_else(
                    || {
                        line_entered
                            && line.is_some_and(|(namespace, line)| {
                                line == breakpoint.line
                                    && self.namespace(namespace)
                                        == breakpoint.namespace
                            })
                    },
                    |at| pipe_entered && pipe == Some(at),
                )
            })
            .map(|breakpoint| breakpoint.id)
            .collect();
        let step = match self.mode {
            Mode::Continue { entry } => entry.then_some("entry"),
            Mode::Next(place) => stepped(
                place,
                depth,
                before,
                line,
                pipe,
                line_entered,
                pipe_entered,
            )
            .then_some("step"),
            Mode::StepIn(place) => ((depth > place.depth && before)
                || stepped(
                    place,
                    depth,
                    before,
                    line,
                    pipe,
                    line_entered,
                    pipe_entered,
                ))
            .then_some("step"),
            Mode::StepOut { depth: from } => (depth < from).then_some("step"),
        };
        let reason = if hits.is_empty() {
            step
        } else {
            Some("breakpoint")
        };
        if let Some(reason) = reason {
            self.current = Some(node);
            self.stopped(reason, &hits, out);
        }
    }

    /// A value written under `node`: the latest value there, a pipe's
    /// output, or a new input to the pipes it feeds.
    fn wrote(&mut self, node: usize, value: Value) {
        for pipe in self.pipes.iter().filter(|pipe| pipe.left == node) {
            let state = self.states.entry(pipe.node).or_default();
            if state.done {
                *state = PipeState::default();
            }
        }
        if self.pipes.iter().any(|pipe| pipe.node == node) {
            let state = self.states.entry(node).or_default();
            state.output = Some(value.clone());
            state.done = true;
        }
        self.seen.insert(node, value);
    }

    /// The run is about to run `node`: a pipe takes its input once.
    fn entered(&mut self, node: usize) {
        let Some(pipe) = self.pipes.iter().find(|pipe| pipe.node == node)
        else {
            return;
        };
        let input = self.seen.get(&pipe.left).cloned();
        let state = self.states.entry(node).or_default();
        if !state.locked {
            state.input = input;
            state.locked = true;
        }
    }

    fn stopped(&mut self, reason: &str, hits: &[u64], out: &mut Outgoing) {
        self.phase = Phase::Stopped;
        self.references.clear();
        let mut body = json!({
            "reason": reason,
            "threadId": THREAD,
            "allThreadsStopped": true,
        });
        if !hits.is_empty() {
            body["hitBreakpointIds"] = json!(hits);
        }
        out.event("stopped", body);
    }

    fn namespace(&self, index: usize) -> String {
        self.program.tables.borrow().namespaces[index].clone()
    }

    /// Replace the breakpoints of one source: a line, or a line and the
    /// column of a pipe mark on it. Returns each breakpoint's answer.
    pub fn set_breakpoints(
        &mut self,
        path: &Path,
        requested: &[Json],
    ) -> Vec<Json> {
        let Some(namespace) = self.files.named(path).map(str::to_string) else {
            return requested
                .iter()
                .map(|_| json!({ "verified": false, "message": "not a source of this program" }))
                .collect();
        };
        self.breakpoints
            .retain(|breakpoint| breakpoint.namespace != namespace);
        requested
            .iter()
            .map(|breakpoint| {
                let line = count(&breakpoint["line"]);
                let column = breakpoint["column"]
                    .is_u64()
                    .then(|| count(&breakpoint["column"]));
                self.breakpoint(&namespace, line, column)
            })
            .collect()
    }

    /// One breakpoint of a namespace: on the pipe mark at `line:column`,
    /// or on `line`; its answer.
    fn breakpoint(
        &mut self,
        namespace: &str,
        line: usize,
        column: Option<usize>,
    ) -> Json {
        self.next_id += 1;
        let id = self.next_id;
        if let Some(column) = column {
            let Some(pipe) = self.mark_at(namespace, line, column) else {
                return json!({
                    "id": id, "verified": false, "line": line,
                    "message": format!("no pipe mark at {line}:{column}"),
                });
            };
            self.breakpoints.push(Breakpoint {
                id,
                namespace: namespace.to_string(),
                line,
                pipe: Some(pipe.node),
            });
            return json!({
                "id": id, "verified": true,
                "line": pipe.line, "column": pipe.column,
            });
        }
        if !self.has_line(namespace, line) {
            return json!({
                "id": id, "verified": false, "line": line,
                "message": format!("nothing runs on line {line}"),
            });
        }
        self.breakpoints.push(Breakpoint {
            id,
            namespace: namespace.to_string(),
            line,
            pipe: None,
        });
        json!({ "id": id, "verified": true, "line": line })
    }

    /// The pipe whose mark covers `line:column` in a namespace.
    fn mark_at(
        &self,
        namespace: &str,
        line: usize,
        column: usize,
    ) -> Option<Pipe> {
        let tables = self.program.tables.borrow();
        self.pipes
            .iter()
            .find(|pipe| {
                tables.namespaces[pipe.namespace] == namespace
                    && pipe.line == line
                    && (pipe.column..pipe.column + pipe.mark.len())
                        .contains(&column)
            })
            .cloned()
    }

    /// Whether some node of a namespace starts on `line`.
    fn has_line(&self, namespace: &str, line: usize) -> bool {
        let tables = self.program.tables.borrow();
        tables.nodes.iter().any(|node| {
            tables.namespaces[node.namespace] == namespace
                && self.files.position(namespace, node.span.start).0 == line
        })
    }

    /// The pipe marks of lines `from` to `to` of a source, as stage
    /// breakpoint locations.
    #[must_use]
    pub fn locations(&self, path: &Path, from: usize, to: usize) -> Vec<Json> {
        let Some(namespace) = self.files.named(path) else {
            return Vec::new();
        };
        let tables = self.program.tables.borrow();
        let mut marks: Vec<(usize, usize)> = self
            .pipes
            .iter()
            .filter(|pipe| {
                tables.namespaces[pipe.namespace] == namespace
                    && (from..=to).contains(&pipe.line)
            })
            .map(|pipe| (pipe.line, pipe.column))
            .collect();
        marks.sort_unstable();
        marks.dedup();
        marks
            .into_iter()
            .map(|(line, column)| json!({ "line": line, "column": column }))
            .collect()
    }

    /// The body frames, innermost first, as stack frames; on a Fault the
    /// one frame where it happened.
    #[must_use]
    pub fn stack(&self) -> Vec<Json> {
        let tables = self.program.tables.borrow();
        if let Phase::Faulted(fault) = &self.phase {
            let name = &tables.namespaces[fault.namespace];
            let (line, column) = self.files.position(name, fault.span.start);
            return vec![json!({
                "id": 0, "name": "fault", "line": line, "column": column,
                "source": self.source(name),
            })];
        }
        let Some(program) = self.machine.program() else {
            return Vec::new();
        };
        let bodies: Vec<_> = self.body_frames();
        bodies
            .iter()
            .rev()
            .enumerate()
            .map(|(id, frame)| {
                let body = &program.bodies[frame.body];
                let node = if id == 0 {
                    self.current.unwrap_or(body.node)
                } else {
                    let block = &body.blocks[frame.block];
                    block
                        .nodes
                        .get(frame.statement)
                        .or_else(|| block.nodes.last())
                        .copied()
                        .unwrap_or(body.node)
                };
                let (name, line, column) = self.position(node, &tables);
                json!({
                    "id": id,
                    "name": self.frame_name(body.node, &tables),
                    "line": line,
                    "column": column,
                    "source": self.source(&name),
                })
            })
            .collect()
    }

    fn body_frames(&self) -> Vec<&fipe::vm::BodyFrame> {
        self.machine
            .frames()
            .iter()
            .filter_map(|frame| match frame {
                Frame::Body(body) => Some(body),
                Frame::Native(_) => None,
            })
            .collect()
    }

    /// A node's namespace, line and column; a pipe's mark for the pipe
    /// itself.
    fn position(&self, node: usize, tables: &Tables) -> (String, usize, usize) {
        let entry = &tables.nodes[node];
        let name = tables.namespaces[entry.namespace].clone();
        if let Some(pipe) = self.pipes.iter().find(|pipe| pipe.node == node) {
            return (name, pipe.line, pipe.column);
        }
        let (line, column) = self.files.position(&name, entry.span.start);
        (name, line, column)
    }

    /// A frame's name: its definition's label, `main` or a module's init,
    /// or the arm a closure runs.
    fn frame_name(&self, node: usize, tables: &Tables) -> String {
        if let Some(definition) =
            tables.definitions.iter().find(|d| d.body == node)
        {
            return definition.label.clone();
        }
        let entry = &tables.nodes[node];
        let name = &tables.namespaces[entry.namespace];
        if matches!(entry.kind, Kind::Function { .. }) {
            let (line, column) = self.files.position(name, entry.span.start);
            return format!("arm at {line}:{column}");
        }
        if name == "main" {
            String::from("main")
        } else {
            format!("{name} (init)")
        }
    }

    fn source(&self, name: &str) -> Json {
        self.files
            .get(name)
            .and_then(|file| file.path.as_ref())
            .map_or_else(
                || json!({ "name": name }),
                |path| json!({ "name": name, "path": path }),
            )
    }

    /// The scopes of a frame: Locals, Captures, Globals, the Pipe of the
    /// top frame's stop, Effects and Budget.
    pub fn scopes(&mut self, frame: usize) -> Vec<Json> {
        let tables = self.program.tables.clone();
        let tables = tables.borrow();
        let mut scopes = Vec::new();
        let (locals, captures) = self.frame_values(frame, &tables);
        if !matches!(self.phase, Phase::Faulted(_)) {
            let reference = self.references.add(locals);
            scopes.push(scope("Locals", reference));
            if !captures.is_empty() {
                let reference = self.references.add(captures);
                scopes.push(scope("Captures", reference));
            }
        }
        let globals = self.globals(&tables);
        if !globals.is_empty() {
            let reference = self.references.add(globals);
            scopes.push(scope("Globals", reference));
        }
        if frame == 0 {
            if let Some(items) = self.pipe_items(&tables) {
                let reference = self.references.add(items);
                scopes.push(scope("Pipe", reference));
            }
        }
        let effects = self
            .effects
            .iter()
            .enumerate()
            .map(|(index, line)| {
                ((index + 1).to_string(), Item::Text(line.clone()))
            })
            .collect();
        let reference = self.references.add(effects);
        scopes.push(scope("Effects", reference));
        let reference = self.references.add(self.budget_items());
        scopes.push(scope("Budget", reference));
        scopes
    }

    /// The named locals and the captures of the frame `frame` from the
    /// top, each named by the slot it holds.
    fn frame_values(&self, frame: usize, tables: &Tables) -> (Named, Named) {
        let Some(program) = self.machine.program() else {
            return (Vec::new(), Vec::new());
        };
        let frames = self.body_frames();
        let Some(frame) = frames.iter().rev().nth(frame) else {
            return (Vec::new(), Vec::new());
        };
        let body = &program.bodies[frame.body];
        let locals = body
            .locals
            .iter()
            .zip(&frame.locals)
            .filter_map(|(local, value)| {
                let name = self.slot_name(local.slot?, tables)?;
                Some((name, Item::Value(value.clone())))
            })
            .collect();
        let slots = match &tables.nodes[body.node].kind {
            Kind::Function { captures, .. } => captures.as_slice(),
            _ => &[],
        };
        let captures = frame
            .captures
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let name = slots
                    .get(index)
                    .and_then(|slot| self.slot_name(*slot, tables))
                    .unwrap_or_else(|| format!("capture {index}"));
                (name, Item::Value(value.clone()))
            })
            .collect();
        (locals, captures)
    }

    /// The name the source binds a slot to.
    fn slot_name(&self, slot: usize, tables: &Tables) -> Option<String> {
        let (namespace, span) = tables.slots.get(slot)?.name?;
        self.files
            .slice(&tables.namespaces[namespace], span)
            .map(str::to_string)
    }

    /// The module-level values the runs so far stored, of the program
    /// and the file modules it loaded, by name.
    fn globals(&self, tables: &Tables) -> Named {
        self.machine
            .globals()
            .filter_map(|(definition, value)| {
                let definition = &tables.definitions[definition];
                let namespace = &tables.namespaces[definition.namespace];
                self.files
                    .get(namespace)
                    .is_some_and(|file| file.path.is_some())
                    .then(|| {
                        (definition.label.clone(), Item::Value(value.clone()))
                    })
            })
            .collect()
    }

    /// The pipe the stop is at: its mark, the value fed to it, the side
    /// that value is on, whether the stage runs, and the value it gave.
    fn pipe_items(&self, tables: &Tables) -> Option<Vec<(String, Item)>> {
        let node = self.current?;
        let pipe = lines::pipe_of(node, tables, &self.pipes)?;
        let state = self.states.get(&pipe.node);
        let input = state.and_then(|state| state.input.clone());
        let output = state
            .filter(|state| state.done)
            .and_then(|state| state.output.clone());
        let side = input
            .as_ref()
            .and_then(|value| inspect::side(value, tables));
        let mut items =
            vec![(String::from("mark"), Item::Text(pipe.mark.to_string()))];
        items.push((
            String::from("in"),
            input.map_or_else(
                || Item::Text(String::from("not reached")),
                Item::Value,
            ),
        ));
        items.push((
            String::from("side"),
            Item::Text(String::from(match (side, pipe.mark) {
                (Some(true), "||" | "&&") if input_is_bool(state, tables) => {
                    "true"
                }
                (Some(false), "||" | "&&") if input_is_bool(state, tables) => {
                    "false"
                }
                (Some(true), _) => "good",
                (Some(false), _) => "bad",
                (None, _) => "value",
            })),
        ));
        items.push((
            String::from("stage"),
            Item::Text(String::from(match side {
                _ if pipe.mark == "|" => "runs",
                Some(good) if pipe.runs(good) => "runs",
                Some(_) => "skipped",
                None if state.and_then(|s| s.input.as_ref()).is_some() => {
                    "runs"
                }
                None => "not reached",
            })),
        ));
        items.push((
            String::from("out"),
            output.map_or_else(
                || Item::Text(String::from("not yet")),
                Item::Value,
            ),
        ));
        Some(items)
    }

    fn budget_items(&self) -> Vec<(String, Item)> {
        let charged = self.machine.charged();
        let mut items = Vec::new();
        match self.machine.budget() {
            Some(budget) => {
                items.push((
                    String::from("remaining"),
                    Item::Text(budget.saturating_sub(charged).to_string()),
                ));
                items.push((
                    String::from("budget"),
                    Item::Text(budget.to_string()),
                ));
            }
            None => items.push((
                String::from("remaining"),
                Item::Text(String::from("unbounded")),
            )),
        }
        items.push((String::from("charged"), Item::Text(charged.to_string())));
        items
    }

    /// The variables behind a reference.
    pub fn variables(&mut self, reference: u64) -> Option<Json> {
        let tables = self.program.tables.clone();
        let tables = tables.borrow();
        self.references.variables(reference, &tables)
    }

    /// The value of a name bound in a frame: its latest local of that
    /// name, else a capture, else a module-level value.
    ///
    /// # Errors
    /// When `expression` is not a name, or names nothing in the frame.
    pub fn evaluate(
        &mut self,
        expression: &str,
        frame: usize,
    ) -> Result<Json, String> {
        let name = expression.trim();
        if !is_name(name) {
            return Err(String::from(
                "evaluate takes a name bound in the frame, not an expression",
            ));
        }
        let tables = self.program.tables.clone();
        let tables = tables.borrow();
        let (locals, captures) = self.frame_values(frame, &tables);
        let found = locals
            .into_iter()
            .rev()
            .chain(captures)
            .chain(self.globals(&tables))
            .find(|(bound, _)| bound == name);
        let Some((_, item)) = found else {
            return Err(format!("{name} is not bound in this frame"));
        };
        let variable = self.references.variable(name, &item, &tables);
        Ok(json!({
            "result": variable["value"],
            "variablesReference": variable["variablesReference"],
        }))
    }

    /// The Fault a run stopped on, for `exceptionInfo`.
    #[must_use]
    pub fn exception(&self) -> Option<Json> {
        let Phase::Faulted(fault) = &self.phase else {
            return None;
        };
        Some(json!({
            "exceptionId": "fault",
            "description": self.located(fault),
            "breakMode": "always",
            "details": {
                "message": fault.reason.describe(&self.program.tables.borrow()),
                "typeName": "Fault",
            },
        }))
    }

    /// A Fault located as `fault in <source> at line:column: <reason>`.
    #[must_use]
    pub fn located(&self, fault: &Fault) -> String {
        let tables = self.program.tables.borrow();
        let name = &tables.namespaces[fault.namespace];
        let (line, column) = self.files.position(name, fault.span.start);
        format!(
            "fault in {name} at {line}:{column}: {}",
            fault.reason.describe(&tables)
        )
    }
}

/// Whether a step from `place` ends at this stop: back in a caller, or
/// in its own frame at another pipe or on another line.
fn stepped(
    place: Place,
    depth: usize,
    before: bool,
    line: Option<(usize, usize)>,
    pipe: Option<usize>,
    line_entered: bool,
    pipe_entered: bool,
) -> bool {
    if depth < place.depth {
        return true;
    }
    depth == place.depth
        && ((pipe_entered && pipe.is_some() && pipe != place.pipe)
            || (before && line_entered && line != place.line))
}

/// Whether a text is one name: a letter or `_`, then letters, digits and
/// `_`.
fn is_name(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && text.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn input_is_bool(state: Option<&PipeState>, tables: &Tables) -> bool {
    let Some(Value::Record(record)) =
        state.and_then(|state| state.input.as_ref())
    else {
        return false;
    };
    tables
        .meanings
        .get(record.meaning)
        .is_some_and(|meaning| meaning.label == "Bool")
}

/// A request's number argument as a `usize`, zero when absent.
fn count(value: &Json) -> usize {
    value
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
        .unwrap_or(0)
}

fn scope(name: &str, reference: u64) -> Json {
    json!({ "name": name, "variablesReference": reference, "expensive": false })
}

/// A host call's `ns.label` and its trace line up to the reply.
fn call_line(request: &Request, tables: &Tables) -> (String, String) {
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        unreachable!("a run asks only for host calls")
    };
    let declared = &tables.declarations[*declaration];
    let callee = format!(
        "{}.{}",
        tables.namespaces[declared.namespace], declared.label
    );
    let arguments: Vec<String> =
        arguments.iter().map(|value| show(value, tables)).collect();
    let mut line = format!("call {callee} ({})", arguments.join(", "));
    for (key, value) in environment {
        write!(line, " with {} {}", tables.keys[*key], show(value, tables))
            .unwrap();
    }
    (callee, line)
}
