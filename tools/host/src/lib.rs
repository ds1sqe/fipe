//! The std host: Fipe scripts against the real machine, under the
//! contract `tools/host/README.md` documents.
//!
//! [`load`] reads and checks a script; [`Host::run`] runs it, answering
//! each host call with a real effect under the [`Policy`]'s write
//! roots, and gives the observation and the exit code. A driver that
//! runs the machine itself (a REPL, a debugger, a native runtime) calls
//! [`Host::answer`] for each request, writes main lines through
//! [`Sink`], takes its [`input`] and ends with [`exit_code`]; with
//! [`Host::streaming`] the output reaches its writers as it is made.
#![forbid(unsafe_code)]

mod answer;
mod load;
mod policy;
mod sink;

pub use load::{load, Diagnostic, LoadError, Script};
pub use policy::Policy;
pub use sink::{located, Sink};

use fipe::{
    decl::Declarations,
    embed::Outcome,
    ir,
    mir::Program,
    value::{Prelude, Reason, Value},
    vm::{self, Fault, Machine, Observation},
};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

const SOURCES: &[(&str, &str)] = &[
    ("commands", include_str!("../decl/commands.fipe")),
    ("fs", include_str!("../decl/fs.fipe")),
    ("sys", include_str!("../decl/sys.fipe")),
    ("cli", include_str!("../decl/cli.fipe")),
    ("text", include_str!("../decl/text.fipe")),
];

/// The std host's declaration files by namespace, `commands` first.
#[must_use]
pub const fn sources() -> &'static [(&'static str, &'static str)] {
    SOURCES
}

/// The std host's declarations, loaded from [`sources`].
///
/// # Panics
/// When a declaration file does not load, a defect the tests catch.
#[must_use]
pub fn declarations() -> Declarations {
    let mut declarations = Declarations::new();
    for (name, text) in SOURCES {
        let loaded = if *name == "commands" {
            declarations.commands(text)
        } else {
            declarations.module(name, text)
        };
        if let Err(errors) = loaded {
            panic!("the std host's {name} declarations: {errors:?}");
        }
    }
    declarations
}

/// The std host over runs: its policy, the script's arguments, the
/// resources `sys.lookup` numbered and the current run's output.
pub struct Host {
    policy: Policy,
    args: Vec<String>,
    resources: Vec<PathBuf>,
    /// The output of the run in progress, or the last one; empty so
    /// far as [`Host::flush`] has moved it to the streams.
    pub sink: Sink,
    /// Where [`Host::flush`] writes the success and error output.
    streams: Option<(Box<dyn Write>, Box<dyn Write>)>,
}

impl Host {
    /// A host under a policy, with no arguments.
    #[must_use]
    pub const fn new(policy: Policy) -> Self {
        Self {
            policy,
            args: Vec::new(),
            resources: Vec::new(),
            sink: Sink::new(),
            streams: None,
        }
    }

    /// The same host writing its output to `out` and `err` as a run
    /// makes it: after each host call and at the run's end. The
    /// observation's output is then empty.
    #[must_use]
    pub fn streaming(
        mut self,
        out: impl Write + 'static,
        err: impl Write + 'static,
    ) -> Self {
        self.streams = Some((Box::new(out), Box::new(err)));
        self
    }

    /// Move the sink's output to the streams, when there are any.
    ///
    /// # Errors
    /// The writers' errors.
    pub fn flush(&mut self) -> io::Result<()> {
        let Some((out, err)) = &mut self.streams else {
            return Ok(());
        };
        if !self.sink.success.is_empty() {
            out.write_all(&self.sink.success)?;
            self.sink.success.clear();
            out.flush()?;
        }
        if !self.sink.error.is_empty() {
            err.write_all(&self.sink.error)?;
            self.sink.error.clear();
            err.flush()?;
        }
        Ok(())
    }

    /// The same host, `cli.args` answering these arguments.
    #[must_use]
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Run a script under a budget: host input from `input` when its
    /// last line opens a `String` or `Vec<U8>` spot, each host call
    /// answered, main lines and Faults written by the sink policy. The
    /// observation and the exit code: 0 a value, 1 an `Err` value or a
    /// Fault, 2 input refused before any effect.
    pub fn run(
        &mut self,
        script: &Script,
        input: &mut dyn Read,
        budget: Option<u64>,
    ) -> (Observation, u8) {
        let program = &script.program;
        self.sink = Sink::new();
        let mut machine = Machine::new(budget);
        let outcome = match self::input(program, input) {
            Ok(None) => machine.run(program),
            Ok(Some(value)) => match machine.run_with_input(program, value) {
                Ok(outcome) => outcome,
                Err(error) => return self.refuse(script, &error.to_string()),
            },
            Err(reason) => return self.refuse(script, &reason),
        };
        let (result, trace) = self.drive(&mut machine, program, outcome);
        let tables = program.tables.borrow();
        match &result {
            Err(fault) => self.sink.fault(fault, &tables, &script.sources),
            Ok(value) => self.sink.ending(value, machine.lines(), &tables),
        }
        let exit = exit_code(&result, &tables);
        drop(tables);
        // A closed stream leaves nothing to report to.
        let _ = self.flush();
        let observation = Observation {
            result,
            trace,
            success: std::mem::take(&mut self.sink.success),
            error: std::mem::take(&mut self.sink.error),
        };
        (observation, exit)
    }

    /// Answer each request until the run ends: its result and trace.
    fn drive(
        &mut self,
        machine: &mut Machine,
        program: &Program,
        mut outcome: Outcome<Result<Value, Fault>>,
    ) -> (
        Result<Value, Fault>,
        Vec<(fipe::embed::Request, fipe::embed::Reply)>,
    ) {
        let mut trace = Vec::new();
        loop {
            match outcome {
                Outcome::Done(result) => {
                    self.sink.lines(machine.lines(), &program.tables.borrow());
                    return (result, trace);
                }
                Outcome::Request(request) => {
                    let reply = {
                        let tables = program.tables.borrow();
                        self.sink.lines(machine.lines(), &tables);
                        self.answer(&request, &tables)
                    };
                    trace.push((request, reply.clone()));
                    let _ = self.flush();
                    outcome = machine.reply(program, reply);
                }
            }
        }
    }

    /// A run refused before any effect: a `Host` Fault at the last line,
    /// written to the error output, and exit 2.
    fn refuse(&mut self, script: &Script, reason: &str) -> (Observation, u8) {
        let program = &script.program;
        let fault = refusal(program, reason);
        self.sink
            .fault(&fault, &program.tables.borrow(), &script.sources);
        let _ = self.flush();
        let observation = Observation {
            result: Err(fault),
            trace: Vec::new(),
            success: std::mem::take(&mut self.sink.success),
            error: std::mem::take(&mut self.sink.error),
        };
        (observation, 2)
    }
}

/// The host input the program's last line takes: none when it is not
/// a function, the input read as text or bytes for a `String` or
/// `Vec<U8>` spot.
///
/// # Errors
/// The refusal's reason: any other open spot, several, input that
/// cannot be read, or input that is not UTF-8 for a `String` spot.
pub fn input(
    program: &Program,
    input: &mut dyn Read,
) -> Result<Option<Value>, String> {
    let spot = match vm::input_spot(program) {
        Ok(spot) => spot,
        Err(error) if error.spots == 0 => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let tables = program.tables.borrow();
    let text = match prelude_label(&tables, &spot) {
        Some("String") => true,
        // `U8` lowers to `Integer` with its limit as an entry, so a
        // `Vec<U8>` spot is a vector of integers here.
        Some("Vec")
            if matches!(&spot, ir::Type::Meaning { arguments, .. }
                if arguments.first().and_then(|element| {
                    prelude_label(&tables, element)
                }) == Some("Integer")) =>
        {
            false
        }
        _ => {
            return Err(String::from(
                "host input enters only a String or Vec<U8> spot",
            ))
        }
    };
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read the input: {error}"))?;
    if text {
        String::from_utf8(bytes)
            .map(|text| Some(Value::text(&text)))
            .map_err(|_| String::from("the input is not UTF-8 text"))
    } else {
        Ok(Some(Value::bytes(&bytes)))
    }
}

/// The `Host` Fault of a run refused before any effect, at the
/// program's last line.
#[must_use]
pub fn refusal(program: &Program, reason: &str) -> Fault {
    let tables = program.tables.borrow();
    let node = &tables.nodes[program.bodies[program.main].node];
    Fault {
        span: node.span,
        namespace: node.namespace,
        reason: Reason::Host(String::from(reason)),
    }
}

/// The exit code of a run that ended: 0 a value, 1 an `Err` value or
/// a Fault.
#[must_use]
pub fn exit_code(result: &Result<Value, Fault>, tables: &ir::Tables) -> u8 {
    match result {
        Err(_) => 1,
        Ok(Value::Record(record))
            if Some(record.meaning) == Prelude::new(tables).result
                && record.constructor == 1 =>
        {
            1
        }
        Ok(_) => 0,
    }
}

/// The label of a prelude meaning.
fn prelude_label<'t>(tables: &'t ir::Tables, ty: &ir::Type) -> Option<&'t str> {
    let ir::Type::Meaning { index, .. } = ty else {
        return None;
    };
    let meaning = &tables.meanings[*index];
    (tables.namespaces[meaning.namespace] == "prelude")
        .then_some(meaning.label.as_str())
}
