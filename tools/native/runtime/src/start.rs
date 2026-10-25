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

/// Install private test state without holding a borrow through runtime operations.
#[cfg(test)]
pub fn test_with_state<R>(state: State, run: impl FnOnce() -> R) -> (R, State) {
    struct Reset(Option<State>);
    impl Drop for Reset {
        fn drop(&mut self) {
            drop(STATE.replace(self.0.take()));
        }
    }
    let reset = Reset(STATE.replace(Some(state)));
    let result = run();
    let state = STATE.take().expect("the test run's state");
    drop(reset);
    (result, state)
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
        Host::Pipe { failed: true, .. } => {
            // /dev/fd/4 opens a duplicate. Only process exit closes every copy;
            // writing Done first could deadlock against a blocked reply writer.
            eprintln!("fipe: the host pipe failed; transport closed");
            2
        }
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
            if host.flush().is_err() {
                return 1;
            }
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
        .map(|(value, spot)| ops::misfit(value, spot))
        .transpose()
        .map_err(|reason| {
            STATE.with_borrow(|state| {
                let state = state.as_ref().expect("the run's state");
                ops::fault(&state.program, line, reason)
            })
        })?
        .flatten();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn generated(out: &mut Value) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        *out = Value::Int(1);
        0
    }

    fn raw_unit(header: &str) -> Unit {
        let mut declarations = fipe::decl::Declarations::new();
        declarations.module("private", header).unwrap();
        let main = fipe::parse::parse(fipe::lex::lex("1")).unwrap();
        let checked = fipe::check::check(
            &main,
            &declarations,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
        let program = fipe::compile::compile(&checked);
        Unit {
            sources: vec![
                ("main".into(), "1".into()),
                ("private".into(), header.into()),
            ],
            files: vec![],
            fingerprint: codec::fingerprint(&program),
        }
    }

    #[test]
    fn resource_reconstruction_refuses_a_matching_fingerprint_before_generated_code(
    ) {
        CALLS.store(0, Ordering::SeqCst);
        let marked = raw_unit("type resource Table");
        let decoded = Unit::decode(&marked.encode()).unwrap();
        assert_eq!(decoded, marked);
        assert_eq!(
            decoded.fingerprint,
            raw_unit("type resource Table").fingerprint
        );
        assert_eq!(
            decoded.compile().unwrap_err(),
            "native execution does not support resource declarations"
        );
        assert_eq!(started(&decoded.encode(), generated), 2);
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        let ordinary = raw_unit("type Packet");
        assert_eq!(
            codec::fingerprint(&ordinary.compile().unwrap()),
            ordinary.fingerprint
        );
        assert_eq!(started(&ordinary.encode(), generated), 0);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod input_work_tests {
    use super::*;
    use std::{
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
    };
    static ENTERED: AtomicBool = AtomicBool::new(false);
    fn body_must_not_start(_out: &mut Value) -> u32 {
        ENTERED.store(true, Ordering::SeqCst);
        0
    }

    #[test]
    fn data_scan_budget_fails_before_main_entry_or_line_effects() {
        let (_, program) = Unit::new(
            "use plain\nf: plain.Opaque -> Integer = _ => 7\nf",
            vec![("plain".into(), "pub type Opaque".into())],
            vec![],
        )
        .unwrap();
        let line = program.bodies[program.main].node;
        let spot = fipe::vm::input_spot(&program).unwrap();
        let input = Value::Tuple(Rc::new(vec![Value::Int(7), Value::Int(8)]));
        for allowance in [0, 4] {
            ENTERED.store(false, Ordering::SeqCst);
            STATE.set(Some(State::new(
                program.clone(),
                Some(allowance),
                Host::open().unwrap(),
            )));
            let result = ran(
                body_must_not_start,
                Some(input.clone()),
                Some(&spot),
                line,
            );
            let state = STATE.take().unwrap();
            assert!(matches!(
                result,
                Err(fipe::vm::Fault {
                    reason: fipe::value::Reason::Budget,
                    ..
                })
            ));
            assert!(!ENTERED.load(Ordering::SeqCst));
            assert_eq!(state.charged, allowance);
            assert!(state.lines.is_empty());
        }
        STATE.set(Some(State::new(program, Some(5), Host::open().unwrap())));
        let admitted = ops::misfit(&input, &spot);
        let state = STATE.take().unwrap();
        assert_eq!(admitted, Ok(None));
        assert_eq!(state.charged, 5);
    }
}
