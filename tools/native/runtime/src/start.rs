//! Startup: the unit the executable carries is compiled again with
//! `fipe`, its fingerprint compared with the one the backend recorded,
//! and the program runs on a thread of its own, whose stack
//! (`FIPE_STACK` bytes, 256 MiB by default) holds its recursion. The
//! budget is `FIPE_BUDGET` units, unbounded when unset. Without a host
//! pipe the std host answers in the process (`host`): the standard
//! input is host input when the last line takes it, and the output
//! follows the std host's sink policy and exit codes.
use crate::{
    codec::{self, Frame, Unit},
    host::Host,
    ops::{self, State},
};
use fipe::value::Value;
use std::{cell::RefCell, thread};

const STACK: usize = 256 << 20;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The state of the run on this thread.
///
/// # Panics
///
/// Outside a run, a defect of the generated code.
pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(|state| f(state.as_mut().expect("a run")))
}

/// Run the program: `run` calls the generated main with the slot of
/// its result and returns its status. The exit code: 0 when the run
/// ended with a value, 1 when it Faulted, 2 when it could not start.
/// An empty unit is an object with no Fipe source: `run` alone.
pub fn main(unit: &'static [u8], run: fn(&mut Value) -> u32) -> i32 {
    if unit.is_empty() {
        return i32::from(run(&mut Value::Unit) != 0);
    }
    let stack = std::env::var("FIPE_STACK")
        .ok()
        .and_then(|bytes| bytes.parse().ok())
        .unwrap_or(STACK);
    let thread = thread::Builder::new()
        .stack_size(stack)
        .spawn(move || started(unit, run));
    match thread.map(thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        _ => 2,
    }
}

fn started(unit: &[u8], run: fn(&mut Value) -> u32) -> i32 {
    let started = Unit::decode(unit)
        .map_err(|error| error.to_string())
        .and_then(|unit| {
            let program = unit.compile()?;
            if codec::fingerprint(&program) == unit.fingerprint {
                Ok((unit, program))
            } else {
                Err(String::from(
                    "the program does not match the one this executable \
                     was built from",
                ))
            }
        });
    let (unit, program) = match started {
        Ok(started) => started,
        Err(error) => {
            eprintln!("fipe: {error}");
            return 2;
        }
    };
    let budget = std::env::var("FIPE_BUDGET")
        .ok()
        .and_then(|units| units.parse().ok());
    let host = match Host::open() {
        Ok(host) => host,
        Err(error) => {
            eprintln!("fipe: {error}");
            return 2;
        }
    };
    let sources: Vec<(String, String)> =
        unit.sources.iter().chain(&unit.files).cloned().collect();
    // Under the std host the input is read before the run, so a refusal
    // leaves no effect.
    let input = if host.connected() {
        None
    } else {
        let read = fipe_host::input(&program, &mut std::io::stdin().lock());
        match read {
            Ok(input) => input,
            Err(reason) => {
                let fault = fipe_host::refusal(&program, &reason);
                let tables = program.tables.borrow();
                eprintln!("{}", fipe_host::located(&fault, &tables, &sources));
                return 2;
            }
        }
    };
    let line = program.bodies[program.main].node;
    let spot = fipe::vm::input_spot(&program).ok();
    let state = State::new(program, budget, host);
    STATE.set(Some(state));
    let result = ran(run, input, spot.as_ref(), line);
    let mut state = STATE.take().expect("the run's state");
    match &mut state.host {
        Host::Pipe { .. } => {
            let code = i32::from(result.is_err());
            let done = Frame::Done {
                result,
                lines: std::mem::take(&mut state.lines),
                charged: state.charged,
            };
            if let Err(error) = state.host.send(&done) {
                eprintln!("fipe: {error}");
                return 2;
            }
            code
        }
        Host::Std(host) => {
            let tables = state.program.tables.borrow();
            host.sink.lines(&state.lines, &tables);
            match &result {
                Err(fault) => host.sink.fault(fault, &tables, &sources),
                Ok(value) => host.sink.ending(value, &state.lines, &tables),
            }
            // A closed stream leaves nothing to report to.
            let _ = host.flush();
            i32::from(fipe_host::exit_code(&result, &tables))
        }
    }
}

/// The run on this thread's state: the generated main, then its value
/// applied to the host input when there is one. Input without the
/// shape of its spot Faults `Host` at the last line before the run,
/// as on the VM.
fn ran(
    run: fn(&mut Value) -> u32,
    input: Option<Value>,
    spot: Option<&fipe::ir::Type>,
    line: usize,
) -> Result<Value, fipe::vm::Fault> {
    let misfit = input
        .as_ref()
        .zip(spot)
        .and_then(|(value, spot)| ops::misfit(value, spot));
    if let Some(named) = misfit {
        return Err(STATE.with_borrow(|state| {
            let program = &state.as_ref().expect("the run's state").program;
            fipe_host::refusal(program, &format!("the input is not {named}"))
        }));
    }
    let mut out = Value::Unit;
    if run(&mut out) != 0 {
        return Err(STATE.with_borrow_mut(|state| {
            let state = state.as_mut().expect("the run's state");
            state.fault.take().expect("a Fault for status 1")
        }));
    }
    match input {
        Some(input) => applied(&out, input, line),
        None => Ok(out),
    }
}

/// The main's function value applied to the host input: its result
/// takes the function's place as the last line's value, as on the VM.
fn applied(
    function: &Value,
    input: Value,
    line: usize,
) -> Result<Value, fipe::vm::Fault> {
    STATE.with_borrow_mut(|state| {
        let state = state.as_mut().expect("the run's state");
        if state.lines.last().is_some_and(|(node, _)| *node == line) {
            state.lines.pop();
        }
    });
    let result = ops::apply(function, input, line);
    STATE.with_borrow_mut(|state| {
        let state = state.as_mut().expect("the run's state");
        match result {
            Ok(value) => {
                state.lines.push((line, value.clone()));
                Ok(value)
            }
            Err(ops::Faulted) => {
                Err(state.fault.take().expect("a Fault for status 1"))
            }
        }
    })
}
