//! The session: one program, launched, run, stopped, stepped and
//! inspected.
//!
//! States: waiting for `launch`, launched, then the program's own
//! phase (running, stopped, faulted, ended) until `disconnect`.
use crate::{
    debuggee::{Debuggee, Host, Phase, THREAD},
    lines::Files,
    protocol::Outgoing,
    replay::Replay,
};
use fipe::{
    check,
    decl::Declarations,
    lex::lex,
    parse::parse_partial,
    token::{Source, Span},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// What the request loop does after a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Exit,
}

/// A debug session over one program.
#[derive(Debug, Default)]
pub struct Session {
    debuggee: Option<Box<Debuggee>>,
    /// Whether `launch` asked to stop at the first stop.
    entry: bool,
    /// Whether the client counts lines, and columns, from zero
    /// (`linesStartAt1` / `columnsStartAt1` false); the adapter counts
    /// from one inside and converts at the wire.
    lines_from_zero: bool,
    columns_from_zero: bool,
    /// The messages to send.
    pub out: Outgoing,
}

impl Session {
    /// Handle one request, queueing its response and any events. A
    /// request that resumes the program leaves it running: the loop
    /// calls [`Session::advance`] until it stops.
    pub fn handle(&mut self, request: &Value) -> Control {
        if !request.is_object() || request["type"] != "request" {
            self.malformed("a message the adapter reads is a request object");
            return Control::Continue;
        }
        let Some(command) = request["command"].as_str() else {
            self.out.respond(
                request,
                Err(String::from("a request needs a command")),
            );
            return Control::Continue;
        };
        let arguments = &self.incoming(command, &request["arguments"]);
        let result = match command {
            "initialize" => {
                self.lines_from_zero =
                    arguments["linesStartAt1"].as_bool() == Some(false);
                self.columns_from_zero =
                    arguments["columnsStartAt1"].as_bool() == Some(false);
                Ok(json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsBreakpointLocationsRequest": true,
                    "supportsExceptionInfoRequest": true,
                }))
            }
            "launch" => match launch(arguments) {
                Ok(debuggee) => {
                    self.debuggee = Some(Box::new(debuggee));
                    self.entry =
                        arguments["stopOnEntry"].as_bool() == Some(true);
                    self.out.respond(request, Ok(Value::Null));
                    self.out.event("initialized", Value::Null);
                    return Control::Continue;
                }
                Err(message) => Err(message),
            },
            "configurationDone" => {
                let entry = self.entry;
                self.with(|debuggee| {
                    if matches!(debuggee.phase, Phase::Idle) {
                        debuggee.start(entry);
                    }
                    Ok(Value::Null)
                })
            }
            "threads" => {
                Ok(json!({ "threads": [{ "id": THREAD, "name": "main" }] }))
            }
            "setBreakpoints" | "breakpointLocations" => {
                self.breakpoints(command, arguments)
            }
            "continue" | "next" | "stepIn" | "stepOut" if self.stopped() => {
                // A Faulted run cannot go on: resuming it ends it.
                self.out.respond(request, Ok(Value::Null));
                if let Some(debuggee) = self.debuggee.as_mut() {
                    debuggee.end(&mut self.out);
                }
                return Control::Continue;
            }
            "continue" | "next" | "stepIn" | "stepOut" => {
                self.with(|debuggee| {
                    debuggee.resume(command)?;
                    Ok(if command == "continue" {
                        json!({ "allThreadsContinued": true })
                    } else {
                        Value::Null
                    })
                })
            }
            "pause" => self.with(|debuggee| {
                if debuggee.running() {
                    debuggee.pause();
                    Ok(Value::Null)
                } else {
                    Err(String::from("the program is not running"))
                }
            }),
            "stackTrace" | "scopes" | "variables" | "evaluate"
            | "exceptionInfo" => self.inspect(command, arguments),
            "disconnect" => {
                self.out.respond(request, Ok(Value::Null));
                self.debuggee = None;
                return Control::Exit;
            }
            _ => Err(format!("unsupported: {command}")),
        };
        let result = result.map(|mut body| {
            self.outgoing(command, &mut body);
            body
        });
        self.out.respond(request, result);
        Control::Continue
    }

    /// Answer `setBreakpoints` or `breakpointLocations`.
    fn breakpoints(
        &mut self,
        command: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        match command {
            "setBreakpoints" => {
                let path = PathBuf::from(
                    arguments["source"]["path"].as_str().unwrap_or_default(),
                );
                let requested = arguments["breakpoints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                self.with(|debuggee| {
                    Ok(json!({ "breakpoints": debuggee.set_breakpoints(&path, &requested) }))
                })
            }
            "breakpointLocations" => {
                let path = PathBuf::from(
                    arguments["source"]["path"].as_str().unwrap_or_default(),
                );
                let from = number(&arguments["line"]);
                let to = if arguments["endLine"].is_u64() {
                    number(&arguments["endLine"])
                } else {
                    from
                };
                self.with(|debuggee| {
                    Ok(json!({ "breakpoints": debuggee.locations(&path, from, to) }))
                })
            }
            _ => unreachable!("not a breakpoint request: {command}"),
        }
    }

    /// Answer a request that reads the stopped run: `stackTrace`,
    /// `scopes`, `variables`, `evaluate` or `exceptionInfo`.
    fn inspect(
        &mut self,
        command: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        match command {
            "stackTrace" => self.with(|debuggee| {
                let frames = debuggee.stack();
                Ok(json!({ "totalFrames": frames.len(), "stackFrames": frames }))
            }),
            "scopes" => frame(arguments).map_or_else(
                || Err(String::from("scopes needs a frameId")),
                |frame| {
                    self.with(|debuggee| {
                        debuggee.frame(frame)?;
                        Ok(json!({ "scopes": debuggee.scopes(frame) }))
                    })
                },
            ),
            "variables" => {
                let reference = arguments["variablesReference"].as_u64().unwrap_or(0);
                self.with(|debuggee| {
                    debuggee
                        .variables(reference)
                        .ok_or_else(|| format!("no variables behind {reference}"))
                })
            }
            "evaluate" => {
                let expression = arguments["expression"].as_str().unwrap_or_default().to_string();
                let frame = if arguments["frameId"].is_null() {
                    Some(0)
                } else {
                    frame(arguments)
                };
                frame.map_or_else(
                    || Err(String::from("frameId is not a frame")),
                    |frame| {
                        self.with(|debuggee| {
                            debuggee.frame(frame)?;
                            debuggee.evaluate(&expression, frame)
                        })
                    },
                )
            }
            "exceptionInfo" => self.with(|debuggee| {
                debuggee
                    .exception()
                    .ok_or_else(|| String::from("the program did not stop on a Fault"))
            }),
            _ => unreachable!("not an inspection: {command}"),
        }
    }

    /// Tell the client a frame on the input could not be read; the
    /// session goes on.
    pub fn malformed(&mut self, why: &str) {
        self.out.event(
            "output",
            json!({
                "category": "important",
                "output": format!("ignored a malformed message: {why}\n"),
            }),
        );
    }

    /// A request's arguments with the client's line and column numbers
    /// counted from one, as the adapter counts.
    fn incoming(&self, command: &str, arguments: &Value) -> Value {
        let mut arguments = arguments.clone();
        let (lines, columns) = (self.lines_from_zero, self.columns_from_zero);
        if lines || columns {
            match command {
                "setBreakpoints" => {
                    if let Some(all) = arguments["breakpoints"].as_array_mut() {
                        for breakpoint in all {
                            shift(breakpoint, lines, columns, 1);
                        }
                    }
                }
                "breakpointLocations" => {
                    shift(&mut arguments, lines, columns, 1);
                }
                _ => {}
            }
        }
        arguments
    }

    /// A response body with line and column numbers counted as the
    /// client counts.
    fn outgoing(&self, command: &str, body: &mut Value) {
        let (lines, columns) = (self.lines_from_zero, self.columns_from_zero);
        if !(lines || columns) {
            return;
        }
        let key = match command {
            "setBreakpoints" | "breakpointLocations" => "breakpoints",
            "stackTrace" => "stackFrames",
            _ => return,
        };
        if let Some(all) = body[key].as_array_mut() {
            for item in all {
                shift(item, lines, columns, -1);
            }
        }
    }

    /// Run the program on for at most `stops` stops.
    pub fn advance(&mut self, stops: usize) {
        if let Some(debuggee) = self.debuggee.as_mut() {
            debuggee.advance(stops, &mut self.out);
        }
    }

    /// Run the program on until it stops, Faults or ends.
    pub fn settle(&mut self) {
        while self.running() {
            self.advance(usize::MAX);
        }
    }

    /// Whether the program runs without the client.
    #[must_use]
    pub fn running(&self) -> bool {
        self.debuggee
            .as_ref()
            .is_some_and(|debuggee| debuggee.running())
    }

    /// Whether the run stopped on a Fault.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.debuggee
            .as_ref()
            .is_some_and(|debuggee| matches!(debuggee.phase, Phase::Faulted(_)))
    }

    fn with(
        &mut self,
        act: impl FnOnce(&mut Debuggee) -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.debuggee.as_mut().map_or_else(
            || Err(String::from("no program is launched")),
            |debuggee| act(debuggee),
        )
    }
}

/// Move the line fields (`line`, `endLine`) and column fields
/// (`column`, `endColumn`) of `value` by `by`, those of each kind only
/// when the client counts it from zero.
fn shift(value: &mut Value, lines: bool, columns: bool, by: i64) {
    for (key, moved) in [
        ("line", lines),
        ("endLine", lines),
        ("column", columns),
        ("endColumn", columns),
    ] {
        if !moved {
            continue;
        }
        if let Some(number) = value[key].as_i64() {
            value[key] = json!((number + by).max(0));
        }
    }
}

/// A request's `frameId` as a frame index, when it is one.
fn frame(arguments: &Value) -> Option<usize> {
    arguments["frameId"]
        .as_u64()
        .and_then(|frame| usize::try_from(frame).ok())
}

fn number(value: &Value) -> usize {
    value
        .as_u64()
        .map_or(0, |number| usize::try_from(number).unwrap_or(0))
}

/// Check and lower the program `launch` names. Arguments: `program`
/// (a path), `host` (a directory of host declaration files:
/// `commands.fipe` is the command namespace, every other file a module
/// by its stem; the std host's own without one), `trace` (a trace
/// fixture to replay; without one the std host answers live, under
/// `cwd`, `allowWrite`, `args` and `input`), `budget` (units; without
/// one the run is unbounded) and `stopOnEntry`.
fn launch(arguments: &Value) -> Result<Debuggee, String> {
    let budget = match &arguments["budget"] {
        Value::Null => None,
        budget => Some(budget.as_u64().ok_or_else(|| {
            format!(
                "launch: the budget is a natural number of units, not {budget}"
            )
        })?),
    };
    let path = arguments["program"]
        .as_str()
        .map(PathBuf::from)
        .ok_or("launch: no program")?;
    let text = read(&path)?;
    let mut files = Files::default();
    files.push("main", Some(path.clone()), text.clone());
    let mut declarations = Declarations::new();
    if let Some(host) = arguments["host"].as_str() {
        load_host(Path::new(host), &mut declarations, &mut files)?;
    } else {
        declarations = fipe_host::declarations();
        for (name, text) in fipe_host::sources() {
            files.push(name, None, String::from(*text));
        }
    }
    let module = parse_partial(lex(&text));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut modules = Vec::new();
    load_uses(dir, &module, &mut modules, &mut files, &declarations)?;
    let unit = check::check(
        &module,
        &declarations,
        &modules,
        check::Options::default(),
    )
    .map_err(|errors| {
        format!(
            "{} does not check: {} diagnostic{}, the first {}",
            path.display(),
            errors.len(),
            if errors.len() == 1 { "" } else { "s" },
            describe(&errors[0], &files)
        )
    })?;
    let program = fipe::compile::compile(&unit);
    let (host, input) = if let Some(trace) = arguments["trace"].as_str() {
        (Host::Replay(Replay::parse(&read(Path::new(trace))?)?), None)
    } else {
        let strings = |key: &str| -> Result<Vec<String>, String> {
            match &arguments[key] {
                Value::Null => Ok(Vec::new()),
                Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str().map(String::from).ok_or_else(|| {
                            format!("launch: {key} holds strings, not {item}")
                        })
                    })
                    .collect(),
                other => Err(format!("launch: {key} is a list, not {other}")),
            }
        };
        let cwd = arguments["cwd"].as_str().unwrap_or(".");
        let allow: Vec<PathBuf> = strings("allowWrite")?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let policy = fipe_host::Policy::new(Path::new(cwd), &allow)
            .map_err(|error| format!("launch: the write roots: {error}"))?;
        let host = fipe_host::Host::new(policy).with_args(strings("args")?);
        let input = match &arguments["input"] {
            Value::Null => None,
            Value::String(text) => {
                fipe_host::input(&program, &mut text.as_bytes())
                    .map_err(|reason| format!("launch: {reason}"))?
            }
            other => return Err(format!("launch: input is text, not {other}")),
        };
        (Host::Live(host), input)
    };
    Ok(Debuggee::new(program, files, host, input, budget))
}

/// A check error in words: where it is and the crate's own words for
/// what it is.
fn describe(error: &check::Error, files: &Files) -> String {
    let at = |name: &str, span: Span| {
        let (line, column) = Source::new(files.text(name)).line_col(span.start);
        if name == "main" {
            format!("at {line}:{column}")
        } else {
            format!("in {name} at {line}:{column}")
        }
    };
    match error {
        check::Error::Syntax(error) => {
            format!("{}: {}", at("main", error.span), error.reason)
        }
        check::Error::Resolve(error) => {
            format!("{}: {}", at("main", error.span), error.reason)
        }
        check::Error::Type(error) => {
            format!("{}: {}", at("main", error.span), error.reason)
        }
        check::Error::File { stem, error } => {
            format!("{}: {}", at(stem, error.span), error.reason)
        }
        check::Error::FileResolve { stem, error } => {
            format!("{}: {}", at(stem, error.span), error.reason)
        }
        check::Error::FileType { stem, error } => {
            format!("{}: {}", at(stem, error.span), error.reason)
        }
        check::Error::Compile { span, what, reason } => {
            format!("{}: {}", at("main", *span), what.describe(reason))
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn load_host(
    dir: &Path,
    declarations: &mut Declarations,
    files: &mut Files,
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
            let text = read(&path)?;
            modules.push((stem, path, text));
        }
    }
    // A module the command namespace uses must be declared first.
    modules.sort();
    for (stem, path, text) in modules {
        declarations.module(&stem, &text).map_err(|errors| {
            format!("host module {stem}: {} errors", errors.len())
        })?;
        files.push(&stem, Some(path), text);
    }
    if commands.exists() {
        let text = read(&commands)?;
        declarations.commands(&text).map_err(|errors| {
            format!("host commands: {} errors", errors.len())
        })?;
        files.push("commands", Some(commands), text);
    }
    Ok(())
}

/// The file modules a module `use`s, beside the program, recursively.
fn load_uses(
    dir: &Path,
    module: &fipe::ast::Module,
    modules: &mut Vec<(String, fipe::ast::Module)>,
    files: &mut Files,
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
            if modules.iter().any(|(loaded, _)| loaded == stem)
                || declarations.get_module(stem).is_some()
                || !path.exists()
            {
                continue;
            }
            let text = read(&path)?;
            let file = parse_partial(lex(&text));
            modules.push((stem.clone(), file.clone()));
            files.push(stem, Some(path), text);
            load_uses(dir, &file, modules, files, declarations)?;
        }
    }
    Ok(())
}
