//! The VM is the oracle. A fixture compiles once; the VM runs it under
//! the conformance host, and the executable the backend builds runs as
//! a child under the same host, which this test is: it reads the
//! child's frames on the child's fd 3, answers on its fd 4, and applies
//! the sink policy. Both observations (the value or the located Fault,
//! the ordered host calls with their replies, the success and error
//! bytes) and the units charged must be equal.
use fipe::{
    embed::{Outcome, Reply, Request},
    mir::Program,
    value::{Reason, Value},
    vm::{Fault, Machine, Observation},
};
use fipe_native::{
    codec::{self, Frame, Unit},
    host_declarations, lower,
};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
};

mod common;
#[path = "../../../tests/host/replies.rs"]
mod replies;
#[path = "../../../tests/host/trace.rs"]
mod trace;

/// What the conformance host's `trace` reads besides the fixture.
mod examples {
    pub const GEOMETRY: &str =
        include_str!("../../../Documentation/examples/geometry.fipe");
}

const FIRST: &str = include_str!("fixtures/first.fipe");
const FILES: &str =
    include_str!("../../../Documentation/examples/01-files.fipe");

/// The other examples, each with its trace fixture.
const EXAMPLES: &[(&str, &str, &str)] = &[
    (
        "02-outcomes",
        include_str!("../../../Documentation/examples/02-outcomes.fipe"),
        include_str!("../../../tests/fixtures/traces/02-outcomes.txt"),
    ),
    (
        "03-types",
        include_str!("../../../Documentation/examples/03-types.fipe"),
        include_str!("../../../tests/fixtures/traces/03-types.txt"),
    ),
    (
        "04-text",
        include_str!("../../../Documentation/examples/04-text.fipe"),
        include_str!("../../../tests/fixtures/traces/04-text.txt"),
    ),
    (
        "05-modules",
        include_str!("../../../Documentation/examples/05-modules.fipe"),
        include_str!("../../../tests/fixtures/traces/05-modules.txt"),
    ),
    (
        "geometry",
        examples::GEOMETRY,
        include_str!("../../../tests/fixtures/traces/geometry.txt"),
    ),
];

/// One program per copied family, each run on the VM and natively.
const DRIFT: &[(&str, &str)] = &[
    ("closures", include_str!("fixtures/drift/closures.fipe")),
    ("guards", include_str!("fixtures/drift/guards.fipe")),
    (
        "returned-guards",
        include_str!("fixtures/drift/returned-guards.fipe"),
    ),
    (
        "higher-order",
        include_str!("fixtures/drift/higher-order.fipe"),
    ),
    ("dispatch", include_str!("fixtures/drift/dispatch.fipe")),
    ("witnesses", include_str!("fixtures/drift/witnesses.fipe")),
    (
        "conversions",
        include_str!("fixtures/drift/conversions.fipe"),
    ),
    ("gates", include_str!("fixtures/drift/gates.fipe")),
    ("sequences", include_str!("fixtures/drift/sequences.fipe")),
    ("packs", include_str!("fixtures/drift/packs.fipe")),
    (
        "environment",
        include_str!("fixtures/drift/environment.fipe"),
    ),
    ("modules", include_str!("fixtures/drift/modules.fipe")),
    ("prelude", include_str!("fixtures/drift/prelude.fipe")),
    ("functions", include_str!("fixtures/drift/functions.fipe")),
];

/// A fixture's unit under the conformance host, with the file module
/// `geometry` it may `use` (`tests/host.rs`), and its program.
fn unit(source: &str) -> (Unit, Program) {
    let host =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/host");
    let files = vec![("geometry".into(), examples::GEOMETRY.into())];
    Unit::new(source, host_declarations(&host).unwrap(), files).unwrap()
}

/// The VM's run under a budget, answered by the conformance host: the
/// observation and the units charged (`tests/host.rs`).
fn on_vm(
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

/// The executable's run under a budget, answered by the conformance
/// host over the pipe: the observation, the units charged and the exit
/// code. `sh` puts the pipe on fds 3 and 4.
fn native(
    executable: &Path,
    program: &Program,
    budget: Option<u64>,
    main: &str,
) -> (Observation, u64, i32) {
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec \"$0\" 3>&1 4<&0 1>&2 0</dev/null"])
        .arg(executable)
        .env_remove("FIPE_BUDGET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(budget) = budget {
        command.env("FIPE_BUDGET", budget.to_string());
    }
    let mut child = command.spawn().unwrap();
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut host = replies::Host::default();
    let mut trace = Vec::new();
    let (result, charged) = loop {
        let mut length = [0; 8];
        frames.read_exact(&mut length).unwrap();
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call { request, lines } => {
                host.write_lines(&lines, &program.tables.borrow());
                let answer = host.answer(&program.tables.borrow(), &request);
                let reply = codec::encode_reply(&answer).unwrap();
                replies
                    .write_all(&(reply.len() as u64).to_le_bytes())
                    .unwrap();
                replies.write_all(&reply).unwrap();
                trace.push((request, Reply::Value(answer)));
            }
            Frame::Done {
                result,
                lines,
                charged,
            } => {
                host.write_lines(&lines, &program.tables.borrow());
                break (result, charged);
            }
        }
    };
    drop(replies);
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let code = output.status.code().unwrap();
    (finish(program, main, result, trace, host), charged, code)
}

/// The observation of a finished run: the host's output, its main
/// lines already written by the sink policy, and a Fault, located, to
/// the error output (`tests/host.rs`).
fn finish(
    program: &Program,
    main: &str,
    result: Result<Value, Fault>,
    trace: Vec<(Request, Reply)>,
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

fn traced(program: &Program, observation: &Observation, main: &str) -> String {
    let sources = trace::Sources::of(main);
    trace::write(observation, &program.tables.borrow(), &sources)
}

#[test]
fn first_observes_and_charges_as_on_the_vm() {
    let (unit, program) = unit(FIRST);
    let executable = common::executable("first", &unit, &program);
    let (vm, vm_units) = on_vm(&program, None, FIRST);
    let (native, units, code) = native(&executable, &program, None, FIRST);
    assert_eq!(
        traced(&program, &native, FIRST),
        traced(&program, &vm, FIRST)
    );
    assert_eq!(native, vm);
    assert_eq!(units, vm_units);
    // `fact 25` past `I64`, the case, one host call, then the division
    // by zero, a Fault: exit code 1.
    let fault = native.result.as_ref().unwrap_err();
    assert_eq!(fault.reason, Reason::DivideByZero);
    assert_eq!(native.trace.len(), 1);
    assert_eq!(
        String::from_utf8_lossy(&native.success),
        "15511210043330985984000000\n33\n"
    );
    assert_eq!(code, 1);
}

#[test]
fn under_a_budget_both_fault_at_one_span_with_equal_units() {
    let (unit, program) = unit(FIRST);
    let executable = common::executable("first_budget", &unit, &program);
    let (_, whole) = on_vm(&program, None, FIRST);
    for budget in [0, 1, 50, whole / 2, whole - 1, whole] {
        let (vm, vm_units) = on_vm(&program, Some(budget), FIRST);
        let (native, units, _) =
            native(&executable, &program, Some(budget), FIRST);
        assert_eq!(native, vm, "budget {budget}");
        assert_eq!(units, vm_units, "budget {budget}");
        if budget == 50 {
            let fault = native.result.as_ref().unwrap_err();
            assert_eq!(fault.reason, Reason::Budget);
            assert_eq!(units, 50);
            assert_eq!(native.trace.len(), 0);
        }
    }
}

/// A result as the pipe carries it: a function in it becomes the
/// placeholder the host shows as `<function>`, the rest is unchanged.
fn carried(result: Result<Value, Fault>) -> Result<Value, Fault> {
    let done = Frame::Done {
        result,
        lines: Vec::new(),
        charged: 0,
    };
    let Frame::Done { result, .. } =
        Frame::decode(&done.encode().unwrap()).unwrap()
    else {
        unreachable!("a Done frame decodes to one")
    };
    result
}

/// A program run on the VM and natively, unbounded and under budgets
/// that stop it partway: equal observations (their traces too) and
/// units each time. The native observation of the unbounded run, its
/// units and its exit code.
fn equal(name: &str, source: &str) -> (Observation, u64, i32) {
    let (unit, program) = unit(source);
    let executable = common::executable(name, &unit, &program);
    let (mut vm, whole) = on_vm(&program, None, source);
    vm.result = carried(vm.result);
    let (observed, units, code) = native(&executable, &program, None, source);
    assert_eq!(
        traced(&program, &observed, source),
        traced(&program, &vm, source),
        "{name}"
    );
    assert_eq!(observed, vm, "{name}");
    assert_eq!(units, whole, "{name}");
    for budget in [whole / 3, whole / 2, whole.saturating_sub(1)] {
        let (mut vm, vm_units) = on_vm(&program, Some(budget), source);
        vm.result = carried(vm.result);
        let (cut, units, _) =
            native(&executable, &program, Some(budget), source);
        assert_eq!(cut, vm, "{name} under budget {budget}");
        assert_eq!(units, vm_units, "{name} under budget {budget}");
    }
    (observed, units, code)
}

#[test]
fn files_observes_as_on_the_vm_and_replays_its_trace() {
    let (observed, units, code) = equal("files", FILES);
    let (_, program) = unit(FILES);
    let fixture = include_str!("../../../tests/fixtures/traces/01-files.txt");
    assert_eq!(traced(&program, &observed, FILES), fixture);
    assert!(units > 0);
    assert_eq!(code, 0);
}

#[test]
fn each_drift_fixture_observes_as_on_the_vm() {
    for (name, source) in DRIFT {
        let (observed, _, code) = equal(name, source);
        assert_eq!(code, i32::from(observed.result.is_err()), "{name}");
        // The guards' fixtures end on the argument a guard refutes, the
        // conversions' on a checked conversion that fails.
        match (*name, observed.result) {
            ("guards" | "returned-guards", Err(fault)) => assert!(
                matches!(&fault.reason, Reason::Failed { value, .. }
                    if value == "300"),
                "{name}: {fault:?}"
            ),
            ("conversions", Err(fault)) => {
                assert_eq!(fault.reason, Reason::Cast);
            }
            // A function shows as on the VM, also in the lines a host
            // call carries.
            ("functions", Ok(value)) => {
                assert_eq!(value, codec::function());
                let success = String::from_utf8_lossy(&observed.success);
                assert!(
                    success.starts_with(
                        "<function>\n(<function>, 1)\nOp \"inc\" <function>\n"
                    ),
                    "{success}"
                );
            }
            (_, result) => assert!(result.is_ok(), "{name}: {result:?}"),
        }
    }
}

#[test]
fn the_listing_of_first_is_stable() {
    let (_, program) = unit(FIRST);
    common::golden("first", &lower::listing(&program));
}

#[test]
fn each_example_observes_as_on_the_vm_and_replays_its_trace() {
    for (name, source, fixture) in EXAMPLES {
        let (observed, units, code) = equal(name, source);
        let (_, program) = unit(source);
        assert_eq!(traced(&program, &observed, source), *fixture, "{name}");
        // `geometry` alone declares; it charges nothing.
        assert_eq!(units == 0, *name == "geometry", "{name}");
        assert_eq!(code, i32::from(observed.result.is_err()), "{name}");
    }
}

#[test]
fn a_recursion_without_end_faults_budget_at_the_vm_s_span() {
    let source = "r = n => r (n + 1)\nr 0";
    let (unit, program) = unit(source);
    let executable = common::executable("recursion", &unit, &program);
    for budget in [1000, 100_000] {
        let (vm, vm_units) = on_vm(&program, Some(budget), source);
        let (native, units, code) =
            native(&executable, &program, Some(budget), source);
        assert_eq!(native, vm, "budget {budget}");
        assert_eq!(units, budget);
        assert_eq!(vm_units, budget);
        let fault = native.result.as_ref().unwrap_err();
        assert_eq!(fault.reason, Reason::Budget);
        assert_eq!(code, 1);
    }
}

#[test]
fn a_gate_lowers_and_branches_as_on_the_vm() {
    // Once a typed error, a `Test` terminator now lowers: every form of
    // the MIR does.
    let gate = "type Even: Integer where self % 2 == 0\n\
                m: Integer -> Integer = { Even n => n, _ => 0 }\n\
                (m 4, m 3)";
    let (observed, _, code) = equal("gate", gate);
    assert_eq!(code, 0);
    let (_, program) = unit(gate);
    let shown = fipe::value::structural::show(
        observed.result.as_ref().unwrap(),
        &program.tables.borrow(),
    );
    assert_eq!(shown, "(4, 0)");
}

#[test]
fn every_value_crosses_the_pipe_a_function_only_where_shown() {
    use fipe::{
        token::Span,
        value::{Packed, Record},
    };
    use std::rc::Rc;
    let big = fipe::value::arith::mul(&Value::Int(i64::MAX), &Value::Int(-3));
    let record = Value::Record(Rc::new(Record {
        meaning: 4,
        constructor: 1,
        fields: vec![Value::text("é"), Value::F32(1.5), Value::F64(-0.25)],
    }));
    let parts = vec![
        Value::Unit,
        Value::Int(i64::MIN),
        big,
        record,
        Value::Tuple(Rc::new(vec![Value::bytes(b"ab"), Value::seq(vec![])])),
        Value::Packed(Rc::new(Packed {
            meaning: 9,
            value: Value::Int(1),
        })),
    ];
    let frame = Frame::Call {
        request: Request::Call {
            declaration: 3,
            arguments: parts.clone(),
            environment: vec![(2, Value::text("cwd"))],
        },
        lines: vec![(7, Value::seq(parts))],
    };
    let bytes = frame.encode().unwrap();
    assert_eq!(Frame::decode(&bytes), Ok(frame));
    for reason in [
        Reason::Budget,
        Reason::NotExact {
            value: "1.5".into(),
            target: "Integer".into(),
        },
        Reason::Failed {
            entry: 2,
            value: "300".into(),
        },
        Reason::Host("no entry".into()),
    ] {
        let done = Frame::Done {
            result: Err(Fault {
                span: Span { start: 3, end: 9 },
                namespace: 1,
                reason,
            }),
            lines: Vec::new(),
            charged: 42,
        };
        assert_eq!(Frame::decode(&done.encode().unwrap()), Ok(done));
    }
    let reply = Err(String::from("no host"));
    let bytes = codec::encode_reply(&reply).unwrap();
    assert_eq!(codec::decode_reply(&bytes), Ok(reply));
    assert_eq!(Frame::decode(&bytes[..4]), Err(codec::Error::Short));
    // A function crosses only where the host shows it: in the lines,
    // not in a request's arguments or a reply.
    let function = Value::Tuple(Rc::new(vec![codec::function()]));
    let call = |arguments, lines| Frame::Call {
        request: Request::Call {
            declaration: 0,
            arguments,
            environment: Vec::new(),
        },
        lines,
    };
    let shown = call(Vec::new(), vec![(1, function.clone())]);
    assert_eq!(Frame::decode(&shown.encode().unwrap()), Ok(shown));
    let asked = call(vec![function.clone()], Vec::new()).encode();
    assert_eq!(asked, Err(codec::Error::Opaque));
    let replied = codec::encode_reply(&Ok(function));
    assert_eq!(replied, Err(codec::Error::Opaque));
    let mut forged = codec::encode_reply(&Ok(Value::Unit)).unwrap();
    forged[8] = 10;
    assert_eq!(codec::decode_reply(&forged), Err(codec::Error::Tag(10)));
    let witness = Value::Meaning(Rc::new(fipe::ir::Type::Unit));
    let opaque = codec::encode_reply(&Ok(witness));
    assert_eq!(opaque, Err(codec::Error::Opaque));
}
