//! The conformance host: the documentation examples run under scripted
//! replies, each run's observation written as a trace and compared
//! byte for byte with its fixture.
use fipe::{
    check,
    decl::Declarations,
    embed::{Outcome, Reply},
    lex::lex,
    mir::Program,
    parse::parse,
    value::Value,
    vm::{Fault, Machine, Observation, Stop},
};

#[path = "host/examples.rs"]
mod examples;
#[path = "host/outputs.rs"]
mod outputs;
#[path = "host/replies.rs"]
mod replies;
#[path = "host/stepping.rs"]
mod stepping;
#[path = "host/trace.rs"]
mod trace;

/// The conformance host's declarations.
fn declarations() -> Declarations {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("fixtures/host/commands.fipe"))
        .unwrap();
    for (name, source) in [
        ("fs", include_str!("fixtures/host/fs.fipe")),
        ("sys", include_str!("fixtures/host/sys.fipe")),
        ("cli", include_str!("fixtures/host/cli.fipe")),
        ("text", include_str!("fixtures/host/text.fipe")),
        ("config", include_str!("fixtures/host/config.fipe")),
    ] {
        declarations.module(name, source).unwrap();
    }
    declarations
}

/// A source checked as one unit against the conformance host, with
/// the file modules it may `use`, lowered.
fn program(source: &str, files: &[(&str, &str)]) -> Program {
    let module = parse(lex(source)).unwrap();
    let files: Vec<_> = files
        .iter()
        .map(|(stem, text)| (String::from(*stem), parse(lex(text)).unwrap()))
        .collect();
    let unit = check::check(
        &module,
        &declarations(),
        &files,
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"));
    fipe::compile::compile(&unit)
}

/// Run a program under a budget, answering each host call from the
/// conformance host and applying its sink policy; the observation and
/// the units charged. `main` is the program's source, where a Fault in
/// it is located.
fn observe(
    program: &Program,
    budget: Option<u64>,
    main: &str,
) -> (Observation, u64) {
    let mut machine = Machine::new(budget);
    let mut host = replies::Host::default();
    let mut trace = Vec::new();
    let mut outcome = machine.run(program);
    let result = loop {
        match outcome {
            Outcome::Done(result) => break result,
            Outcome::Request(request) => {
                host.write_lines(machine.lines(), &program.tables.borrow());
                let reply = Reply::Value(
                    host.answer(&program.tables.borrow(), &request),
                );
                trace.push((request, reply.clone()));
                outcome = machine.reply(program, reply);
            }
        }
    };
    host.write_lines(machine.lines(), &program.tables.borrow());
    let observation = finish(program, main, result, trace, host);
    (observation, machine.charged())
}

/// The same run stepped stop by stop: the observation, the units
/// charged and every stop in order.
fn observe_stepped(
    program: &Program,
    budget: Option<u64>,
    main: &str,
) -> (Observation, u64, Vec<Stop>) {
    let mut machine = Machine::new(budget);
    let mut host = replies::Host::default();
    let mut trace = Vec::new();
    let mut stops = Vec::new();
    machine.begin(program);
    let result = loop {
        let stop = machine.next_stop();
        stops.push(stop.clone());
        match stop {
            Stop::Done(result) => break result,
            Stop::Request(request) => {
                host.write_lines(machine.lines(), &program.tables.borrow());
                let reply = Reply::Value(
                    host.answer(&program.tables.borrow(), &request),
                );
                trace.push((request, reply.clone()));
                machine.answer(reply);
            }
            Stop::Before(_) | Stop::After(..) => {}
        }
    };
    host.write_lines(machine.lines(), &program.tables.borrow());
    let observation = finish(program, main, result, trace, host);
    (observation, machine.charged(), stops)
}

/// The observation of a finished run: the host's output, its main
/// lines already written by the sink policy, and a Fault, located, to
/// the error output.
fn finish(
    program: &Program,
    main: &str,
    result: Result<Value, Fault>,
    trace: Vec<(fipe::embed::Request, Reply)>,
    host: replies::Host,
) -> Observation {
    let tables = program.tables.borrow();
    let success = host.success;
    let mut error = host.error;
    if let Err(fault) = &result {
        let sources = trace::Sources::of(main);
        error.extend(trace::located(fault, &tables, &sources).bytes());
        error.push(b'\n');
    }
    drop(tables);
    Observation {
        result,
        trace,
        success,
        error,
    }
}
