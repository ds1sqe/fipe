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
    let mut machine = Machine::new(Some(BUDGET));
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
    let mut machine = Machine::new(Some(BUDGET));
    match machine.run(&program) {
        Outcome::Done(Ok(value)) => {
            structural::show(&value, &program.tables.borrow())
        }
        Outcome::Done(Err(fault)) => format!("fault: {}", fault.reason),
        Outcome::Request(request) => panic!("{source}: {request:?}"),
    }
}

/// Lines accepted one by one through a session, each run on the one
/// machine, a line whose run Faults withdrawn: each line's value or
/// Fault, rendered.
pub fn session(lines: &[&str]) -> Vec<String> {
    let mut session = Session::new(Declarations::new());
    let mut machine = Machine::new(Some(BUDGET));
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
                Outcome::Done(Err(fault)) => {
                    let line = report.accepted.expect("an accepted line");
                    let withdrawn = session.withdraw(line).expect("the last");
                    machine.withdraw(&withdrawn);
                    format!("fault: {}", fault.reason)
                }
                Outcome::Request(request) => panic!("{line}: {request:?}"),
            }
        })
        .collect()
}

/// A unit's checked program, for runs under several budgets.
pub fn program(source: &str) -> fipe::mir::Program {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    compile::compile(&unit)
}

/// A program run under a budget: its result and the units it charged.
pub fn budgeted(
    program: &fipe::mir::Program,
    budget: u64,
) -> (Result<Value, Fault>, u64) {
    let mut machine = Machine::new(Some(budget));
    match machine.run(program) {
        Outcome::Done(result) => (result, machine.charged()),
        Outcome::Request(request) => panic!("{request:?}"),
    }
}

/// The source text a Fault's span covers.
pub fn spanned<'a>(source: &'a str, fault: &Fault) -> &'a str {
    &source[fault.span.start..fault.span.end]
}

/// The charge units a unit's run counted on a fresh machine.
pub fn charged(source: &str) -> u64 {
    budgeted(&program(source), BUDGET).1
}
