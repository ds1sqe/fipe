//! Helpers: run one source as a unit, or lines through a session, on a
//! machine; render a value in the language's spelling.
use super::{
    check, lex, parse, structural, Declarations, Fault, Machine, Outcome, Value,
};
use fipe::{compile, session::Session};

/// The test budget: enough for every row, too small to wait on a loop.
pub const BUDGET: u64 = 1 << 20;

/// A unit's program run on a fresh machine: its value, or its Fault.
pub fn run(source: &str) -> Result<Value, Fault> {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let program = compile::compile(&unit);
    let mut machine = Machine::new(BUDGET);
    match machine.run(&program) {
        Outcome::Done(result) => result,
        Outcome::Request(request) => panic!("{source}: {request:?}"),
    }
}

/// A unit run, rendered: the value in the language's spelling, or the
/// Fault's reason in words.
pub fn shown(source: &str) -> String {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let program = compile::compile(&unit);
    let mut machine = Machine::new(BUDGET);
    match machine.run(&program) {
        Outcome::Done(Ok(value)) => {
            structural::show(&value, &program.tables.borrow())
        }
        Outcome::Done(Err(fault)) => format!("fault: {}", fault.reason),
        Outcome::Request(request) => panic!("{source}: {request:?}"),
    }
}

/// Lines accepted one by one through a session, each run on the one
/// machine: each line's value or Fault, rendered.
pub fn session(lines: &[&str]) -> Vec<String> {
    let mut session = Session::new(Declarations::new());
    let mut machine = Machine::new(BUDGET);
    lines
        .iter()
        .map(|line| {
            let report = match session.line(line) {
                Outcome::Done(report) => report,
                Outcome::Request(request) => panic!("{line}: {request:?}"),
            };
            let unit = report
                .unit
                .unwrap_or_else(|| panic!("{line}: {:?}", report.diagnostics));
            let program = compile::compile(&unit);
            match machine.run(&program) {
                Outcome::Done(Ok(value)) => {
                    structural::show(&value, &program.tables.borrow())
                }
                Outcome::Done(Err(fault)) => format!("fault: {}", fault.reason),
                Outcome::Request(request) => panic!("{line}: {request:?}"),
            }
        })
        .collect()
}
