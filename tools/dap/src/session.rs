//! The session: one program, launched, run and reported.
//!
//! States: waiting for `launch`, launched, running, stopped
//! on a Fault, done. Every value it shows was produced by the VM and
//! every span comes from the program's tables.
use crate::{protocol::Outgoing, replay::Replay};
use fipe::{
    check,
    decl::{self, Declarations},
    embed::{Reply, Request},
    lex::lex,
    mir::Program,
    parse::parse_partial,
    token::Source,
    value::structural::show,
    vm::{Fault, Machine, Stop},
};
use serde_json::{json, Value};
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

/// The one thread a run has.
pub const THREAD: u64 = 1;

/// What the request loop does after a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Exit,
}

/// Where a session is.
#[derive(Debug, Default)]
enum State {
    #[default]
    Waiting,
    Launched(Box<Launched>),
    /// On a Fault, until `disconnect`.
    Stopped,
    Done,
}

/// A checked program ready to run, with what its run reads.
#[derive(Debug)]
struct Launched {
    program: Program,
    /// The text of each source a span may be in, by namespace.
    sources: Vec<(String, String)>,
    replay: Replay,
    budget: Option<u64>,
}

/// A debug session over one program.
#[derive(Debug, Default)]
pub struct Session {
    state: State,
    /// The messages to send.
    pub out: Outgoing,
}

impl Session {
    /// Handle one request, queueing its response and any events.
    pub fn handle(&mut self, request: &Value) -> Control {
        let command = request["command"].as_str().unwrap_or_default();
        match command {
            "initialize" => {
                self.out.respond(
                    request,
                    Ok(json!({
                        "supportsConfigurationDoneRequest": true,
                    })),
                );
                self.out.event("initialized", Value::Null);
            }
            "launch" => {
                let result = launch(&request["arguments"]);
                match result {
                    Ok(launched) => {
                        self.state = State::Launched(Box::new(launched));
                        self.out.respond(request, Ok(Value::Null));
                    }
                    Err(message) => self.out.respond(request, Err(message)),
                }
            }
            "configurationDone" => {
                self.out.respond(request, Ok(Value::Null));
                if let State::Launched(launched) =
                    std::mem::take(&mut self.state)
                {
                    self.state = run(*launched, &mut self.out);
                }
            }
            "threads" => self.out.respond(
                request,
                Ok(json!({ "threads": [{ "id": THREAD, "name": "main" }] })),
            ),
            "disconnect" => {
                self.out.respond(request, Ok(Value::Null));
                self.state = State::Done;
                return Control::Exit;
            }
            _ => self
                .out
                .respond(request, Err(format!("unsupported: {command}"))),
        }
        Control::Continue
    }

    /// Whether the run stopped on a Fault.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        matches!(self.state, State::Stopped)
    }
}

/// Check and lower the program `launch` names. Arguments: `program`
/// (a path), `host` (a directory of host declaration files:
/// `commands.fipe` is the command namespace, every other file a module
/// by its stem), `trace` (a trace fixture to replay; without one every
/// host call fails) and `budget` (units; without one the run is
/// unbounded).
fn launch(arguments: &Value) -> Result<Launched, String> {
    let path = arguments["program"]
        .as_str()
        .map(PathBuf::from)
        .ok_or("launch: no program")?;
    let text = read(&path)?;
    let mut sources = vec![(String::from("main"), text.clone())];
    let mut declarations = Declarations::new();
    if let Some(host) = arguments["host"].as_str() {
        load_host(Path::new(host), &mut declarations, &mut sources)?;
    }
    let module = parse_partial(lex(&text));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut files = Vec::new();
    load_uses(dir, &module, &mut files, &mut sources, &declarations)?;
    let unit =
        check::check(&module, &declarations, &files, check::Options::default())
            .map_err(|errors| {
                format!(
                    "{} does not check: {} diagnostics, the first {:?}",
                    path.display(),
                    errors.len(),
                    errors[0]
                )
            })?;
    let replay = match arguments["trace"].as_str() {
        Some(trace) => Replay::parse(&read(Path::new(trace))?)?,
        None => Replay::default(),
    };
    Ok(Launched {
        program: fipe::compile::compile(&unit),
        sources,
        replay,
        budget: arguments["budget"].as_u64(),
    })
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_host(
    dir: &Path,
    declarations: &mut Declarations,
    sources: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let commands = dir.join("commands.fipe");
    let mut modules = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "fipe") && path != commands
        {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            modules.push((stem, read(&path)?));
        }
    }
    // A module the command namespace uses must be declared first.
    modules.sort();
    for (stem, text) in &modules {
        declarations
            .module(stem, text)
            .map_err(|errors| format!("host module {stem}: {errors:?}"))?;
        sources.push((stem.clone(), text.clone()));
    }
    if commands.exists() {
        let text = read(&commands)?;
        declarations
            .commands(&text)
            .map_err(|errors| format!("host commands: {errors:?}"))?;
        sources.push((String::from("commands"), text));
    }
    Ok(())
}

/// The file modules a module `use`s, beside the program, recursively.
fn load_uses(
    dir: &Path,
    module: &fipe::ast::Module,
    files: &mut Vec<(String, fipe::ast::Module)>,
    sources: &mut Vec<(String, String)>,
    declarations: &Declarations,
) -> Result<(), String> {
    for entry in &module.entries {
        let fipe::ast::Entry::Item(item) = entry else {
            continue;
        };
        let fipe::ast::ItemKind::Use { names, .. } = &item.kind else {
            continue;
        };
        for name in names {
            let Some(stem) = name.segments.first() else {
                continue;
            };
            let path = dir.join(format!("{stem}.fipe"));
            if files.iter().any(|(loaded, _)| loaded == stem)
                || declarations.get_module(stem).is_some()
                || !path.exists()
            {
                continue;
            }
            let text = read(&path)?;
            let file = parse_partial(lex(&text));
            files.push((stem.clone(), file.clone()));
            sources.push((stem.clone(), text));
            load_uses(dir, &file, files, sources, declarations)?;
        }
    }
    Ok(())
}

/// Run a launched program to its end or its Fault: one `output` event
/// per host call, as the trace writes it; `exited` and `terminated` at
/// the end; `stopped` with reason `exception` on a Fault.
fn run(mut launched: Launched, out: &mut Outgoing) -> State {
    let mut machine = Machine::new(launched.budget);
    machine.begin(&launched.program);
    loop {
        match machine.next_stop() {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let tables = launched.program.tables.borrow();
                let (callee, mut line) = call(&request, &tables);
                let reply = launched.replay.answer(&callee, &tables);
                match &reply {
                    Ok(value) => {
                        write!(line, " -> {}", show(value, &tables)).unwrap();
                    }
                    Err(message) => {
                        write!(line, " -> failure {message:?}").unwrap();
                    }
                }
                drop(tables);
                out.event(
                    "output",
                    json!({ "category": "stdout", "output": line + "\n" }),
                );
                machine.answer(Reply::Value(reply));
            }
            Stop::Done(Ok(_)) => {
                out.event("exited", json!({ "exitCode": 0 }));
                out.event("terminated", Value::Null);
                return State::Done;
            }
            Stop::Done(Err(fault)) => {
                let description = located(&fault, &launched);
                out.event(
                    "stopped",
                    json!({
                        "reason": "exception",
                        "description": description,
                        "text": fault.reason.to_string(),
                        "threadId": THREAD,
                        "allThreadsStopped": true,
                    }),
                );
                return State::Stopped;
            }
        }
    }
}

/// A host call's `ns.label` and its trace line up to the reply.
fn call(request: &Request, tables: &fipe::ir::Tables) -> (String, String) {
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

/// A Fault located as `fault in <source> at line:column: <reason>`.
fn located(fault: &Fault, launched: &Launched) -> String {
    let tables = launched.program.tables.borrow();
    let namespace = &tables.namespaces[fault.namespace];
    let text = if namespace == "prelude" {
        decl::prelude_source()
    } else {
        launched
            .sources
            .iter()
            .find(|(name, _)| name == namespace)
            .map_or("", |(_, text)| text.as_str())
    };
    let (line, column) = Source::new(text).line_col(fault.span.start);
    format!("fault in {namespace} at {line}:{column}: {}", fault.reason)
}
