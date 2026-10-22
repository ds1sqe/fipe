//! Startup: the unit the executable carries is compiled again with
//! `fipe`, its fingerprint compared with the one the backend recorded,
//! and the program runs on a thread of its own, whose stack
//! (`FIPE_STACK` bytes, 256 MiB by default) holds its recursion. The
//! budget is `FIPE_BUDGET` units, unbounded when unset.
use crate::{
    codec::{self, Frame, Unit},
    host::Host,
    ops::State,
};
use fipe::{
    token::Source,
    value::{structural, Value},
    vm::Fault,
};
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
    let state = State::new(program, budget, Host::open());
    STATE.set(Some(state));
    let mut out = Value::Unit;
    let status = run(&mut out);
    let mut state = STATE.take().expect("the run's state");
    let result = if status == 0 {
        Ok(out)
    } else {
        Err(state.fault.take().expect("a Fault for status 1"))
    };
    let code = i32::from(result.is_err());
    if state.host.connected() {
        let done = Frame::Done {
            result,
            lines: std::mem::take(&mut state.lines),
            charged: state.charged,
        };
        if let Err(error) = state.host.send(&done) {
            eprintln!("fipe: {error}");
            return 2;
        }
        return code;
    }
    let tables = state.program.tables.borrow();
    match result {
        Ok(value) => println!("{}", structural::show(&value, &tables)),
        Err(fault) => eprintln!("{}", located(&fault, &unit, &tables)),
    }
    code
}

/// A Fault located in its source, as the conformance host words it:
/// `fault in <namespace> at <line>:<column>: <reason>`.
fn located(fault: &Fault, unit: &Unit, tables: &fipe::ir::Tables) -> String {
    let namespace = &tables.namespaces[fault.namespace];
    let text = if namespace == "prelude" {
        Some(fipe::decl::prelude_source())
    } else {
        unit.source(namespace)
    };
    let (line, column) =
        Source::new(text.unwrap_or_default()).line_col(fault.span.start);
    format!("fault in {namespace} at {line}:{column}: {}", fault.reason)
}
