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

// Every pipe owner reaps its child on early framing/write failure or assertion.
struct PipeChild(Option<std::process::Child>);
impl PipeChild {
    const fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }
    fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.0.take().unwrap().wait_with_output()
    }
}
impl std::ops::Deref for PipeChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for PipeChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}
impl Drop for PipeChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn pipe_reply(
    output: &mut impl Write,
    reply: &Result<Value, String>,
) -> Result<(), String> {
    // Serialization/size refusal occurs before even the eight-byte header.
    let encoded =
        codec::encode_reply(reply).map_err(|error| error.to_string())?;
    pipe_payload(output, &encoded)
}
fn pipe_payload(output: &mut impl Write, encoded: &[u8]) -> Result<(), String> {
    let length = u64::try_from(encoded.len()).unwrap();
    if length > codec::MAX_REPLY_BYTES {
        return Err(codec::Error::TooLong(length).to_string());
    }
    output
        .write_all(&length.to_le_bytes())
        .and_then(|()| output.write_all(encoded))
        .map_err(|error| error.to_string())
}

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
    sources: &trace::Sources<'_>,
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
    let observation = finish(program, sources, result, trace, host);
    (observation, machine.charged())
}

/// The executable's run under a budget, answered by the conformance
/// host over the pipe: the observation, the units charged and the exit
/// code. `sh` puts the pipe on fds 3 and 4.
fn native(
    executable: &Path,
    program: &Program,
    budget: Option<u64>,
    sources: &trace::Sources<'_>,
) -> (Observation, u64, i32) {
    let stderr_path = executable.with_extension("stderr");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec \"$0\" 3>&1 4<&0 1>&2 0</dev/null"])
        .arg(executable)
        .env_remove("FIPE_BUDGET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr));
    if let Some(budget) = budget {
        command.env("FIPE_BUDGET", budget.to_string());
    }
    let mut child = PipeChild::new(command.spawn().unwrap());
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut host = replies::Host::default();
    let mut trace = Vec::new();
    let (result, charged) = loop {
        let mut length = [0; 8];
        if let Err(error) = frames.read_exact(&mut length) {
            drop(replies);
            drop(frames);
            let mut supervisor_killed = false;
            let mut kill_error = None;
            let status = child.try_wait().unwrap().unwrap_or_else(|| {
                match child.kill() {
                    Ok(()) => supervisor_killed = true,
                    Err(error) => kill_error = Some(error.to_string()),
                }
                child.wait().unwrap()
            });
            let mut stderr = Vec::new();
            std::fs::File::open(&stderr_path)
                .unwrap()
                .take(65_536)
                .read_to_end(&mut stderr)
                .unwrap();
            let stderr = String::from_utf8_lossy(&stderr);
            panic!(
                "native frame header: {error}; executable={}; budget={budget:?}; status={status}; supervisor_killed={supervisor_killed}; kill_error={kill_error:?}; stderr(first64KiB)={stderr:?}",
                executable.display()
            );
        }
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call { request, lines } => {
                host.write_lines(&lines, &program.tables.borrow());
                let answer = host.answer(&program.tables.borrow(), &request);
                if let Err(error) = pipe_reply(&mut replies, &answer) {
                    drop(replies);
                    drop(frames);
                    drop(child);
                    panic!("native reply failed before completion: {error}");
                }
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
    assert_eq!(std::fs::read(&stderr_path).unwrap(), b"");
    let code = output.status.code().unwrap();
    (finish(program, sources, result, trace, host), charged, code)
}

/// The observation of a finished run: the host's output, its main
/// lines already written by the sink policy, and a Fault, located, to
/// the error output (`tests/host.rs`).
fn finish(
    program: &Program,
    sources: &trace::Sources<'_>,
    result: Result<Value, Fault>,
    trace: Vec<(Request, Reply)>,
    host: replies::Host,
) -> Observation {
    let tables = program.tables.borrow();
    let success = host.success;
    let mut error = host.error;
    if let Err(fault) = &result {
        error.extend(trace::located(fault, &tables, sources).bytes());
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

fn traced(
    program: &Program,
    observation: &Observation,
    sources: &trace::Sources<'_>,
) -> String {
    trace::write(observation, &program.tables.borrow(), sources)
}

#[test]
fn first_observes_and_charges_as_on_the_vm() {
    let (unit, program) = unit(FIRST);
    let executable = common::executable("first", &unit, &program);
    let (vm, vm_units) = on_vm(&program, None, &trace::Sources::of(FIRST));
    let (native, units, code) =
        native(&executable, &program, None, &trace::Sources::of(FIRST));
    assert_eq!(
        traced(&program, &native, &trace::Sources::of(FIRST)),
        traced(&program, &vm, &trace::Sources::of(FIRST))
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
    let (_, whole) = on_vm(&program, None, &trace::Sources::of(FIRST));
    for budget in [0, 1, 50, whole / 2, whole - 1, whole] {
        let (vm, vm_units) =
            on_vm(&program, Some(budget), &trace::Sources::of(FIRST));
        let (native, units, _) = native(
            &executable,
            &program,
            Some(budget),
            &trace::Sources::of(FIRST),
        );
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
    equal_prepared(name, source, &unit, &program)
}

fn equal_prepared(
    name: &str,
    source: &str,
    unit: &Unit,
    program: &Program,
) -> (Observation, u64, i32) {
    assert_eq!(
        unit.sources
            .iter()
            .find(|(name, _)| name == "main")
            .map(|(_, text)| text.as_str()),
        Some(source)
    );
    // The actual unit owns declaration/file texts; no geometry placeholder may
    // shadow a supplied source when locating a budget Fault in a written body.
    let sources = trace::Sources(
        unit.sources
            .iter()
            .chain(&unit.files)
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect(),
    );
    let executable = common::executable(name, unit, program);
    let (mut vm, whole) = on_vm(program, None, &sources);
    vm.result = carried(vm.result);
    let (observed, units, code) = native(&executable, program, None, &sources);
    assert_eq!(
        traced(program, &observed, &sources),
        traced(program, &vm, &sources),
        "{name}"
    );
    assert_eq!(observed, vm, "{name}");
    assert_eq!(units, whole, "{name}");
    for budget in [whole / 3, whole / 2, whole.saturating_sub(1)] {
        let (mut vm, vm_units) = on_vm(program, Some(budget), &sources);
        vm.result = carried(vm.result);
        let (cut, units, _) =
            native(&executable, program, Some(budget), &sources);
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
    assert_eq!(
        traced(&program, &observed, &trace::Sources::of(FILES)),
        fixture
    );
    assert!(units > 0);
    assert_eq!(code, 0);
}

#[test]
fn each_drift_fixture_observes_as_on_the_vm() {
    for (name, source) in DRIFT {
        eprintln!("DRIFT BEGIN {name}");
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
    common::golden("first", &lower::listing(&program).unwrap());
}

#[test]
fn each_example_observes_as_on_the_vm_and_replays_its_trace() {
    for (name, source, fixture) in EXAMPLES {
        let (observed, units, code) = equal(name, source);
        let (_, program) = unit(source);
        assert_eq!(
            traced(&program, &observed, &trace::Sources::of(source)),
            *fixture,
            "{name}"
        );
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
        let (vm, vm_units) =
            on_vm(&program, Some(budget), &trace::Sources::of(source));
        let (native, units, code) = native(
            &executable,
            &program,
            Some(budget),
            &trace::Sources::of(source),
        );
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
        Value::Packed(Rc::new(
            Packed::from_data(
                fipe::ir::Type::Meaning {
                    index: 9,
                    arguments: vec![fipe::ir::Type::Unit],
                },
                fipe::ir::Type::Meaning {
                    index: 10,
                    arguments: vec![],
                },
                &Value::Int(1),
            )
            .unwrap(),
        )),
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
    let witness = Value::Meaning(Rc::new(fipe::value::Meaning::new(
        fipe::ir::Type::Unit,
    )));
    let opaque = codec::encode_reply(&Ok(witness));
    assert_eq!(opaque, Err(codec::Error::Opaque));
}

/// Compile directly through Core, so Unit's earlier gate cannot hide an emitter bypass.
fn classified_program(header: &str) -> Program {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("private", header).unwrap();
    let module = fipe::parse::parse(fipe::lex::lex("1")).unwrap();
    let checked = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    fipe::compile::compile(&checked)
}

#[test]
fn resource_program_emitters_refuse_private_unused_declarations() {
    let program = classified_program("type resource Table");
    let tables = program.tables.borrow();
    assert!(tables.has_resources);
    assert!(tables.meanings.iter().all(|meaning| !meaning.resource));
    drop(tables);
    let expected = "native execution does not support resource declarations";
    assert_eq!(codec::ensure_resource_free(&program).unwrap_err(), expected);
    assert_eq!(lower::lower(&program).unwrap_err(), expected);
    assert_eq!(lower::object(&program, b"").unwrap_err(), expected);
    assert_eq!(lower::listing(&program).unwrap_err(), expected);
    let ordinary = classified_program("type Packet");
    assert!(!ordinary.tables.borrow().has_resources);
    codec::ensure_resource_free(&ordinary).unwrap();
    assert!(!lower::lower(&ordinary).unwrap().is_empty());
    assert!(!lower::listing(&ordinary).unwrap().is_empty());
    assert!(!lower::object(&ordinary, b"").unwrap().text.is_empty());
}

#[test]
fn resource_unit_metadata_roundtrips_but_cannot_compile() {
    let host = vec![("private".into(), "type resource Table".into())];
    let expected = "native execution does not support resource declarations";
    assert_eq!(Unit::new("1", host.clone(), vec![]).unwrap_err(), expected);
    let program = classified_program("type resource Table");
    let unit = Unit {
        sources: vec![("main".into(), "1".into()), host[0].clone()],
        files: vec![],
        fingerprint: codec::fingerprint(&program),
    };
    let decoded = Unit::decode(&unit.encode()).unwrap();
    assert_eq!(decoded, unit);
    assert_eq!(decoded.source("private"), Some("type resource Table"));
    assert_eq!(unit.compile().unwrap_err(), expected);
    assert_eq!(decoded.compile().unwrap_err(), expected);
    let (ordinary, _) =
        Unit::new("1", vec![("private".into(), "type Packet".into())], vec![])
            .unwrap();
    Unit::decode(&ordinary.encode()).unwrap().compile().unwrap();
}

fn resource_value_boundary_fixture() -> (Value, Value) {
    let mut host = fipe::decl::Declarations::new();
    host.module("csv", "pub type resource Table\npub get: Table\npub echo: Eq -> Eq\nimpl Table: Eq { eq: Self -> (self: Self) -> Bool }")
        .unwrap();
    let module =
        fipe::parse::parse(fipe::lex::lex("use csv\ncsv.echo csv.get"))
            .unwrap();
    let unit = fipe::check::check(
        &module,
        &host,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let ty = fipe::value::resource::ResourceType::registered(
        &unit.tables(),
        "csv",
        "Table",
    )
    .unwrap();
    let resource = fipe::value::resource::ResourceDomain::new()
        .issue(&ty, std::rc::Rc::from(&b"native-refusal-private-entry"[..]));
    let program = fipe::compile::compile(&unit);
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let Outcome::Request(Request::Call { arguments, .. }) =
        machine.reply(&program, Reply::Value(Ok(resource.clone())))
    else {
        panic!("checked packed resource")
    };
    let [packed @ Value::Packed(_)] = arguments.as_slice() else {
        panic!("actual checked Pack")
    };
    (resource, packed.clone())
}
fn resource_value_boundary_functions(value: Value, filled: bool) -> Value {
    Value::Function(std::rc::Rc::new(fipe::value::Closure {
        body: usize::MAX,
        captures: if filled { vec![] } else { vec![value.clone()] },
        meanings: vec![],
        filled: if filled { vec![value] } else { vec![] },
        guards: vec![],
    }))
}
fn resource_value_boundary_assert_roots(value: &Value) {
    use fipe::token::Span;
    let call = |arguments, environment, lines| Frame::Call {
        request: Request::Call {
            declaration: 0,
            arguments,
            environment,
        },
        lines,
    };
    let bad = [
        call(vec![value.clone()], vec![], vec![]),
        call(vec![], vec![(0, value.clone())], vec![]),
        call(vec![], vec![], vec![(0, value.clone())]),
        Frame::Done {
            result: Ok(value.clone()),
            lines: vec![],
            charged: 0,
        },
        Frame::Done {
            result: Ok(Value::Unit),
            lines: vec![(0, value.clone())],
            charged: 0,
        },
        Frame::Done {
            result: Err(Fault {
                span: Span::default(),
                namespace: 0,
                reason: Reason::Budget,
            }),
            lines: vec![(0, value.clone())],
            charged: 0,
        },
    ];
    for frame in bad {
        assert_eq!(frame.encode(), Err(codec::Error::Opaque));
    }
    assert_eq!(
        codec::encode_reply(&Ok(value.clone())),
        Err(codec::Error::Opaque)
    );
}
#[test]
fn resource_value_boundary_refuses_every_root_and_hidden_function_slot() {
    use fipe::value::{Packed, Record};
    use std::rc::Rc;
    let (resource, packed) = resource_value_boundary_fixture();
    assert!(Packed::from_data(
        fipe::ir::Type::Unit,
        fipe::ir::Type::Unit,
        &resource
    )
    .is_none());
    let values = [
        resource.clone(),
        Value::seq(vec![resource.clone()]),
        Value::Tuple(Rc::new(vec![resource.clone()])),
        Value::Record(Rc::new(Record {
            meaning: 0,
            constructor: 0,
            fields: vec![resource.clone()],
        })),
        packed,
        resource_value_boundary_functions(resource.clone(), false),
        resource_value_boundary_functions(resource, true),
    ];
    for value in values {
        resource_value_boundary_assert_roots(&value);
    }
    for filled in [false, true] {
        let ordinary = resource_value_boundary_functions(Value::Int(7), filled);
        let frame = Frame::Done {
            result: Ok(Value::Unit),
            lines: vec![(0, ordinary)],
            charged: 0,
        };
        let decoded = Frame::decode(&frame.encode().unwrap()).unwrap();
        assert_eq!(
            decoded,
            Frame::Done {
                result: Ok(Value::Unit),
                lines: vec![(0, codec::function())],
                charged: 0
            }
        );
    }
}
#[test]
fn hidden_meaning_witnesses_link_and_run() {
    let (observed, units, code) =
        equal("witnesses", include_str!("fixtures/drift/witnesses.fipe"));
    assert!(observed.result.is_ok());
    assert!(units > 0);
    assert_eq!(code, 0);
}

#[test]
fn bound_members_align_the_receiver_to_the_selected_row() {
    let source = "type P: Ord { value: Integer }\nimpl P: Ord { cmp = self other => if self.value < other.value { Less } else { Greater } }\ntype Q: Ord { value: Integer }\nimpl Q: Ord { cmp = other self => if self.value < other.value { Less } else { Greater } }\ncompare: Ord a => a -> a -> Ordering = a b => a.cmp b\nalias: Ord a => a -> a -> Ordering = a b => { f = a.cmp\nf b }\n(compare 1 2, compare (P 1) (P 2), compare (Q 1) (Q 2), alias (P 1) (P 2), (P 1).cmp (P 2))";
    let (observed, units, code) = equal("bound_members", source);
    let result = observed.result.unwrap();
    let Value::Tuple(parts) = result else {
        panic!("expected comparison tuple")
    };
    assert_eq!(parts.len(), 5);
    assert!(parts.iter().all(|value| matches!(value,Value::Record(record) if record.constructor == 0 && record.fields.is_empty())));
    assert!(units > 0);
    assert_eq!(code, 0);
}

#[test]
fn packed_written_wrapper_ord_and_nested_layers_match_the_vm() {
    fn layers(value: &Value, program: &Program, same_alias: bool) {
        let Value::Tuple(parts) = value else {
            panic!("nested result tuple")
        };
        assert_eq!(parts[2], Value::Int(7));
        let [Value::Packed(child), Value::Packed(shape), _] = parts.as_slice()
        else {
            panic!("both actual wrappers")
        };
        let tables = program.tables.borrow();
        let named = |label: &str| fipe::ir::Type::Meaning {
            index: tables
                .meanings
                .iter()
                .position(|meaning| meaning.label == label)
                .unwrap(),
            arguments: vec![],
        };
        assert_eq!(child.source_type(), &named("P"));
        assert_eq!(child.target_type(), &named("Child"));
        assert_eq!(shape.source_type(), &named("Child"));
        assert_eq!(shape.target_type(), &named("Shape"));
        let Value::Packed(retained) = shape.value() else {
            panic!("outer wrapper retains Child")
        };
        assert_eq!(retained.source_type(), child.source_type());
        assert_eq!(retained.target_type(), child.target_type());
        if same_alias {
            assert!(std::rc::Rc::ptr_eq(retained, child));
        }
    }

    for (name, member, header, source, expected) in [
        ("packed_written_wrapper_ord_precedes_payload_order", "cmp", "pub type Key: Ord\nimpl Key: Ord { eq: Self -> (self: Self) -> Bool = other self => False, cmp: Self -> (self: Self) -> Ordering = other self => Greater }", "use probe\ntype P: probe.Key { n: Integer }\na: probe.Key = P 1\nb: probe.Key = P 2\na < b", "False"),
        ("packed_nested_written_wrapper_eq_precedes_payload_inequality", "eq", "pub type Key: Eq\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool = other self => True }", "use probe\ntype P: probe.Key { n: Integer }\nx: probe.Key = P 1\ny: probe.Key = P 2\n(x, 0) == (y, 0)", "True"),
        ("packed_nested_written_wrapper_show_preserves_argument_parentheses", "show", "pub type Key: Show\nimpl Key: Show { show: (self: Self) -> String = self => \"key word\" }", "use probe\ntype P: probe.Key { n: Integer }\ntype Holder: Show { item: probe.Key }\nx: probe.Key = P 7\nshow (x, Holder x)", "\"(key word, Holder (key word))\""),
    ] {
        let (unit, program) = Unit::new(source, vec![("probe".into(), header.into())], vec![]).unwrap();
        {
            let tables = program.tables.borrow();
            let key = tables.meanings.iter().position(|meaning| meaning.label == "Key" && tables.namespaces[meaning.namespace] == "probe").unwrap();
            assert!(tables.dispatch.iter().filter(|dispatch| dispatch.label == member).flat_map(|dispatch| &dispatch.rows).any(|row| {
                matches!(row.receiver, fipe::ir::Type::Meaning { index, .. } if index == key)
                    && matches!(row.target, fipe::ir::Implementation::Definition { index, .. } if program.definitions[index].is_some())
            }), "the actual wrapper owns a reached written body");
        }
        let (observed, _, code) = equal_prepared(name, source, &unit, &program);
        assert_eq!(code, 0);
        assert!(observed.trace.is_empty(), "written wrapper bodies require no Host callback");
        assert_eq!(fipe::value::structural::show(observed.result.as_ref().unwrap(), &program.tables.borrow()), expected);
    }

    let source = "type Shape where self.area: Integer\ntype Child: Shape & Show\ntype P: Child { n: Integer }\nimpl P: Shape { area = self => self.n }\nimpl P: Show { show = self => self.n.show }\nx: Child = P 7\ny: Shape = x\n(x, y, y.area)";
    let (unit, program) = unit(source);
    let Outcome::Done(Ok(value)) = Machine::new(None).run(&program) else {
        panic!("actual VM nested producer")
    };
    layers(&value, &program, true);
    let (observed, _, code) =
        equal_prepared("packed_nested_layers", source, &unit, &program);
    assert_eq!(code, 0);
    assert!(observed.trace.is_empty());
    layers(observed.result.as_ref().unwrap(), &program, false);
}

#[test]
fn packed_constant_targets_survive_compilation_and_codec() {
    let source = "type A: Eq & Show\ntype B: Eq & Show\ntype P: A & B { n: Integer }\nx: A = P 7\ny: B = P 7\nCompile (x, y)";
    let (observed, _, code) = equal("packed_constant_targets", source);
    assert_eq!(code, 0);
    let value = observed.result.unwrap();
    let Value::Tuple(parts) = &value else {
        panic!("constant tuple")
    };
    let [Value::Packed(a), Value::Packed(b)] = parts.as_slice() else {
        panic!("two constant packs")
    };
    assert_eq!(a.source_type(), b.source_type());
    assert_ne!(
        a.target_type(),
        b.target_type(),
        "constant interning must preserve each target"
    );
    assert_eq!(a, b, "targets do not affect language equality");
    let frame = Frame::Done {
        result: Ok(value.clone()),
        lines: vec![],
        charged: 0,
    };
    let Frame::Done {
        result: Ok(Value::Tuple(decoded)),
        ..
    } = Frame::decode(&frame.encode().unwrap()).unwrap()
    else {
        panic!("ordinary data-only constant codec")
    };
    let [Value::Packed(da), Value::Packed(db)] = decoded.as_slice() else {
        panic!("decoded packs")
    };
    assert_eq!(da.source_type(), a.source_type());
    assert_eq!(da.target_type(), a.target_type());
    assert_eq!(db.target_type(), b.target_type());
    assert_eq!(a, da);
    assert_eq!(da, b);
    assert_eq!(a, b, "descriptive constant equality must be transitive");

    let runtime = source.replace("Compile (x, y)", "(x, y)");
    let (_, program) = unit(&runtime);
    assert!(
        program
            .bodies
            .iter()
            .flat_map(|body| &body.blocks)
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(
                statement,
                fipe::mir::Statement::Assign(_, fipe::mir::Rvalue::Pack { .. })
            ))
            .count()
            >= 2,
        "ordinary producer must execute real Pack assignments"
    );
    let Outcome::Done(Ok(runtime_value @ Value::Tuple(_))) =
        Machine::new(None).run(&program)
    else {
        panic!("actual runtime packed producer")
    };
    let Value::Tuple(runtime_parts) = &runtime_value else {
        unreachable!()
    };
    let [Value::Packed(checked_a), Value::Packed(checked_b)] =
        runtime_parts.as_slice()
    else {
        panic!("runtime packs")
    };
    let frame = Frame::Done {
        result: Ok(runtime_value.clone()),
        lines: vec![],
        charged: 0,
    };
    let Frame::Done {
        result: Ok(Value::Tuple(decoded)),
        ..
    } = Frame::decode(&frame.encode().unwrap()).unwrap()
    else {
        panic!("runtime-to-descriptive codec")
    };
    let [Value::Packed(descriptive_a), Value::Packed(descriptive_b)] =
        decoded.as_slice()
    else {
        panic!("decoded runtime packs")
    };
    assert_ne!(checked_a.target_type(), checked_b.target_type());
    assert_eq!(descriptive_a.target_type(), checked_a.target_type());
    assert_eq!(descriptive_b.target_type(), checked_b.target_type());
    assert_eq!(checked_a, descriptive_a);
    assert_eq!(descriptive_a, checked_b);
    assert_eq!(
        checked_a, checked_b,
        "actual checked/descriptive equality must be transitive"
    );
}

#[test]
fn field_dispatch_receiver_matches_the_vm() {
    for (name, source, expected) in [
        ("field_generic", "type Record { n: Integer }\nget = r => r.n\na = get (Record 4)\na + 1", "5"),
        ("field_anonymous", "type R { name: String, size: Integer }\nxs = [R \"a\" 2, R \"b\" 1]\n(xs | map .name, xs | sort -by .size | map .name)", "([\"a\", \"b\"], [\"b\", \"a\"])") ,
        ("field_function", "type F { op: Integer -> Integer }\nf = F (x => x + 1)\nget = r => r.op\n(get f) 7", "8"),
        ("field_partial", "type F { op: Integer -> Integer -> Integer }\ntype M { n: Integer }\nimpl M { op = a b self => self.n + a * 10 + b }\ncall = x => x.op 3 4\npartial = x => x.op 3\nf = F (a b => a * 10 + b)\n(call f, call (M 2), (partial f) 4, (partial (M 2)) 4)", "(34, 36, 34, 36)"),
        ("field_mixed", "type F { op: Integer -> Integer }\ntype M { n: Integer }\nimpl M { op = delta self => self.n + delta }\ncall = x => x.op 7\n(call (F (x => x + 1)), call (M 2))", "(8, 9)"),
    ] {
        let (observed, _, code) = equal(name, source);
        assert_eq!(code, 0);
        let (_, program) = unit(source);
        assert_eq!(fipe::value::structural::show(observed.result.as_ref().unwrap(), &program.tables.borrow()), expected, "{source}");
    }
}

#[test]
fn receiver_adapters_match_raw_arity_and_prefix_application() {
    for (name, source, expected) in [
        ("receiver_positions", "type M { n: Integer }\nimpl M { first = self a b => self.n + a * 10 + b }\nimpl M { middle = a self b => self.n + a * 10 + b }\nimpl M { last = a b self => self.n + a * 10 + b }\nget = m => m.last\np = M.first (M 2)\nq = M.middle 3\nr = M.last 3\ns = get (M 2)\n(p 3 4, q (M 2) 4, r 4 (M 2), s 3 4)", "(36, 36, 36, 36)"),
        ("receiver_returned_function", "type M { n: Integer }\nimpl M { op = self => (x => self.n + x) }\nget = m => m.op\nf = get (M 7)\nf 3", "10"),
        ("receiver_derived_and_declared", "f = (Just 1).eq\nget = x => x.len\n(f (Just 1), get [1, 2])", "(True, 2)"),
    ] {
        let (observed, _, code) = equal(name, source);
        assert_eq!(code, 0);
        let (_, program) = unit(source);
        assert_eq!(fipe::value::structural::show(observed.result.as_ref().unwrap(), &program.tables.borrow()), expected, "{source}");
    }
}

fn assert_bodyless_wrapper_vm(
    vm: &mut Machine,
    program: &Program,
    declaration: usize,
    yes: &Value,
) -> Value {
    let Outcome::Request(Request::Call {
        declaration: actual,
        arguments,
        ..
    }) = vm.run(program)
    else {
        panic!(
            "VM must invoke wrapper-owned Eq before unequal payload fallback"
        )
    };
    assert_eq!(actual, declaration);
    assert_eq!(arguments.len(), 2);
    assert!(arguments
        .iter()
        .all(|value| matches!(value, Value::Packed(_))));
    let Outcome::Done(Ok(vm_value)) =
        vm.reply(program, Reply::Value(Ok(yes.clone())))
    else {
        panic!("VM accepts real Eq reply")
    };
    assert_eq!(&vm_value, yes);
    vm_value
}

#[test]
fn bodyless_ordinary_wrapper_eq_keeps_its_host_callback() {
    let header =
        "pub type Key: Eq\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool }";
    let source = "use probe\ntype P: probe.Key { n: Integer }\nx: probe.Key = P 1\ny: probe.Key = P 2\n(x, 0) == (y, 0)";
    let (unit, program) =
        Unit::new(source, vec![("probe".into(), header.into())], vec![])
            .expect(
                "ordinary bodyless wrapper is admitted by native Unit::compile",
            );
    let (declaration, yes) = {
        let tables = program.tables.borrow();
        assert!(!tables.has_resources);
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        let declaration = tables.dispatch.iter().filter(|member| member.label == "eq").flat_map(|member| &member.rows).find_map(|row| {
            if matches!(row.receiver, fipe::ir::Type::Meaning { index, .. } if index == key) {
                if let fipe::ir::Implementation::Declared(declaration) = row.target { return Some(declaration); }
            }
            None
        }).expect("actual Key-owned bodyless Host eq row");
        (declaration, fipe::value::Prelude::new(&tables).bool(true))
    };
    let mut vm = Machine::new(None);
    let vm_value =
        assert_bodyless_wrapper_vm(&mut vm, &program, declaration, &yes);
    let executable =
        common::executable("bodyless-ordinary-wrapper-eq", &unit, &program);
    let mut child = PipeChild::new(
        Command::new("sh")
            .args(["-c", "exec \"$0\" 3>&1 4<&0 1>&2 0</dev/null"])
            .arg(&executable)
            .env_remove("FIPE_BUDGET")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut calls = 0;
    let native_value = loop {
        let mut length = [0; 8];
        frames.read_exact(&mut length).unwrap();
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call {
                request:
                    Request::Call {
                        declaration: actual,
                        arguments,
                        ..
                    },
                ..
            } => {
                assert_eq!(actual, declaration);
                assert_eq!(arguments.len(), 2);
                assert!(arguments
                    .iter()
                    .all(|value| matches!(value, Value::Packed(_))));
                calls += 1;
                assert!(calls <= 1, "one actual comparison");
                if let Err(error) = pipe_reply(&mut replies, &Ok(yes.clone())) {
                    drop(replies);
                    drop(frames);
                    drop(child);
                    panic!("wrapper reply failed before completion: {error}");
                }
            }
            Frame::Done { result, .. } => break result.unwrap(),
            other @ Frame::Call {
                request: Request::File { .. },
                ..
            } => panic!("unexpected native protocol: {other:?}"),
        }
    };
    drop(replies);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        calls, 1,
        "native must preserve the admitted wrapper-owned Host effect"
    );
    assert_eq!(
        native_value, vm_value,
        "unequal payloads cannot override Host Eq True"
    );
}

type HostRun = (
    Result<Value, Fault>,
    Vec<(Request, Reply, Vec<(usize, Value)>)>,
    Vec<(usize, Value)>,
    u64,
    i32,
);

fn host_run_vm(
    program: &Program,
    budget: Option<u64>,
    mut reply: impl FnMut(&Request) -> Value,
) -> HostRun {
    host_run_vm_observed(program, budget, |_, request| reply(request))
}

fn host_run_vm_observed(
    program: &Program,
    budget: Option<u64>,
    mut reply: impl FnMut(&Machine, &Request) -> Value,
) -> HostRun {
    let mut machine = Machine::new(budget);
    let mut calls = Vec::new();
    let mut outcome = machine.run(program);
    let result = loop {
        match outcome {
            Outcome::Request(request) => {
                let answer = Reply::Value(Ok(reply(&machine, &request)));
                calls.push((request, answer.clone(), machine.lines().to_vec()));
                outcome = machine.reply(program, answer);
            }
            Outcome::Done(result) => break result,
        }
    };
    let code = i32::from(result.is_err());
    (
        result,
        calls,
        machine.lines().to_vec(),
        machine.charged(),
        code,
    )
}

fn host_run_native(
    executable: &Path,
    budget: Option<u64>,
    mut reply: impl FnMut(&Request) -> Value,
) -> HostRun {
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec \"$0\" 3>&1 4<&0 1>&2 0</dev/null"])
        .arg(executable)
        .env_remove("FIPE_BUDGET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(limit) = budget {
        command.env("FIPE_BUDGET", limit.to_string());
    }
    let mut child = PipeChild::new(command.spawn().unwrap());
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut calls = Vec::new();
    let (result, lines, charged) = loop {
        let mut length = [0; 8];
        frames.read_exact(&mut length).unwrap();
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call { request, lines } => {
                let value = reply(&request);
                let encoded = match codec::encode_reply(&Ok(value.clone())) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        drop(replies);
                        drop(frames);
                        drop(child);
                        panic!("supplied reply refused before header: {error}");
                    }
                };
                calls.push((request, Reply::Value(Ok(value)), lines));
                if let Err(error) = pipe_payload(&mut replies, &encoded) {
                    drop(replies);
                    drop(frames);
                    drop(child);
                    panic!("supplied reply failed before completion: {error}");
                }
            }
            Frame::Done {
                result,
                lines,
                charged,
            } => break (result, lines, charged),
        }
    };
    drop(replies);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let code = output.status.code().expect("native exits normally");
    assert_eq!(
        code,
        i32::from(result.is_err()),
        "framed Host success/Fault exit status"
    );
    (result, calls, lines, charged, code)
}

fn assert_supplied_host_rows(
    program: &Program,
    source: &str,
    expression: &str,
    expected: &[&str],
) {
    let tables = program.tables.borrow();
    assert!(!tables.has_resources);
    let prelude = fipe::value::Prelude::new(&tables);
    let key = tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == "Key")
        .unwrap();
    for label in expected {
        if *label == "default" && expression == "probe.Key.default" {
            let actual = fipe::ir::Type::Meaning {
                index: key,
                arguments: vec![fipe::ir::Type::Meaning {
                    index: prelude.integer.expect("actual Integer meaning"),
                    arguments: Vec::new(),
                }],
            };
            let start = source.find(expression).expect("direct default call");
            let span = fipe::token::Span {
                start,
                end: start + expression.len(),
            };
            let declarations: Vec<_> = tables
                .nodes
                .iter()
                .filter_map(|node| {
                    let fipe::ir::Kind::Declared(index) = node.kind else {
                        return None;
                    };
                    let declaration = &tables.declarations[index];
                    (declaration.label == "default"
                        && tables.namespaces[declaration.namespace] == "probe"
                        && node.span == span
                        && node.ty == actual)
                        .then_some(index)
                })
                .collect();
            assert!(
                !declarations.is_empty(),
                "actual direct Host default IR has Key<Integer> result"
            );
            assert!(program.bodies.iter().any(|body| body.blocks.iter().any(|block| {
                matches!(&block.terminator, fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Host(index), dest, .. }
                    if declarations.contains(index) && body.locals[*dest].ty == actual)
            })), "actual direct Host MIR destination retains full Key<Integer> result");
        } else {
            assert!(tables.dispatch.iter().filter(|member| member.label == *label).flat_map(|member| &member.rows).any(|row| matches!(row.receiver, fipe::ir::Type::Meaning { index, .. } if index == key) && matches!(row.target, fipe::ir::Implementation::Declared(_))), "actual Key-owned {label} row");
        }
    }
    if expression == "Holder.default" {
        assert!(tables.nodes.iter().any(|node| matches!(node.kind, fipe::ir::Kind::Declared(index) if tables.declarations[index].label == "default")), "enclosing default retains its supplied child declaration");
        assert!(tables.nodes.iter().any(|node| matches!(node.kind, fipe::ir::Kind::Call { callee, .. } if matches!(tables.nodes[callee].kind, fipe::ir::Kind::Constructor { meaning, .. } if tables.meanings[meaning].label == "Holder"))), "public enclosing default constructs Holder");
    }
}

fn supplied_host_reply(
    tables: &fipe::ir::Tables,
    request: &Request,
    saved: &mut (Option<Value>, Option<Value>),
    wrong: bool,
) -> Value {
    let prelude = fipe::value::Prelude::new(tables);
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("only supplied calls")
    };
    let label = tables.declarations[*declaration].label.as_str();
    if label == "keep" {
        assert!(environment.is_empty());
        saved.0 = Some(arguments[0].clone());
        return Value::Unit;
    }
    if label == "keep_wrong" {
        assert!(environment.is_empty());
        saved.1 = Some(arguments[0].clone());
        return Value::Unit;
    }
    if label == "after" {
        assert!(environment.is_empty(), "with environment closes");
        return Value::Unit;
    }
    assert_eq!(
        environment.len(),
        1,
        "supplied row keeps exactly the open with key"
    );
    assert_eq!(tables.keys[environment[0].0], "lang");
    assert_eq!(environment[0].1, Value::text("audit"));
    if wrong {
        return if matches!(label, "default" | "zero" | "add") {
            saved
                .1
                .as_ref()
                .expect("actual script-issued wrong generic record")
                .clone()
        } else {
            Value::Unit
        };
    }
    match label {
        "eq" => prelude.bool(true),
        "cmp" => prelude.ordering(std::cmp::Ordering::Less),
        "show" => Value::text("wrapped"),
        "len" => Value::Int(17),
        "add" => arguments[1].clone(),
        "zero" | "default" => saved
            .0
            .as_ref()
            .expect("script-issued generic record")
            .clone(),
        _ => panic!("unexpected supplied row {label}"),
    }
}

/// Drive the same supplied Host rows through VM and the framed native channel.
fn supplied_host_family_case(
    name: &str,
    expression: &str,
    expected: &[&str],
    wrong: bool,
) {
    let header = "pub type Key<T>: Eq & Ord & Show & Len & Add & Zero & Default { pub n: T }\nimpl Key<T>: Eq { eq: Self -> (self: Self) -> Bool }\nimpl Key<T>: Ord { cmp: Self -> (self: Self) -> Ordering }\nimpl Key<T>: Show { show: (self: Self) -> String }\nimpl Key<T>: Len { len: (self: Self) -> Natural }\nimpl Key<T>: Add { add: Self -> (self: Self) -> Self }\nimpl Key<T>: Zero { zero: Self }\nimpl Key<T>: Default { default: Self }\npub keep: Key<Integer> -> ()\npub keep_wrong: Key<String> -> ()\npub after: ()";
    let default_setup = if expression == "Holder.default" {
        "type Holder: Default { item: probe.Key<Integer> }\n"
    } else {
        ""
    };
    let annotation = if expression == "probe.Key.default" {
        ": probe.Key<Integer>"
    } else {
        ""
    };
    let source = format!("use probe\n{default_setup}x: probe.Key<Integer> = probe.Key 1\ny: probe.Key<Integer> = probe.Key 2\nprobe.keep x\nprobe.keep_wrong (probe.Key \"wrong\")\nempty: Vec<probe.Key<Integer>> = []\nanswer{annotation} = with lang \"audit\" {{ {expression} }}\nprobe.after\nanswer");
    let (unit, program) =
        Unit::new(&source, vec![("probe".into(), header.into())], vec![])
            .expect("supplied Host family is admitted");
    assert_supplied_host_rows(&program, &source, expression, expected);
    let tables = program.tables.borrow();
    let reply = |request: &Request,
                 saved: &mut (Option<Value>, Option<Value>)| {
        supplied_host_reply(&tables, request, saved, wrong)
    };
    let on_vm = |budget| {
        let mut saved = (None, None);
        host_run_vm(&program, budget, |request| reply(request, &mut saved))
    };
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| {
            let Request::Call { declaration, .. } = request else {
                panic!("call")
            };
            tables.declarations[*declaration].label.as_str()
        })
        .collect();
    let wanted: Vec<_> = ["keep", "keep_wrong"]
        .into_iter()
        .chain(expected.iter().copied())
        .chain((!wrong).then_some("after"))
        .collect();
    assert_eq!(labels, wanted, "VM actual connected producer");
    if wrong {
        let fault = whole
            .0
            .as_ref()
            .expect_err("wrong reply refuses before after");
        assert!(
            matches!(&fault.reason, Reason::Host(message) if message.starts_with("the reply is not "))
        );
        let call_source = expression;
        let start = source.find(call_source).expect("the tested call");
        assert_eq!(
            fault.span,
            fipe::token::Span {
                start,
                end: start + call_source.len()
            },
            "Host refusal uses the actual current call"
        );
        assert_eq!(tables.namespaces[fault.namespace], "main");
    } else {
        assert!(whole.0.is_ok());
    }
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget: Option<u64>| {
        let mut saved = (None, None);
        host_run_native(&executable, budget, |request| {
            reply(request, &mut saved)
        })
    };
    assert_eq!(
        on_native(None),
        whole,
        "whole result/Fault identity, calls/environment/lines, units and exit"
    );
    for limit in [whole.3 / 3, whole.3 / 2, whole.3.saturating_sub(1)] {
        let vm = on_vm(Some(limit));
        assert!(
            matches!(&vm.0, Err(fault) if fault.reason == Reason::Budget),
            "cut budget is nonvacuous"
        );
        assert_eq!(
            on_native(Some(limit)),
            vm,
            "same budget {limit} observes exactly the same boundary"
        );
    }
}

#[test]
fn supplied_host_eq_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-eq",
        "(x, 0) == (y, 0)",
        &["eq"],
        false,
    );
}

#[test]
fn supplied_host_cmp_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-cmp",
        "[(x, 0), (y, 0)] | sort",
        &["cmp"],
        false,
    );
}

#[test]
fn supplied_host_show_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-show",
        "show (x, 0)",
        &["show"],
        false,
    );
}

#[test]
fn supplied_host_len_matches_the_vm() {
    supplied_host_family_case("supplied-host-len", "len x", &["len"], false);
}

#[test]
fn supplied_host_add_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-add",
        "sum [x, y]",
        &["add"],
        false,
    );
}

#[test]
fn supplied_host_zero_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-zero",
        "sum empty",
        &["zero"],
        false,
    );
}

#[test]
fn supplied_host_default_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-default",
        "probe.Key.default",
        &["default"],
        false,
    );
}

#[test]
fn supplied_host_holder_default_matches_the_vm() {
    supplied_host_family_case(
        "supplied-host-holder_default",
        "Holder.default",
        &["default"],
        false,
    );
}

#[test]
fn supplied_host_scalar_reply_fails_before_later_effects() {
    supplied_host_family_case(
        "supplied-host-bad-scalar",
        "(x, 0) == (y, 0)",
        &["eq"],
        true,
    );
}

#[test]
fn supplied_host_generic_reply_fails_before_later_effects() {
    supplied_host_family_case(
        "supplied-host-bad-generic",
        "probe.Key.default",
        &["default"],
        true,
    );
}

fn admission_answer(
    program: &Program,
    declaration: usize,
    arguments: &[Value],
    descriptive: bool,
) -> Value {
    match program.tables.borrow().declarations[declaration]
        .label
        .as_str()
    {
        "number" => {
            assert!(arguments.is_empty());
            Value::Int(7)
        }
        "echo" => {
            let [Value::Packed(packed)] = arguments else {
                panic!("actual runtime Pack")
            };
            let fipe::ir::Type::Meaning { index, .. } = packed.target_type()
            else {
                panic!("nominal Key")
            };
            assert_eq!(program.tables.borrow().meanings[*index].label, "Key");
            if descriptive {
                Value::Packed(std::rc::Rc::new(
                    fipe::value::Packed::from_data(
                        packed.source_type().clone(),
                        packed.target_type().clone(),
                        packed.value(),
                    )
                    .unwrap(),
                ))
            } else {
                arguments[0].clone()
            }
        }
        label => panic!("unexpected admission callback {label}"),
    }
}

fn admission_vm(
    program: &Program,
    budget: Option<u64>,
    descriptive: bool,
) -> (Result<Value, Fault>, u64, Vec<String>, Option<u64>) {
    let mut machine = Machine::new(budget);
    let mut outcome = machine.run(program);
    let mut calls = Vec::new();
    let mut reply_work = None;
    loop {
        match outcome {
            Outcome::Done(result) => {
                if let Ok(value) = &result {
                    assert_eq!(
                        machine.lines(),
                        &[(program.bodies[program.main].node, value.clone())]
                    );
                } else {
                    assert!(machine.lines().is_empty());
                }
                return (result, machine.charged(), calls, reply_work);
            }
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                assert!(machine.lines().is_empty());
                let label = program.tables.borrow().declarations[declaration]
                    .label
                    .clone();
                let answer = admission_answer(
                    program,
                    declaration,
                    &arguments,
                    descriptive,
                );
                calls.push(label.clone());
                let before = machine.charged();
                outcome = machine.reply(program, Reply::Value(Ok(answer)));
                if label == "echo" {
                    reply_work = Some(machine.charged() - before);
                }
            }
            other @ Outcome::Request(Request::File { .. }) => {
                panic!("unexpected {other:?}")
            }
        }
    }
}

fn admission_native(
    executable: &Path,
    program: &Program,
    budget: Option<u64>,
) -> (Result<Value, Fault>, u64, Vec<String>) {
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
    let mut child = PipeChild::new(command.spawn().unwrap());
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut calls = Vec::new();
    let (result, charged) = loop {
        let mut length = [0; 8];
        frames.read_exact(&mut length).unwrap();
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call {
                request:
                    Request::Call {
                        declaration,
                        arguments,
                        environment,
                    },
                lines,
            } => {
                assert!(
                    environment.is_empty(),
                    "the admission fixture opens no environment"
                );
                assert!(
                    lines.is_empty(),
                    "no main value completes before these callbacks"
                );
                calls.push(
                    program.tables.borrow().declarations[declaration]
                        .label
                        .clone(),
                );
                let answer =
                    admission_answer(program, declaration, &arguments, false);
                if let Err(error) = pipe_reply(&mut replies, &Ok(answer)) {
                    drop(replies);
                    drop(frames);
                    drop(child);
                    panic!("admission reply failed before completion: {error}");
                }
            }
            Frame::Done {
                result,
                charged,
                lines,
            } => {
                if let Ok(value) = &result {
                    assert_eq!(
                        lines,
                        [(program.bodies[program.main].node, value.clone())]
                    );
                } else {
                    assert!(
                        lines.is_empty(),
                        "failed admission completes no line"
                    );
                }
                break (result, charged);
            }
            other @ Frame::Call {
                request: Request::File { .. },
                ..
            } => panic!("unexpected native admission protocol {other:?}"),
        }
    };
    drop(replies);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(i32::from(result.is_err())));
    (result, charged, calls)
}

#[test]
fn opaque_packed_reply_admission_work_is_independent_of_private_proof() {
    let header = "pub type Key: Eq\npub number: Integer\npub echo: Key -> Key";
    let source = "use probe\ntype P: probe.Key { n: Integer }\nx: probe.Key = P probe.number\nprobe.echo x";
    let (unit, program) =
        Unit::new(source, vec![("probe".into(), header.into())], vec![])
            .unwrap();
    assert!(!program.tables.borrow().has_resources);
    assert!(program
        .bodies
        .iter()
        .flat_map(|body| &body.blocks)
        .flat_map(|block| &block.statements)
        .any(|statement| matches!(
            statement,
            fipe::mir::Statement::Assign(_, fipe::mir::Rvalue::Pack { .. })
        )));
    let checked = admission_vm(&program, None, false);
    let descriptive = admission_vm(&program, None, true);
    assert!(checked.0.is_ok());
    assert_eq!(checked, descriptive);
    assert_eq!(checked.3, Some(4));
    assert_eq!(checked.2, ["number", "echo"]);
    let executable =
        common::executable("opaque-packed-reply-work", &unit, &program);
    let native = admission_native(&executable, &program, None);
    assert_eq!(native, (checked.0.clone(), checked.1, checked.2.clone()));
    for budget in [0, checked.1 - 1, checked.1] {
        let vm = admission_vm(&program, Some(budget), false);
        let descriptive = admission_vm(&program, Some(budget), true);
        assert_eq!(
            vm, descriptive,
            "private-proof independence under budget {budget}"
        );
        let native = admission_native(&executable, &program, Some(budget));
        assert_eq!(
            native,
            (vm.0.clone(), vm.1, vm.2.clone()),
            "budget {budget}"
        );
        assert_eq!(vm.1, budget);
        if budget == checked.1 {
            assert!(vm.0.is_ok());
        } else {
            assert!(matches!(
                vm.0,
                Err(Fault {
                    reason: Reason::Budget,
                    ..
                })
            ));
        }
        if budget == 0 {
            assert!(
                vm.2.is_empty(),
                "no Host effect before an unaffordable main"
            );
        } else {
            assert_eq!(
                vm.2,
                ["number", "echo"],
                "short budget reaches echo then faults during reply admission"
            );
        }
    }
}

/// Each route must really transport an operation witness through dynamic dispatch.
fn operation_witness_fixture(name: &str, declared: bool) {
    let (header, source, member) = if declared {
        (
            "pub type W<T> { pub value: T }\nimpl W<a> { lookup: (self: Self) -> a }",
            "use sys\nget = w => w.lookup\nget (sys.W 1)",
            "lookup",
        )
    } else {
        (
            "pub lookup: () -> a",
            "use sys\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => sys.lookup () }\nget = w => w.op\nget (W 1)",
            "op",
        )
    };
    let (unit, program) =
        Unit::new(source, vec![("sys".into(), header.into())], vec![])
            .expect("public generic operation witness fixture is admitted");
    {
        let tables = program.tables.borrow();
        assert!(!tables.has_resources);
        let adapters: Vec<_> = program
            .receiver_adapters
            .iter()
            .filter(|adapter| {
                tables.dispatch[adapter.member].label == member
                    && matches!(
                        (declared, &adapter.target),
                        (true, fipe::mir::ReceiverTarget::Declared(_))
                            | (false, fipe::mir::ReceiverTarget::Definition(_))
                    )
            })
            .collect();
        assert!(!adapters.is_empty(), "actual operation adapter exists");
        if !declared {
            for adapter in adapters {
                let row = &tables.dispatch[adapter.member].rows[adapter.row];
                let body = &program.bodies[adapter.body];
                let context = program.receiver_contexts[adapter.body]
                    .as_ref()
                    .expect("receiver context");
                let end = context
                    .protocol_result
                    .as_ref()
                    .map_or(body.hidden.len(), |result| result.meaning);
                let operation = &body.hidden[1..end];
                assert!(
                    !operation.is_empty(),
                    "operation witness is nonzero before the result carrier"
                );
                assert_eq!(context.protocol_result.is_some(), declared);
                if let fipe::ir::Implementation::Definition {
                    index,
                    meanings,
                } = &row.target
                {
                    assert_eq!(tables.definitions[*index].label, "op");
                    assert!(!tables.definitions[*index].hidden.is_empty());
                    assert_eq!(
                        meanings.len(),
                        operation.len(),
                        "raw operation witnesses retain their order/count"
                    );
                } else {
                    assert!(matches!(
                        row.target,
                        fipe::ir::Implementation::Declared(_)
                    ));
                }
                assert!(program.bodies.iter().any(|caller| caller.blocks.iter().any(|block| {
                matches!(&block.terminator,
                    fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Dispatch { member, witness: fipe::mir::Witness::Value(_), .. }, .. }
                        if *member == adapter.member)
            })), "generic caller really executes Value-witness dispatch");
                eprintln!("OPERATION_WITNESS declared={declared} member={} row={} operation_count={} result_carrier={}", adapter.member, adapter.row, operation.len(), context.protocol_result.is_some());
            }
        }
    }
    if declared {
        observe_invoked_declared_operation(&program);
    }

    // Existing helper preserves complete observations, whole units and cut budgets.
    let (observed, _, code) = equal_prepared(name, source, &unit, &program);
    assert_eq!(observed.result, Ok(Value::Int(7)));
    assert_eq!(code, 0);
}

/// Inspect the actual pending Host invocation, using the joined Machine program.
fn observe_invoked_declared_operation(program: &Program) {
    let mut machine = Machine::new(None);
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(program)
    else {
        panic!("actual declared member reaches its Host callback")
    };
    assert_eq!(arguments.len(), 1, "self-only protocol");
    {
        let joined = machine.program().expect("actual joined bodies");
        let tables = joined.tables.borrow();
        assert_eq!(
            tables.namespaces[tables.declarations[declaration].namespace],
            "sys"
        );
        assert_eq!(tables.declarations[declaration].label, "lookup");
        let mut invoked = 0;
        for frame in machine.frames() {
            let fipe::vm::Frame::Body(frame) = frame else {
                continue;
            };
            let Some(adapter) = joined
                .receiver_adapters
                .iter()
                .find(|adapter| adapter.body == frame.body)
            else {
                continue;
            };
            if adapter.target
                != fipe::mir::ReceiverTarget::Declared(declaration)
            {
                continue;
            }
            let row = &tables.dispatch[adapter.member].rows[adapter.row];
            assert_eq!(tables.dispatch[adapter.member].label, "lookup");
            assert_eq!(
                row.target,
                fipe::ir::Implementation::Declared(declaration)
            );
            assert_eq!(row.receiver_spot, Some(0));
            assert_eq!(adapter.receiver_spot, 0);
            let body = &joined.bodies[frame.body];
            let context = joined.receiver_contexts[frame.body]
                .as_ref()
                .expect("invoked context");
            let result = context
                .protocol_result
                .as_ref()
                .expect("actual Declared result carrier");
            let operation = &body.hidden[1..result.meaning];
            assert!(
                !operation.is_empty(),
                "ACTUALLY INVOKED adapter has nonzero operation witnesses"
            );
            assert_eq!(frame.meanings.len(), body.hidden.len());
            let integer = fipe::ir::Type::Meaning {
                index: fipe::value::Prelude::new(&tables).integer.unwrap(),
                arguments: Vec::new(),
            };
            assert!(
                frame.meanings[1..result.meaning]
                    .iter()
                    .all(|ty| *ty == integer),
                "invoked operation receives concrete Integer witnesses"
            );
            assert!(joined.bodies.iter().any(|caller| caller.blocks.iter().any(|block| matches!(&block.terminator,
                fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Dispatch { member, witness: fipe::mir::Witness::Value(_), .. }, .. }
                    if *member == adapter.member))), "same invoked member has actual Value dispatch producer");
            eprintln!("INVOKED_OPERATION declared=true member={} row={} body={} self_spot={} arity={} operation_count={}", adapter.member, adapter.row, frame.body, adapter.receiver_spot, body.arity, operation.len());
            invoked += 1;
        }
        assert_eq!(
            invoked, 1,
            "one actual pending adapter, not arbitrary generated variants"
        );
    }
    assert!(matches!(
        machine.reply(program, Reply::Value(Ok(Value::Int(7)))),
        Outcome::Done(Ok(Value::Int(7)))
    ));
}

#[test]
fn receiver_definition_operation_witness_keeps_vm_work_schedule() {
    operation_witness_fixture("definition-operation-witness", false);
}

#[test]
fn receiver_declared_operation_witness_keeps_vm_work_schedule() {
    operation_witness_fixture("declared-operation-witness", true);
}

fn graph_echo_vm(
    program: &Program,
    reply: &Value,
    budget: Option<u64>,
) -> (Result<Value, Fault>, u64, Vec<(Request, Reply)>) {
    let mut machine = Machine::new(budget);
    let mut outcome = machine.run(program);
    let mut trace = Vec::new();
    loop {
        match outcome {
            Outcome::Done(result) => {
                if let Ok(value) = &result {
                    assert_eq!(
                        machine.lines(),
                        &[(program.bodies[program.main].node, value.clone())]
                    );
                } else {
                    assert!(machine.lines().is_empty());
                }
                return (result, machine.charged(), trace);
            }
            Outcome::Request(request @ Request::Call { .. }) => {
                assert!(machine.lines().is_empty());
                let answer = graph_echo_answer(program, &request, reply);
                trace.push((request, Reply::Value(Ok(answer.clone()))));
                outcome = machine.reply(program, Reply::Value(Ok(answer)));
            }
            other @ Outcome::Request(Request::File { .. }) => {
                panic!("unexpected graph protocol {other:?}")
            }
        }
    }
}

fn graph_echo_answer(
    program: &Program,
    request: &Request,
    reply: &Value,
) -> Value {
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("call")
    };
    assert!(environment.is_empty());
    match program.tables.borrow().declarations[*declaration]
        .label
        .as_str()
    {
        "number" => {
            assert!(arguments.is_empty());
            Value::Int(7)
        }
        "echo" => {
            assert!(matches!(arguments.as_slice(), [Value::Packed(_)]));
            reply.clone()
        }
        label => panic!("unexpected graph callback {label}"),
    }
}

fn graph_echo_native(
    executable: &Path,
    program: &Program,
    reply: &Value,
    budget: Option<u64>,
) -> (Result<Value, Fault>, u64, Vec<(Request, Reply)>) {
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
    let mut child = PipeChild::new(command.spawn().unwrap());
    let mut frames = child.stdout.take().unwrap();
    let mut replies = child.stdin.take().unwrap();
    let mut trace = Vec::new();
    let (result, charged) = loop {
        let mut length = [0; 8];
        frames.read_exact(&mut length).unwrap();
        let mut payload =
            vec![0; usize::try_from(u64::from_le_bytes(length)).unwrap()];
        frames.read_exact(&mut payload).unwrap();
        match Frame::decode(&payload).unwrap() {
            Frame::Call { request, lines } => {
                assert!(lines.is_empty());
                let answer = graph_echo_answer(program, &request, reply);
                if let Err(error) =
                    pipe_reply(&mut replies, &Ok(answer.clone()))
                {
                    drop(replies);
                    drop(frames);
                    drop(child);
                    panic!("graph reply failed before completion: {error}");
                }
                trace.push((request, Reply::Value(Ok(answer))));
            }
            Frame::Done {
                result,
                lines,
                charged,
            } => {
                if let Ok(value) = &result {
                    assert_eq!(
                        lines,
                        [(program.bodies[program.main].node, value.clone())]
                    );
                    let (Value::Tuple(a), Value::Tuple(b)) =
                        (value, &lines[0].1)
                    else {
                        panic!("returned graph")
                    };
                    assert!(
                        std::rc::Rc::ptr_eq(a, b),
                        "result/line root alias survives one frame"
                    );
                } else {
                    assert!(lines.is_empty());
                }
                break (result, charged);
            }
        }
    };
    drop(replies);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(i32::from(result.is_err())));
    (result, charged, trace)
}

#[test]
fn opaque_shared_data_echo_keeps_native_aliases_and_all_budget_observations() {
    let header = "pub type Key: Eq\npub number: Integer\npub echo: Key -> Key";
    let source = "use probe\ntype P: probe.Key { n: Integer }\nx: probe.Key = P probe.number\nprobe.echo x";
    let (unit, program) =
        Unit::new(source, vec![("probe".into(), header.into())], vec![])
            .unwrap();
    assert!(!program.tables.borrow().has_resources);
    let echo = program
        .tables
        .borrow()
        .declarations
        .iter()
        .find(|declaration| declaration.label == "echo")
        .unwrap()
        .ty
        .clone();
    let fipe::ir::Type::Function { parameter, result } = echo else {
        panic!("echo schema")
    };
    assert_eq!(parameter, result);
    let child = std::rc::Rc::new(vec![Value::Int(7)]);
    let shared = Value::Tuple(std::rc::Rc::new(vec![
        Value::Tuple(child.clone()),
        Value::Tuple(child),
    ]));
    let expanded = Value::Tuple(std::rc::Rc::new(vec![
        Value::Tuple(std::rc::Rc::new(vec![Value::Int(7)])),
        Value::Tuple(std::rc::Rc::new(vec![Value::Int(7)])),
    ]));
    assert_eq!(shared, expanded);
    let shared_bytes = codec::encode_reply(&Ok(shared.clone())).unwrap();
    let expanded_bytes = codec::encode_reply(&Ok(expanded.clone())).unwrap();
    assert_ne!(
        shared_bytes, expanded_bytes,
        "aliases are now explicit wire data"
    );
    let decoded = codec::decode_reply(&shared_bytes).unwrap().unwrap();
    let Value::Tuple(parts) = &decoded else {
        panic!("decoded tuple")
    };
    let (Value::Tuple(a), Value::Tuple(b)) = (&parts[0], &parts[1]) else {
        panic!("decoded children")
    };
    assert!(std::rc::Rc::ptr_eq(a, b));
    assert_eq!(decoded, shared);
    let executable =
        common::executable("opaque-shared-data-echo", &unit, &program);
    let mut whole = Vec::new();
    for reply in [&shared, &expanded] {
        let vm = graph_echo_vm(&program, reply, None);
        let native = graph_echo_native(&executable, &program, reply, None);
        assert!(vm.0.is_ok());
        assert_eq!(native, vm);
        assert_eq!(vm.2.len(), 2);
        whole.push(vm.1);
        for budget in [0, vm.1 / 2, vm.1 - 1, vm.1] {
            assert_eq!(
                graph_echo_native(&executable, &program, reply, Some(budget)),
                graph_echo_vm(&program, reply, Some(budget)),
                "budget {budget}"
            );
        }
    }
    assert_eq!(
        whole[1] - whole[0],
        2,
        "expanded control still has distinct allocation work"
    );
    assert_eq!(
        graph_echo_vm(&program, &decoded, None),
        graph_echo_vm(&program, &shared, None)
    );
}

fn shared_size_source(depth: usize) -> String {
    use core::fmt::Write as _;
    assert!(matches!(depth, 0 | 2 | 12 | 14));
    let mut source = String::from("use sys\ntype Tree: Eq & Show { Leaf(value: Integer), Branch(children: Vec<Tree>) }\nimpl Tree: Show { show = self => \"tree\" }\nrender = tree => show tree\nt0 = Leaf (sys.lookup ())\n");
    for level in 1..=depth {
        writeln!(source, "t{level} = Branch [t{}, t{}]", level - 1, level - 1)
            .unwrap();
    }
    write!(source, "render t{depth}").unwrap();
    source
}

fn assert_shared_size_dag(
    value: &Value,
    tables: &fipe::ir::Tables,
    depth: usize,
) -> (usize, usize) {
    let mut value = value;
    let mut records = std::collections::HashSet::new();
    let mut sequences = std::collections::HashSet::new();
    for _ in 0..depth {
        let Value::Record(record) = value else {
            panic!("real Tree Branch")
        };
        assert_eq!(tables.meanings[record.meaning].label, "Tree");
        assert_eq!(record.constructor, 1);
        assert!(
            records.insert(std::rc::Rc::as_ptr(record)),
            "distinct physical layer"
        );
        let [Value::Seq(children)] = record.fields.as_slice() else {
            panic!("Branch children")
        };
        assert!(sequences.insert(std::rc::Rc::as_ptr(children)));
        let [Value::Record(left), Value::Record(right)] = children.as_slice()
        else {
            panic!("two Tree children")
        };
        assert!(
            std::rc::Rc::ptr_eq(left, right),
            "actual script-created shared DAG at each layer"
        );
        value = &children[0];
    }
    let Value::Record(leaf) = value else {
        panic!("Leaf")
    };
    assert_eq!(leaf.constructor, 0);
    assert_eq!(leaf.fields, vec![Value::Int(7)]);
    assert!(records.insert(std::rc::Rc::as_ptr(leaf)));
    assert_eq!(records.len(), depth + 1);
    assert_eq!(sequences.len(), depth);
    (records.len(), sequences.len())
}

/// Observe the actual native Show argument without unfolding its shared children.
fn shared_size_setup(program: &Program, depth: usize) -> u64 {
    let mut machine = Machine::new(None);
    machine.begin(program);
    let mut seed_calls = 0;
    for _ in 0..10_000 {
        match machine.next_stop() {
            fipe::vm::Stop::Request(request) => {
                let Request::Call { declaration, .. } = &request else {
                    panic!("scalar seed call")
                };
                let joined = machine.program().expect("joined source");
                let tables = joined.tables.borrow();
                assert_eq!(
                    tables.namespaces
                        [tables.declarations[*declaration].namespace],
                    "sys"
                );
                assert_eq!(tables.declarations[*declaration].label, "lookup");
                let mut host = replies::Host::default();
                let value = host.answer(&tables, &request).unwrap();
                assert_eq!(value, Value::Int(7));
                seed_calls += 1;
                drop(tables);
                machine.answer(Reply::Value(Ok(value)));
            }
            fipe::vm::Stop::Before(_) => {
                let Some(fipe::vm::Frame::Body(frame)) =
                    machine.frames().last()
                else {
                    continue;
                };
                let joined = machine.program().expect("joined body identity");
                let body = &joined.bodies[frame.body];
                let block = &body.blocks[frame.block];
                if frame.statement != block.statements.len() {
                    continue;
                }
                let fipe::mir::Terminator::Call {
                    callee: fipe::mir::Callee::Native(op),
                    args,
                    ..
                } = &block.terminator
                else {
                    continue;
                };
                let tables = joined.tables.borrow();
                let declaration = &tables.declarations[*op];
                if declaration.label != "show"
                    || tables.namespaces[declaration.namespace] != "prelude"
                {
                    continue;
                }
                let [fipe::mir::Operand::Local(local)] = args.as_slice() else {
                    panic!("actual native Show has a local argument")
                };
                let (record_count, sequence_count) = assert_shared_size_dag(
                    &frame.locals[*local],
                    &tables,
                    depth,
                );
                assert_eq!(seed_calls, 1);
                let charged = machine.charged();
                let predicted = 4_u64 * (1_u64 << depth) - 3;
                eprintln!("SIZE_REACHABLE depth={depth} physical_records={record_count} physical_sequences={sequence_count} predicted_descendant_occurrences={predicted} setup_charged={charged}");
                drop(tables);
                assert!(
                    machine.abandon(),
                    "active paused observer is abandoned"
                );
                return charged;
            }
            fipe::vm::Stop::After(_, _) => {}
            stop @ fipe::vm::Stop::Done(_) => {
                panic!("before actual Show: {stop:?}")
            }
        }
    }
    panic!("bounded observer never reached native Show")
}

fn shared_size_case(test_name: &str, depth: usize) {
    let source = shared_size_source(depth);
    let (unit, program) = Unit::new(
        &source,
        vec![("sys".into(), "pub lookup: () -> Integer".into())],
        vec![],
    )
    .expect("public recursive Tree with its written Show is admitted");
    if std::env::var("FIPE_SIZE_PROBE_CHILD").as_deref() != Ok(test_name) {
        // Build outside the execution deadline; the worker does not compile native code.
        let executable = common::executable(
            &format!("size-preflight-{depth}"),
            &unit,
            &program,
        );
        let status = Command::new("timeout")
            .arg("15")
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env("FIPE_SIZE_PROBE_CHILD", test_name)
            .env("FIPE_SIZE_PROBE_EXECUTABLE", executable)
            .status()
            .unwrap();
        assert!(status.success(), "bounded execution worker exited {status}");
        return;
    }
    let start = std::time::Instant::now();
    let setup = shared_size_setup(&program, depth);
    let budget = setup.checked_add(4).unwrap();
    let sources = trace::Sources(
        unit.sources
            .iter()
            .chain(&unit.files)
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect(),
    );
    let (mut vm, charged) = on_vm(&program, Some(budget), &sources);
    let fault = vm
        .result
        .as_ref()
        .expect_err("finite budget refuses at Show");
    assert_eq!(fault.reason, Reason::Budget);
    assert_eq!(&source[fault.span.start..fault.span.end], "show tree");
    assert_eq!(program.tables.borrow().namespaces[fault.namespace], "main");
    assert_eq!(charged, budget);
    assert_eq!(vm.trace.len(), 1, "only scalar seed crosses Host admission");
    vm.result = carried(vm.result);
    let executable = std::env::var_os("FIPE_SIZE_PROBE_EXECUTABLE").unwrap();
    let (observed, units, code) =
        native(Path::new(&executable), &program, Some(budget), &sources);
    assert_eq!(
        observed, vm,
        "same located terminal, trace, replies and lines"
    );
    assert_eq!(units, charged);
    assert_eq!(code, 1);
    eprintln!("SIZE_BOUNDED depth={depth} budget={budget} elapsed={:?}; occurrence count is source-derived, not an instrumented loop counter", start.elapsed());
}

#[test]
fn shared_dag_size_preflight_depth12_has_finite_budget_terminal() {
    shared_size_case(
        "shared_dag_size_preflight_depth12_has_finite_budget_terminal",
        12,
    );
}

#[test]
fn shared_dag_size_preflight_depth14_has_finite_budget_terminal() {
    shared_size_case(
        "shared_dag_size_preflight_depth14_has_finite_budget_terminal",
        14,
    );
}

#[test]
fn affordable_shared_dag_preserves_logical_occurrence_price() {
    let mut tail_price = None;
    for (depth, logical) in [(0, 1_u64), (2, 13_u64)] {
        let source = shared_size_source(depth);
        let (unit, program) = Unit::new(
            &source,
            vec![("sys".into(), "pub lookup: () -> Integer".into())],
            vec![],
        )
        .unwrap();
        let setup = shared_size_setup(&program, depth);
        let (observed, charged, code) = equal_prepared(
            &format!("affordable-shared-{depth}"),
            &source,
            &unit,
            &program,
        );
        assert_eq!(observed.result, Ok(Value::text("tree")));
        assert_eq!(code, 0);
        let rest = charged
            .checked_sub(setup + logical)
            .expect("logical descendant price is paid");
        if let Some(expected) = tail_price {
            assert_eq!(rest, expected, "same written Show and return path after the original logical price");
        }
        tail_price = Some(rest);
    }
}

const PREDICATE_NATIVE_HOST: &str =
    "pub items: () -> Vec<Integer>\npub after: () -> Integer\n";
const PREDICATE_NATIVE_GENERIC: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Sorted<a> -> Integer = xs => host.after ()\napply = f x => f x\ngo = xs => apply keep xs\ngo (host.items ())\n";
const PREDICATE_NATIVE_CONCRETE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nx: Sorted<Integer> = host.items ()\nhost.after ()\n";

fn native_sorted_identity(program: &Program, source: &str) -> (usize, usize) {
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "main"
                && source.get(test.span.start..test.span.end)
                    == Some("is_sorted self"))
            .then_some(index)
        })
        .collect();
    let [test] = tests.as_slice() else {
        panic!("one semantic Sorted source predicate");
    };
    (
        *test,
        program.tests[*test].expect("actual lowered Sorted body"),
    )
}

fn native_sorted_failed_at(program: &Program, fault: &Fault, test: usize) {
    let Reason::Failed { entry, .. } = &fault.reason else {
        panic!("Sorted must refute its input, not fail for an unrelated reason: {fault:?}");
    };
    let tables = program.tables.borrow();
    let mut predicates = vec![&tables.entries[*entry].predicate];
    let mut found = false;
    while let Some(predicate) = predicates.pop() {
        match predicate {
            fipe::ir::Predicate::All(parts)
            | fipe::ir::Predicate::Any(parts) => predicates.extend(parts),
            fipe::ir::Predicate::Not(inner) => predicates.push(inner),
            fipe::ir::Predicate::Function { parameter, result } => {
                predicates.push(result);
                predicates.push(parameter);
            }
            fipe::ir::Predicate::Test { index, .. } => found |= *index == test,
            fipe::ir::Predicate::Nonzero { .. } => {}
        }
    }
    assert!(found, "failed entry is the same semantic Sorted test");
}

fn native_sorted_reply(
    program: &Program,
    request: &Request,
    sorted: bool,
) -> Value {
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("only typed Host callbacks in unchanged Sorted fixture");
    };
    let tables = program.tables.borrow();
    let declaration = &tables.declarations[*declaration];
    assert_eq!(tables.namespaces[declaration.namespace], "host");
    assert_eq!(arguments, &[Value::Unit]);
    assert!(environment.is_empty());
    match declaration.label.as_str() {
        "items" => Value::seq(if sorted {
            vec![Value::Int(1), Value::Int(2)]
        } else {
            vec![Value::Int(2), Value::Int(1)]
        }),
        "after" => {
            assert!(sorted, "after cannot run past a refuted Sorted predicate");
            Value::Int(7)
        }
        label => panic!("unexpected typed Host callback {label}"),
    }
}

/// Observe the real joined predicate frame, rather than a guessed function label.
fn native_sorted_observe_integer(
    program: &Program,
    source: &str,
    sorted: bool,
) {
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(program);
    let (test, body) =
        native_sorted_identity(machine.program().unwrap(), source);
    let mut tested = false;
    let mut labels = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if let Some(fipe::vm::Frame::Body(frame)) = machine.frames().last() {
            if frame.body == body {
                let joined = machine.program().unwrap();
                let tables = joined.tables.borrow();
                let body = &joined.bodies[frame.body];
                assert!(
                    !body.hidden.is_empty(),
                    "actual Sorted body has a generic witness"
                );
                assert_eq!(body.hidden.len(), frame.meanings.len());
                for meaning in &frame.meanings {
                    let fipe::ir::Type::Meaning { index, arguments } = meaning
                    else {
                        panic!("actual predicate receives Integer, not template {meaning:?}");
                    };
                    assert_eq!(tables.meanings[*index].label, "Integer");
                    assert!(arguments.is_empty());
                }
                tested = true;
            }
        }
        match stop {
            fipe::vm::Stop::Before(_) | fipe::vm::Stop::After(..) => {}
            fipe::vm::Stop::Request(request) => {
                let Request::Call { declaration, .. } = &request else {
                    panic!("typed callback");
                };
                labels.push(
                    machine.program().unwrap().tables.borrow().declarations
                        [*declaration]
                        .label
                        .clone(),
                );
                let answer = native_sorted_reply(
                    machine.program().unwrap(),
                    &request,
                    sorted,
                );
                machine.answer(Reply::Value(Ok(answer)));
            }
            fipe::vm::Stop::Done(result) => {
                assert!(
                    tested,
                    "the semantic Sorted predicate body must actually execute"
                );
                if sorted {
                    assert_eq!(result.unwrap(), Value::Int(7));
                    assert_eq!(labels, ["items", "after"]);
                } else {
                    native_sorted_failed_at(
                        machine.program().unwrap(),
                        &result.unwrap_err(),
                        test,
                    );
                    assert_eq!(labels, ["items"]);
                }
                return;
            }
        }
    }
    panic!("bounded actual Sorted predicate observation did not finish");
}

fn native_sorted_case(name: &str, source: &str, sorted: bool) {
    let (unit, program) = Unit::new(
        source,
        vec![("host".into(), PREDICATE_NATIVE_HOST.into())],
        vec![],
    )
    .expect("unchanged original Sorted fixture");
    assert!(!program.tables.borrow().has_resources);
    native_sorted_observe_integer(&program, source, sorted);
    let (test, _) = native_sorted_identity(&program, source);
    let on_vm = |budget| {
        host_run_vm(&program, budget, |request| {
            native_sorted_reply(&program, request, sorted)
        })
    };
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| {
            let Request::Call { declaration, .. } = request else {
                panic!("typed call");
            };
            program.tables.borrow().declarations[*declaration]
                .label
                .clone()
        })
        .collect();
    if sorted {
        assert_eq!(whole.0.as_ref().unwrap(), &Value::Int(7));
        assert_eq!(labels, ["items", "after"]);
        assert_eq!(whole.4, 0);
    } else {
        native_sorted_failed_at(&program, whole.0.as_ref().unwrap_err(), test);
        assert_eq!(labels, ["items"]);
        assert_eq!(whole.4, 1);
    }
    assert!(whole.3 > 0);
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget| {
        host_run_native(&executable, budget, |request| {
            native_sorted_reply(&program, request, sorted)
        })
    };
    assert_eq!(
        on_native(None),
        whole,
        "actual predicate result/Fault, typed callbacks, lines, work and exit"
    );
    for limit in [whole.3 / 3, whole.3 / 2, whole.3.saturating_sub(1)] {
        let vm = on_vm(Some(limit));
        assert!(
            matches!(&vm.0, Err(fault) if fault.reason == Reason::Budget),
            "cut is a real budget refusal"
        );
        assert_eq!(
            on_native(Some(limit)),
            vm,
            "actual predicate work/fault boundary at budget {limit}"
        );
    }
}

#[test]
fn generic_sorted_predicate_matches_the_vm() {
    native_sorted_case(
        "generic-sorted-predicate",
        PREDICATE_NATIVE_GENERIC,
        true,
    );
}
#[test]
fn generic_unsorted_predicate_matches_the_vm() {
    native_sorted_case(
        "generic-unsorted-predicate",
        PREDICATE_NATIVE_GENERIC,
        false,
    );
}
#[test]
fn concrete_sorted_predicate_matches_the_vm() {
    native_sorted_case(
        "concrete-sorted-predicate",
        PREDICATE_NATIVE_CONCRETE,
        true,
    );
}
#[test]
fn concrete_unsorted_predicate_matches_the_vm() {
    native_sorted_case(
        "concrete-unsorted-predicate",
        PREDICATE_NATIVE_CONCRETE,
        false,
    );
}

fn raw_host_semantic_test(program: &Program, host: &str) -> usize {
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "host"
                && host.get(test.span.start..test.span.end)
                    == Some("self.len > 0"))
            .then_some(index)
        })
        .collect();
    let [test] = tests.as_slice() else {
        panic!("one semantic Host Nonempty test")
    };
    assert!(
        program.tests[*test].is_some(),
        "actual supplied predicate body is lowered"
    );
    *test
}

fn raw_host_helper_case(
    name: &str,
    host: &str,
    source: &str,
    expected: Option<&Value>,
    expected_calls: &[&str],
) {
    let (unit, program) =
        Unit::new(source, vec![("host".into(), host.into())], vec![])
            .expect("exact qualified raw Host predicate fixture");
    assert!(!program.tables.borrow().has_resources);
    let test = raw_host_semantic_test(&program, host);
    let reply = |request: &Request| {
        let Request::Call {
            declaration,
            arguments,
            environment,
        } = request
        else {
            panic!("only typed Host callbacks")
        };
        let tables = program.tables.borrow();
        let declaration = &tables.declarations[*declaration];
        assert_eq!(tables.namespaces[declaration.namespace], "host");
        assert_eq!(declaration.label, "after");
        assert_eq!(arguments, &[Value::Int(0)]);
        assert!(environment.is_empty());
        Value::Int(7)
    };
    let on_vm = |budget| host_run_vm(&program, budget, reply);
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| {
            let Request::Call { declaration, .. } = request else {
                panic!("typed Host callback")
            };
            program.tables.borrow().declarations[*declaration]
                .label
                .clone()
        })
        .collect();
    assert_eq!(labels, expected_calls);
    if let Some(value) = expected {
        assert_eq!(whole.0.as_ref().unwrap(), value);
        assert_eq!(whole.4, 0);
    } else {
        let fault = whole.0.as_ref().unwrap_err();
        let Reason::Failed { entry, .. } = &fault.reason else {
            panic!("Nonempty refuses input: {fault:?}")
        };
        let tables = program.tables.borrow();
        let mut predicates = vec![&tables.entries[*entry].predicate];
        let mut same_test = false;
        while let Some(predicate) = predicates.pop() {
            match predicate {
                fipe::ir::Predicate::All(parts)
                | fipe::ir::Predicate::Any(parts) => predicates.extend(parts),
                fipe::ir::Predicate::Not(inner) => predicates.push(inner),
                fipe::ir::Predicate::Function { parameter, result } => {
                    predicates.push(result);
                    predicates.push(parameter);
                }
                fipe::ir::Predicate::Test { index, .. } => {
                    same_test |= *index == test;
                }
                fipe::ir::Predicate::Nonzero { .. } => {}
            }
        }
        assert!(
            same_test,
            "refusal names the actual Host Nonempty predicate"
        );
        assert_eq!(whole.4, 1);
    }
    assert!(whole.3 > 1);
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget| host_run_native(&executable, budget, reply);
    assert_eq!(
        on_native(None),
        whole,
        "full raw Host outcome/Fault, callbacks/replies/lines, work and exit"
    );
    for limit in [whole.3 / 2, whole.3.saturating_sub(1)] {
        let vm = on_vm(Some(limit));
        assert!(
            matches!(&vm.0, Err(fault) if fault.reason == Reason::Budget),
            "cut is a real Budget refusal"
        );
        assert_eq!(
            on_native(Some(limit)),
            vm,
            "raw Host predicate parity at budget {limit}"
        );
    }
}

#[test]
fn raw_host_helper_fixed_predicate_matches_vm() {
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub helper: Vec<Integer> -> Integer = xs => {\n checked: Nonempty<Integer> = xs\n 0\n}\npub after: Integer -> Integer\n";
    raw_host_helper_case(
        "raw-host-helper-empty",
        HOST,
        "use host\nhost.after (host.helper [])\n",
        None,
        &[],
    );
    raw_host_helper_case(
        "raw-host-helper-nonempty",
        HOST,
        "use host\nhost.after (host.helper [1])\n",
        Some(&Value::Int(7)),
        &["after"],
    );
}

#[test]
fn raw_host_helper_generic_default_matches_vm() {
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type R<T> { pub items: Vec<T>, checked: Integer = (helper items) }\npub helper: Vec<a> -> Integer = xs => {\n checked: Nonempty<a> = xs\n 0\n}\npub read: R<a> -> Integer = r => r.checked\n";
    for (name, uses) in [
        (
            "raw-host-helper-default-string-first",
            "(host.read (make [\"s\"]), host.read (make [1]))",
        ),
        (
            "raw-host-helper-default-int-first",
            "(host.read (make [1]), host.read (make [\"s\"]))",
        ),
    ] {
        let source =
            format!("use host\nmake = xs => host.R {{ items: xs }}\n{uses}\n");
        raw_host_helper_case(
            name,
            HOST,
            &source,
            Some(&Value::Tuple(std::rc::Rc::new(vec![
                Value::Int(0),
                Value::Int(0),
            ]))),
            &[],
        );
    }
}

#[test]
fn raw_host_generic_helper_body_predicate_matches_vm() {
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub helper: Vec<a> -> Integer = xs => {\n checked: Nonempty<a> = xs\n 0\n}\npub after: Integer -> Integer\n";
    raw_host_helper_case(
        "raw-host-generic-helper-empty",
        HOST,
        "use host\nhost.after (host.helper [])\n",
        None,
        &[],
    );
    raw_host_helper_case(
        "raw-host-generic-helper-nonempty",
        HOST,
        "use host\nhost.after (host.helper [1])\n",
        Some(&Value::Int(7)),
        &["after"],
    );
}

#[test]
fn retained_default_tests_follow_used_and_nested_replays_only() {
    const HOST: &str = "pub type Leaf { pub n: U64 = 1 }\npub type Outer { pub leaf: Leaf = Leaf }\n";
    for (source, layers) in [
        ("use host\nhost.Leaf\n", 1),
        ("use host\nhost.Outer\n", 2),
        ("use host\n1\n", 0),
    ] {
        let (_, program) =
            Unit::new(source, vec![("host".into(), HOST.into())], vec![])
                .unwrap();
        let mut machine = Machine::new(None);
        let Outcome::Done(Ok(mut value)) = machine.run(&program) else {
            panic!("literal defaults require no host effect")
        };
        assert!(
            machine
                .program()
                .unwrap()
                .tables
                .borrow()
                .entries
                .is_empty(),
            "proved literal defaults have no executable open check"
        );
        for _ in 0..layers {
            let Value::Record(record) = value else {
                panic!("actual declared record layer")
            };
            assert_eq!(record.fields.len(), 1);
            value = record.fields[0].clone();
        }
        assert_eq!(value, Value::Int(1));
        if layers == 0 {
            assert!(
                machine.program().unwrap().tables.borrow().tests.is_empty(),
                "unused defaults introduce no test catalog"
            );
        }
    }
}

fn retained_u64_test(program: &Program) -> usize {
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "prelude"
                && fipe::decl::prelude_source()
                    .get(test.span.start..test.span.end)
                    == Some("self <= 18446744073709551615"))
            .then_some(index)
        })
        .collect();
    assert_eq!(
        tests.len(),
        1,
        "the unknown default requires the actual U64 test"
    );
    assert!(program.tests[tests[0]].is_some());
    tests[0]
}

fn retained_dynamic_u64_case(
    name: &str,
    source: &str,
    supplied: &Value,
    healthy: bool,
) {
    const HOST: &str = "pub value: () -> Integer\npub after: Integer -> Integer\npub type Leaf { pub n: U64 = (value ()) }\npub type Outer { pub leaf: Leaf = Leaf }\n";
    let (unit, program) =
        Unit::new(source, vec![("host".into(), HOST.into())], vec![]).unwrap();
    let _test = retained_u64_test(&program);
    let reply = |request: &Request| {
        let Request::Call {
            declaration,
            arguments,
            environment,
        } = request
        else {
            panic!("{request:?}")
        };
        let tables = program.tables.borrow();
        let declaration = &tables.declarations[*declaration];
        assert_eq!(tables.namespaces[declaration.namespace], "host");
        assert!(environment.is_empty());
        match declaration.label.as_str() {
            "value" => {
                assert_eq!(arguments, &[Value::Unit]);
                supplied.clone()
            }
            "after" => {
                assert!(healthy);
                assert_eq!(arguments, &[Value::Int(7)]);
                Value::Int(9)
            }
            label => panic!("unexpected {label}"),
        }
    };
    let whole = host_run_vm(&program, None, reply);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| {
            let Request::Call { declaration, .. } = request else {
                panic!("{request:?}")
            };
            program.tables.borrow().declarations[*declaration]
                .label
                .clone()
        })
        .collect();
    if healthy {
        assert_eq!(labels, ["value", "after"]);
        assert_eq!(whole.0.as_ref().unwrap(), &Value::Int(9));
    } else {
        assert_eq!(labels, ["value"], "the own U64 refusal precedes after");
        let fault = whole.0.as_ref().unwrap_err();
        let Reason::Failed { entry, value } = &fault.reason else {
            panic!("{fault:?}")
        };
        assert_eq!(value, "18446744073709551616");
        let mut machine = Machine::new(None);
        let Outcome::Request(request) = machine.run(&program) else {
            panic!("the default must request its value")
        };
        let answer = reply(&request);
        let Outcome::Done(Err(joined_fault)) =
            machine.reply(&program, Reply::Value(Ok(answer)))
        else {
            panic!("the oversized default must refuse immediately")
        };
        assert_eq!(&joined_fault, fault);
        let joined = machine.program().unwrap();
        let joined_test = retained_u64_test(joined);
        let tables = joined.tables.borrow();
        let mut pending = vec![&tables.entries[*entry].predicate];
        let mut own = false;
        while let Some(predicate) = pending.pop() {
            match predicate {
                fipe::ir::Predicate::All(parts)
                | fipe::ir::Predicate::Any(parts) => pending.extend(parts),
                fipe::ir::Predicate::Not(inner) => pending.push(inner),
                fipe::ir::Predicate::Function { parameter, result } => {
                    pending.push(parameter);
                    pending.push(result);
                }
                fipe::ir::Predicate::Test { index, .. } => {
                    own |= *index == joined_test;
                }
                fipe::ir::Predicate::Nonzero { .. } => {}
            }
        }
        assert!(
            own,
            "the failed entry contains the actual U64 upper-bound test"
        );
    }
    let executable = common::executable(name, &unit, &program);
    assert_eq!(host_run_native(&executable, None, reply), whole);
    for limit in [whole.3 / 2, whole.3.saturating_sub(1)] {
        let vm = host_run_vm(&program, Some(limit), reply);
        assert!(matches!(&vm.0, Err(fault) if fault.reason == Reason::Budget));
        assert_eq!(host_run_native(&executable, Some(limit), reply), vm);
    }
}

#[test]
fn dynamic_retained_default_tests_cover_used_nested_and_unused_sources() {
    const HOST: &str = "pub value: () -> Integer\npub type Leaf { pub n: U64 = (value ()) }\npub type Outer { pub leaf: Leaf = Leaf }\n";
    let too_big = Value::integer(
        false,
        fipe::token::Magnitude::parse("18446744073709551616", 10).unwrap(),
    );
    for (label, source) in [
        ("leaf", "use host\nhost.after ((host.Leaf).n)\n"),
        ("outer", "use host\nhost.after ((host.Outer).leaf.n)\n"),
    ] {
        retained_dynamic_u64_case(
            &format!("dynamic-default-{label}-good"),
            source,
            &Value::Int(7),
            true,
        );
        retained_dynamic_u64_case(
            &format!("dynamic-default-{label}-bad"),
            source,
            &too_big,
            false,
        );
    }
    let (_, program) =
        Unit::new("use host\n1\n", vec![("host".into(), HOST.into())], vec![])
            .unwrap();
    let mut machine = Machine::new(None);
    assert_eq!(
        machine.run(&program),
        Outcome::Done(Ok(Value::Int(1))),
        "unused defaults do not request the host"
    );
    let tables = machine.program().unwrap().tables.borrow();
    assert!(
        !tables
            .tests
            .iter()
            .any(|test| tables.namespaces[test.namespace] == "prelude"
                && fipe::decl::prelude_source()
                    .get(test.span.start..test.span.end)
                    == Some("self <= 18446744073709551615")),
        "unused defaults do not emit the U64 test catalog"
    );
}

#[test]
fn default_check_reports_preserve_strict_and_compile_decisions() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("host", "pub value: () -> Integer\n")
        .unwrap();
    let dynamic = "use host\ntype Positive: Integer where self > 0\ntype R { n: Positive = (host.value ()) }\nR\n";
    let checked = |source: &str, options| {
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        fipe::check::check(&module, &declarations, &[], options)
    };
    let module = fipe::parse::parse(fipe::lex::lex(dynamic)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let receiving = dynamic.rfind("\nR\n").unwrap() + 1;
    assert!(typed.checks.iter().any(|check| check.name == "Positive"
        && check.role == fipe::limits::CheckRole::CallerReport
        && check.place.start == receiving
        && check.place.end == receiving + 1));
    assert!(
        typed.default_instances.values().any(|instance| instance
            .typed
            .checks
            .iter()
            .any(|check| check.name == "Positive"
                && check.role == fipe::limits::CheckRole::Local)),
        "the retained default owns the executable check"
    );
    let strict = |source: &str, errors: &[fipe::check::Error]| {
        let receiving = source.rfind("\nR\n").unwrap() + 1;
        assert!(errors.iter().any(|error| matches!(error,
            fipe::check::Error::Type(fipe::infer::Error { span, reason: fipe::infer::Reason::Limit { message } })
                if span.start == receiving && span.end == receiving + 1
                    && message.starts_with("open entry against Positive in strict session:"))),
            "the caller R span owns the Positive strict diagnostic: {errors:?}");
    };
    strict(
        dynamic,
        &checked(
            dynamic,
            fipe::check::Options {
                strict: true,
                ..Default::default()
            },
        )
        .unwrap_err(),
    );
    let good = "is_positive: Integer -> Bool = n => n > 0\ntype Positive: Integer where is_positive self\ntype R { n: Positive = 7 }\nR\n";
    let bad = good.replace("= 7", "= 0");
    let unit = checked(good, fipe::check::Options::default()).unwrap();
    assert!(
        unit.plan.is_empty(),
        "the actual default entry settles at compile time"
    );
    let bad_errors =
        checked(&bad, fipe::check::Options::default()).unwrap_err();
    assert!(bad_errors.iter().any(|error| matches!(error,
        fipe::check::Error::Compile { what: fipe::check::Evaluated::Entry { limit, value: Some(value) }, reason: Reason::Failed { .. }, .. }
            if limit == "Positive" && value == "0")),
        "known bad default reports the actual Positive compile refusal: {bad_errors:?}");
    let exhausted = checked(
        good,
        fipe::check::Options {
            compile_budget: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        exhausted
            .plan
            .iter()
            .any(|entry| entry.reason == fipe::ir::Reason::CompileBudget),
        "the actual exhausted default entry stays open"
    );
    strict(
        good,
        &checked(
            good,
            fipe::check::Options {
                compile_budget: 1,
                strict: true,
            },
        )
        .unwrap_err(),
    );
}

const CURRENT_CALLABLE_U8_HOST: &str =
    "pub tick: () -> Integer\npub after: () -> Integer\npub arg: () -> Integer";
const CURRENT_CALLABLE_U8_GENERIC: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\nget = m => m.op\nf = get (M 7)\nhost.after ()\nf (host.arg ())";
const CURRENT_CALLABLE_U8_DIRECT: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\nf = (M 7).op\nhost.after ()\nf (host.arg ())";
const CURRENT_CALLABLE_U8_PLAIN: &str = "use host\nk: Integer -> U8 -> U8 = self => (x => x)\nget = m => k m\nf = get 7\nhost.after ()\nf (host.arg ())";
const CURRENT_CALLABLE_DYNAMIC_SORTED: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ntype M<T: Ord> { n: Integer }\nimpl M<a> { op: Self -> Sorted<a> -> Integer = self => (xs => host.after ()) }\nget = m => m.op\nx: M<Integer> = M 7\nf = get x\nf (host.items ())";
const CURRENT_CALLABLE_PRESTRUCTURED_HOST: &str =
    "pub after: () -> Integer\npub arg: () -> Integer\n";
const CURRENT_CALLABLE_PRESTRUCTURED_GET: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()) }\nget = m => m.op\nf = get (M 7)\nprovide: Integer -> U8 = n => 3\nf [provide]";
const CURRENT_CALLABLE_PRESTRUCTURED_INVOKE: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()) }\ninvoke = m fs => m.op fs\nprovide: Integer -> U8 = n => 3\ninvoke (M 7) [provide]";
fn current_callable_host_label(program: &Program, request: &Request) -> String {
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("only the exact typed Host callbacks are allowed: {request:?}")
    };
    assert_eq!(arguments, &[Value::Unit]);
    assert!(environment.is_empty());
    let tables = program.tables.borrow();
    let declaration = &tables.declarations[*declaration];
    assert_eq!(tables.namespaces[declaration.namespace], "host");
    declaration.label.clone()
}

fn current_callable_u8_reply(
    program: &Program,
    request: &Request,
    argument: i64,
    labels: &mut Vec<String>,
) -> Value {
    let label = current_callable_host_label(program, request);
    labels.push(label.clone());
    match label.as_str() {
        "after" => {
            assert_eq!(
                labels.as_slice(),
                ["after"],
                "original earlier effect precedes argument/refusal"
            );
            Value::Int(99)
        }
        "arg" => {
            assert_eq!(labels.as_slice(), ["after", "arg"]);
            Value::Int(argument)
        }
        _ => panic!("no tick or extra effect in original U8 fixture: {label}"),
    }
}

fn current_callable_assert_parity(
    name: &str,
    whole: &HostRun,
    on_vm: impl Fn(Option<u64>) -> HostRun,
    on_native: impl Fn(Option<u64>) -> HostRun,
) {
    assert!(whole.3 > 0);
    assert_eq!(&on_native(None), whole, "{name}: complete result/located Fault, callbacks/replies/lines, work and exit");
    for allowance in [whole.3 / 3, whole.3 / 2, whole.3.saturating_sub(1)] {
        let expected = on_vm(Some(allowance));
        assert!(matches!(&expected.0, Err(fault) if fault.reason == Reason::Budget),
            "{name}: measured cut {allowance} is actual Budget, not semantic refusal");
        assert_eq!(on_native(Some(allowance)), expected,
            "{name}: exact whole/cut-work and callback-prefix contract at {allowance}");
    }
}

fn current_callable_u8_case(name: &str, source: &str, argument: i64) {
    let (unit, program) = Unit::new(
        source,
        vec![("host".into(), CURRENT_CALLABLE_U8_HOST.into())],
        vec![],
    )
    .expect("unchanged qualified current-source U8 fixture");
    assert!(!program.tables.borrow().has_resources);
    let on_vm = |budget| {
        let mut labels = Vec::new();
        host_run_vm(&program, budget, |request| {
            current_callable_u8_reply(&program, request, argument, &mut labels)
        })
    };
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| current_callable_host_label(&program, request))
        .collect();
    assert_eq!(labels, ["after", "arg"]);
    if argument == 3 {
        assert_eq!(whole.0.as_ref().unwrap(), &Value::Int(3));
        assert_eq!(whole.4, 0);
    } else {
        assert_eq!(argument, 300);
        let fault = whole
            .0
            .as_ref()
            .expect_err("original authored U8 contract must refuse 300");
        let Reason::Failed { entry, value } = &fault.reason else {
            panic!("{fault:?}")
        };
        assert_eq!(value, "300");
        let tables = program.tables.borrow();
        let label = &tables.entries[*entry].label;
        assert!(
            label.starts_with("U8") || label.contains("-> U8"),
            "actual Failed entry must identify authored U8 limit: {label}"
        );
        assert_eq!(tables.namespaces[fault.namespace], "main");
        assert!(
            fault.span.start < fault.span.end && fault.span.end <= source.len()
        );
        assert_eq!(whole.4, 1);
    }
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget| {
        let mut labels = Vec::new();
        host_run_native(&executable, budget, |request| {
            current_callable_u8_reply(&program, request, argument, &mut labels)
        })
    };
    current_callable_assert_parity(name, &whole, on_vm, on_native);
}

fn current_callable_prestructured_case(name: &str, source: &str) {
    let (unit, program) = Unit::new(
        source,
        vec![("host".into(), CURRENT_CALLABLE_PRESTRUCTURED_HOST.into())],
        vec![],
    )
    .expect("unchanged qualified healthy prestructured function-list fixture");
    assert!(!program.tables.borrow().has_resources);
    let reply = |request: &Request| {
        assert_eq!(
            current_callable_host_label(&program, request),
            "after",
            "provide is not executed or sent to the Host"
        );
        Value::Int(7)
    };
    let on_vm = |budget| host_run_vm(&program, budget, reply);
    let whole = on_vm(None);
    assert_eq!(whole.0.as_ref().unwrap(), &Value::Int(7));
    assert_eq!(whole.1.len(), 1);
    assert_eq!(
        current_callable_host_label(&program, &whole.1[0].0),
        "after"
    );
    assert_eq!(whole.4, 0);
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget| host_run_native(&executable, budget, reply);
    current_callable_assert_parity(name, &whole, on_vm, on_native);
}

#[test]
fn current_source_callable_u8_generic_accepts() {
    current_callable_u8_case(
        "current-callable-u8-generic-3",
        CURRENT_CALLABLE_U8_GENERIC,
        3,
    );
}

#[test]
fn current_source_callable_u8_generic_refuses() {
    current_callable_u8_case(
        "current-callable-u8-generic-300",
        CURRENT_CALLABLE_U8_GENERIC,
        300,
    );
}

#[test]
fn current_source_callable_u8_direct_accepts() {
    current_callable_u8_case(
        "current-callable-u8-direct-3",
        CURRENT_CALLABLE_U8_DIRECT,
        3,
    );
}

#[test]
fn current_source_callable_u8_direct_refuses() {
    current_callable_u8_case(
        "current-callable-u8-direct-300",
        CURRENT_CALLABLE_U8_DIRECT,
        300,
    );
}

#[test]
fn current_source_callable_u8_plain_accepts() {
    current_callable_u8_case(
        "current-callable-u8-plain-3",
        CURRENT_CALLABLE_U8_PLAIN,
        3,
    );
}

#[test]
fn current_source_callable_u8_plain_refuses() {
    current_callable_u8_case(
        "current-callable-u8-plain-300",
        CURRENT_CALLABLE_U8_PLAIN,
        300,
    );
}

#[test]
fn current_source_callable_dynamic_sorted_accepts() {
    native_sorted_case(
        "current-callable-dynamic-sorted",
        CURRENT_CALLABLE_DYNAMIC_SORTED,
        true,
    );
}

#[test]
fn current_source_callable_dynamic_sorted_refuses() {
    native_sorted_case(
        "current-callable-dynamic-unsorted",
        CURRENT_CALLABLE_DYNAMIC_SORTED,
        false,
    );
}

#[test]
fn current_source_callable_prestructured_get() {
    current_callable_prestructured_case(
        "current-callable-prestructured-get",
        CURRENT_CALLABLE_PRESTRUCTURED_GET,
    );
}

#[test]
fn current_source_callable_prestructured_invoke() {
    current_callable_prestructured_case(
        "current-callable-prestructured-invoke",
        CURRENT_CALLABLE_PRESTRUCTURED_INVOKE,
    );
}

// These fixtures retain the public bodyless-row boundary independently of written methods.
const ROW_NATIVE_METHODS: &str =
    "pub type M { pub n: Integer }\nimpl M { op: U8 -> (self: Self) -> U8 }\n";
const ROW_NATIVE_HOST: &str =
    "pub after: () -> Integer\npub arg: () -> Integer\n";
const ROW_NATIVE_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ())";
const ROW_NATIVE_SORTED_METHODS: &str = "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\npub type Sorted<T: Ord>: Vec<T> where is_sorted self\npub type M<T: Ord> { pub token: T }\nimpl M { op: Sorted<T> -> (self: Self) -> Integer }\n";
const ROW_NATIVE_SORTED_INTEGER: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ())\n";
const ROW_NATIVE_SORTED_STRING: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M \"tag\")\nhost.after ()\nf (host.arg ())\n";
const ROW_NATIVE_UNBOUND_METHODS: &str = "pub type Tagged<T>: Integer where self < 10\npub type M { pub token: Integer }\nimpl M { op: Tagged<a> -> (self: Self) -> Integer }\n";
const ROW_NATIVE_PARTIAL_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: U8 -> Integer -> (self: Self) -> Integer }\n";
const ROW_NATIVE_PARTIAL_HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub saved: (Integer -> Integer) -> Integer\n";
const ROW_NATIVE_PARTIAL_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\ng = f (host.arg ())\nhost.saved g\ng 9\n";

fn row_native_call<'a>(
    program: &Program,
    request: &'a Request,
) -> (String, &'a [Value]) {
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!(
            "only the supplied typed Host callbacks are allowed: {request:?}"
        )
    };
    assert!(environment.is_empty());
    let tables = program.tables.borrow();
    let declaration = &tables.declarations[*declaration];
    let namespace = &tables.namespaces[declaration.namespace];
    assert!(namespace == "host" || namespace == "methods");
    (format!("{namespace}.{}", declaration.label), arguments)
}

fn row_native_reply(
    program: &Program,
    request: &Request,
    input: &Value,
    partial: bool,
) -> Value {
    let (label, arguments) = row_native_call(program, request);
    match label.as_str() {
        "host.after" | "host.saved" => {
            assert_eq!(arguments, [Value::Unit]);
            if label == "host.saved" {
                assert!(partial);
            }
            Value::Int(7)
        }
        "host.arg" => {
            assert_eq!(arguments, [Value::Unit]);
            input.clone()
        }
        "methods.op" => {
            let (actual, receiver) = if partial {
                let [actual, Value::Int(9), Value::Record(receiver)] =
                    arguments
                else {
                    panic!("the actual raw three-spot operation: {arguments:?}")
                };
                (actual, receiver)
            } else {
                let [actual, Value::Record(receiver)] = arguments else {
                    panic!("the actual raw two-spot operation: {arguments:?}")
                };
                (actual, receiver)
            };
            assert_eq!(actual, input);
            assert_eq!(receiver.fields.len(), 1);
            assert!(
                receiver.fields[0] == Value::Int(7)
                    || receiver.fields[0] == Value::text("tag")
            );
            if matches!(input, Value::Int(3)) && !partial {
                Value::Int(3)
            } else {
                Value::Int(7)
            }
        }
        _ => panic!("no extra callback is admitted: {label}"),
    }
}

fn row_native_u8_failed(program: &Program, source: &str, run: &HostRun) {
    let fault = run
        .0
        .as_ref()
        .expect_err("300 is refused before the bodyless operation");
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "300");
    let tables = program.tables.borrow();
    assert_eq!(tables.entries[*entry].label, "U8");
    assert_eq!(tables.namespaces[fault.namespace], "main");
    assert!(
        fault.span.start < fault.span.end && fault.span.end <= source.len()
    );
    assert_eq!(run.4, 1);
}

fn row_native_closed_call(program: &Program, source: &str) {
    let text = "f (host.arg ())";
    let start = source.find(text).unwrap();
    let tables = program.tables.borrow();
    assert!(
        program
            .bodies
            .iter()
            .any(|body| body.blocks.iter().any(|block| {
                matches!(&block.terminator, fipe::mir::Terminator::Call {
            callee: fipe::mir::Callee::Value(_), args, ..
        } if args.len() == 1)
                    && block.nodes.last().is_some_and(|node| {
                        let node = &tables.nodes[*node];
                        tables.namespaces[node.namespace] == "main"
                            && node.span.start == start
                            && node.span.end == start + text.len()
                    })
            })),
        "actual source-owned closed Value call supplies one argument"
    );
}

fn row_native_partial_observe(argument: i64) {
    let (_, program) = Unit::new(
        ROW_NATIVE_PARTIAL_MAIN,
        vec![
            ("host".into(), ROW_NATIVE_PARTIAL_HOST.into()),
            ("methods".into(), ROW_NATIVE_PARTIAL_METHODS.into()),
        ],
        vec![],
    )
    .expect("the original saved-Function VM fixture must be admitted");
    row_native_closed_call(&program, ROW_NATIVE_PARTIAL_MAIN);
    let input = Value::Int(argument);
    let mut saved = false;
    let whole = host_run_vm_observed(&program, None, |machine, request| {
        let joined = machine.program().unwrap();
        let (label, arguments) = row_native_call(joined, request);
        if label == "host.saved" {
            assert_eq!(
                argument, 3,
                "invalid input is refused before the saved partial escapes"
            );
            let [Value::Function(partial)] = arguments else {
                panic!("actual saved Function")
            };
            assert!(joined
                .receiver_adapters
                .iter()
                .any(|adapter| adapter.body == partial.body));
            assert_eq!(joined.bodies[partial.body].arity, 3);
            assert_eq!(
                partial.filled.len(),
                2,
                "same adapter rather than an open wrapper"
            );
            assert!(
                matches!(&partial.filled[0], Value::Record(receiver) if receiver.fields == [Value::Int(7)])
            );
            assert_eq!(partial.filled[1], Value::Int(3));
            saved = true;
            Value::Int(7)
        } else {
            row_native_reply(joined, request, &input, true)
        }
    });
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| row_native_call(&program, request).0)
        .collect();
    if argument == 3 {
        assert!(saved);
        assert_eq!(whole.0, Ok(Value::Int(7)));
        assert_eq!(
            labels,
            ["host.after", "host.arg", "host.saved", "methods.op"]
        );
    } else {
        assert!(!saved);
        row_native_u8_failed(&program, ROW_NATIVE_PARTIAL_MAIN, &whole);
        assert_eq!(labels, ["host.after", "host.arg"]);
    }
}

fn row_native_case(
    name: &str,
    sources: (&str, &str, &str),
    input: &Value,
    healthy: bool,
    partial: bool,
    sorted: bool,
) {
    let (methods, source, host) = sources;
    let (unit, program) = Unit::new(
        source,
        vec![
            ("host".into(), host.into()),
            ("methods".into(), methods.into()),
        ],
        vec![],
    )
    .expect("the exact bodyless-row fixture is source-admitted");
    assert!(!program.tables.borrow().has_resources);
    if partial {
        row_native_closed_call(&program, source);
    }
    let on_vm = |budget| {
        host_run_vm(&program, budget, |request| {
            row_native_reply(&program, request, input, partial)
        })
    };
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| row_native_call(&program, request).0)
        .collect();
    if healthy {
        assert_eq!(
            whole.0,
            Ok(if sorted || partial {
                Value::Int(7)
            } else {
                Value::Int(3)
            })
        );
        assert_eq!(whole.4, 0);
        assert_eq!(
            labels,
            if partial {
                vec!["host.after", "host.arg", "host.saved", "methods.op"]
            } else {
                vec!["host.after", "host.arg", "methods.op"]
            }
        );
    } else {
        assert_eq!(
            labels,
            ["host.after", "host.arg"],
            "refusal precedes sentinel and operation callbacks"
        );
        if sorted {
            let tables = program.tables.borrow();
            let tests: Vec<_> = tables
                .tests
                .iter()
                .enumerate()
                .filter_map(|(index, test)| {
                    (tables.namespaces[test.namespace] == "methods"
                        && methods.get(test.span.start..test.span.end)
                            == Some("is_sorted self"))
                    .then_some(index)
                })
                .collect();
            let [test] = tests.as_slice() else {
                panic!("one real methods Sorted predicate")
            };
            drop(tables);
            native_sorted_failed_at(
                &program,
                whole.0.as_ref().unwrap_err(),
                *test,
            );
            assert_eq!(whole.4, 1);
        } else {
            row_native_u8_failed(&program, source, &whole);
        }
    }
    let executable = common::executable(name, &unit, &program);
    let on_native = |budget| {
        host_run_native(&executable, budget, |request| {
            row_native_reply(&program, request, input, partial)
        })
    };
    current_callable_assert_parity(name, &whole, on_vm, on_native);
}

fn row_native_u8(argument: i64) {
    row_native_case(
        &format!("bodyless-u8-{argument}"),
        (ROW_NATIVE_METHODS, ROW_NATIVE_MAIN, ROW_NATIVE_HOST),
        &Value::Int(argument),
        argument == 3,
        false,
        false,
    );
}

fn row_native_sorted(strings: bool, healthy: bool) {
    let (source, ty, values) = if strings {
        (
            ROW_NATIVE_SORTED_STRING,
            "String",
            if healthy {
                vec![Value::text("a"), Value::text("b")]
            } else {
                vec![Value::text("b"), Value::text("a")]
            },
        )
    } else {
        (
            ROW_NATIVE_SORTED_INTEGER,
            "Integer",
            if healthy {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            },
        )
    };
    let host = format!("pub after: () -> Integer\npub arg: () -> Vec<{ty}>\n");
    row_native_case(
        &format!("bodyless-sorted-{ty}-{healthy}"),
        (ROW_NATIVE_SORTED_METHODS, source, &host),
        &Value::seq(values),
        healthy,
        false,
        true,
    );
}

fn row_native_partial_sentinel(argument: i64) {
    // Function inputs are not representable in the existing native Call codec.
    // The original VM fixture above proves the actual partial before this scalar sentinel variant.
    row_native_partial_observe(argument);
    let host = ROW_NATIVE_PARTIAL_HOST.replace(
        "pub saved: (Integer -> Integer) -> Integer",
        "pub saved: () -> Integer",
    );
    let source =
        ROW_NATIVE_PARTIAL_MAIN.replace("host.saved g", "host.saved ()");
    row_native_case(
        &format!("bodyless-partial-sentinel-{argument}"),
        (ROW_NATIVE_PARTIAL_METHODS, &source, &host),
        &Value::Int(argument),
        argument == 3,
        true,
        false,
    );
}

#[test]
fn bodyless_row_u8_accepts_before_host_operation() {
    row_native_u8(3);
}
#[test]
fn bodyless_row_u8_refuses_before_host_operation() {
    row_native_u8(300);
}
#[test]
fn bodyless_row_sorted_integer_accepts_before_host_operation() {
    row_native_sorted(false, true);
}
#[test]
fn bodyless_row_sorted_integer_refuses_before_host_operation() {
    row_native_sorted(false, false);
}
#[test]
fn bodyless_row_sorted_string_accepts_before_host_operation() {
    row_native_sorted(true, true);
}
#[test]
fn bodyless_row_sorted_string_refuses_before_host_operation() {
    row_native_sorted(true, false);
}
#[test]
fn bodyless_row_partial_sentinel_accepts_after_actual_adapter_partial() {
    row_native_partial_sentinel(3);
}
#[test]
fn bodyless_row_partial_sentinel_refuses_before_saved_and_operation() {
    row_native_partial_sentinel(300);
}

#[test]
fn bodyless_row_unbound_predicate_refuses_before_all_callbacks() {
    let (unit, program) = Unit::new(
        ROW_NATIVE_MAIN,
        vec![
            ("host".into(), ROW_NATIVE_HOST.into()),
            ("methods".into(), ROW_NATIVE_UNBOUND_METHODS.into()),
        ],
        vec![],
    )
    .expect("the exact unbound predicate fixture is admitted");
    let on_vm = |budget| {
        host_run_vm(&program, budget, |request| {
            panic!("missing predicate witness precedes every callback: {request:?}")
        })
    };
    let whole = on_vm(None);
    assert!(whole.1.is_empty());
    let fault = whole.0.as_ref().unwrap_err();
    assert_eq!(
        fault.reason,
        Reason::Host("missing predicate witness".into())
    );
    let tables = program.tables.borrow();
    assert_eq!(tables.namespaces[fault.namespace], "main");
    assert!(
        fault.span.start < fault.span.end
            && fault.span.end <= ROW_NATIVE_MAIN.len()
    );
    drop(tables);
    assert_eq!(whole.4, 1);
    let executable =
        common::executable("bodyless-unbound-predicate", &unit, &program);
    let on_native = |budget| {
        host_run_native(&executable, budget, |request| {
            panic!("no callback on native missing witness: {request:?}")
        })
    };
    current_callable_assert_parity(
        "bodyless-unbound-predicate",
        &whole,
        on_vm,
        on_native,
    );
}

const NATIVE_BOUND_NEXT_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ()) (host.later ())\n";
const NATIVE_BOUND_NEXT_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: U8 -> Integer -> (self: Self) -> Integer }\n";
const NATIVE_BOUND_NEXT_HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub later: () -> Integer\n";
const NATIVE_BOUND_HOLE_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\ng = f (host.arg ())\nhost.saved g\nTrue | g\n";
const NATIVE_BOUND_HOLE_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: (enabled: Bool) -> (n: U8) -> (self: Self) -> Integer }\n";
const NATIVE_BOUND_HOLE_HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub saved: (Bool -> Integer) -> Integer\n";

fn native_bound_selected(machine: &Machine) -> (usize, usize) {
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let globals: Vec<_> = machine
        .globals()
        .filter(|(index, _)| {
            let definition = &tables.definitions[*index];
            tables.namespaces[definition.namespace] == "main"
                && definition.label == "f"
        })
        .collect();
    let [(_, Value::Function(function))] = globals.as_slice() else {
        panic!("actual bound method f")
    };
    let adapters: Vec<_> = joined
        .receiver_adapters
        .iter()
        .filter(|adapter| adapter.body == function.body)
        .collect();
    let [adapter] = adapters.as_slice() else {
        panic!("actual joined adapter")
    };
    let row = &tables.dispatch[adapter.member].rows[adapter.row];
    let fipe::ir::Implementation::Declared(declaration) = &row.target else {
        panic!("selected bodyless declaration")
    };
    let declaration = &tables.declarations[*declaration];
    assert_eq!(tables.namespaces[declaration.namespace], "methods");
    assert_eq!(declaration.label, "op");
    assert_eq!(row.receiver_spot, Some(2));
    let entry = row.input_contract.expect("selected row input contract");
    assert_eq!(
        joined.receiver_contexts[function.body]
            .as_ref()
            .unwrap()
            .input_contract,
        Some(entry)
    );
    assert_eq!(joined.bodies[function.body].arity, 3);
    assert_eq!(function.filled.len(), 1);
    assert!(
        matches!(&function.filled[0], Value::Record(receiver) if receiver.fields == [Value::Int(7)])
    );
    println!("NATIVE_BOUND_ROW member={} row={} body={} entry={entry} predicate={:?}", adapter.member, adapter.row, function.body, tables.entries[entry].predicate);
    (entry, function.body)
}

fn native_bound_reply(
    program: &Program,
    request: &Request,
    argument: i64,
    hole: bool,
) -> Value {
    let (label, arguments) = row_native_call(program, request);
    match label.as_str() {
        "host.after" => {
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(7)
        }
        "host.arg" => {
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(argument)
        }
        "host.later" => {
            assert!(!hole);
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(9)
        }
        "host.saved" => {
            assert!(hole);
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(7)
        }
        "methods.op" => {
            let [first, second, Value::Record(receiver)] = arguments else {
                panic!("actual raw three spots")
            };
            assert_eq!(receiver.fields, [Value::Int(7)]);
            if hole {
                let Value::Record(boolean) = first else {
                    panic!("actual Bool Record")
                };
                let tables = program.tables.borrow();
                let meaning = &tables.meanings[boolean.meaning];
                assert_eq!(tables.namespaces[meaning.namespace], "prelude");
                assert_eq!(meaning.label, "Bool");
                assert_eq!(
                    meaning.constructors[boolean.constructor].label,
                    "True"
                );
                assert!(boolean.fields.is_empty());
                assert_eq!(second, &Value::Int(argument));
            } else {
                assert_eq!(first, &Value::Int(argument));
                assert_eq!(second, &Value::Int(9));
            }
            Value::Int(7)
        }
        _ => panic!("no extra Host callback: {label}"),
    }
}

fn native_bound_expected(
    program: &Program,
    source: &str,
    run: &HostRun,
    argument: i64,
    hole: bool,
    entry: usize,
) {
    let labels: Vec<_> = run
        .1
        .iter()
        .map(|(request, _, _)| row_native_call(program, request).0)
        .collect();
    if argument == 3 {
        assert_eq!(run.0, Ok(Value::Int(7)));
        assert_eq!(run.4, 0);
        assert_eq!(
            labels,
            if hole {
                vec!["host.after", "host.arg", "host.saved", "methods.op"]
            } else {
                vec!["host.after", "host.arg", "host.later", "methods.op"]
            }
        );
    } else {
        assert_eq!(argument, 300);
        let fault = run.0.as_ref().expect_err("selected row U8 refuses 300");
        let Reason::Failed {
            entry: failed,
            value,
        } = &fault.reason
        else {
            panic!("{fault:?}")
        };
        assert_eq!(
            *failed, entry,
            "the real joined selected row owns the failure"
        );
        assert_eq!(value, "300");
        let tables = program.tables.borrow();
        assert_eq!(tables.entries[*failed].label, "U8");
        assert_eq!(tables.namespaces[fault.namespace], "main");
        assert!(
            fault.span.start < fault.span.end && fault.span.end <= source.len()
        );
        assert_eq!(run.4, 1);
        assert_eq!(
            labels,
            ["host.after", "host.arg"],
            "required refusal before later/saved/op"
        );
    }
}

fn native_bound_original_hole(argument: i64) -> (Program, HostRun, usize) {
    let (_, program) = Unit::new(
        NATIVE_BOUND_HOLE_MAIN,
        vec![
            ("host".into(), NATIVE_BOUND_HOLE_HOST.into()),
            ("methods".into(), NATIVE_BOUND_HOLE_METHODS.into()),
        ],
        vec![],
    )
    .expect("unchanged measured original Bool-hole fixture");
    let mut selected = None;
    let run = host_run_vm_observed(&program, None, |machine, request| {
        let joined = machine.program().unwrap();
        let (label, arguments) = row_native_call(joined, request);
        if label == "host.after" {
            selected = Some(native_bound_selected(machine));
        }
        if label == "host.saved" {
            let [Value::Function(function)] = arguments else {
                panic!("actual remaining Bool function")
            };
            assert_eq!(joined.bodies[function.body].arity, 1);
            assert!(function.filled.is_empty());
            let (_, original_body) =
                selected.expect("actual selected adapter before saved");
            assert!(function.captures.contains(&Value::Int(argument)));
            assert!(function.captures.iter().any(|capture| matches!(capture, Value::Function(original) if original.body == original_body)), "nonprefix wrapper captures that actual joined adapter");
            println!("NATIVE_BOUND_ORIGINAL_SAVED argument={argument} body={} filled={:?} captures={:?}", function.body, function.filled, function.captures);
            Value::Int(7)
        } else {
            native_bound_reply(joined, request, argument, true)
        }
    });
    println!(
        "NATIVE_BOUND_ORIGINAL_VM argument={argument} observation={run:?}"
    );
    let entry = selected.expect("actual selected row before arg").0;
    (program, run, entry)
}

fn native_bound_case(hole: bool, argument: i64) {
    use std::cell::Cell;
    // Keep the original Function-input VM fixture separate from the scalar native sentinel.
    let original = hole.then(|| native_bound_original_hole(argument));
    let (source, methods, host) = if hole {
        (
            NATIVE_BOUND_HOLE_MAIN.replace("host.saved g", "host.saved ()"),
            NATIVE_BOUND_HOLE_METHODS,
            NATIVE_BOUND_HOLE_HOST.replace(
                "pub saved: (Bool -> Integer) -> Integer",
                "pub saved: () -> Integer",
            ),
        )
    } else {
        (
            NATIVE_BOUND_NEXT_MAIN.to_owned(),
            NATIVE_BOUND_NEXT_METHODS,
            NATIVE_BOUND_NEXT_HOST.to_owned(),
        )
    };
    let (unit, program) = Unit::new(
        &source,
        vec![("host".into(), host), ("methods".into(), methods.into())],
        vec![],
    )
    .expect("exact next fixture or explicit scalar-sentinel derivative");
    assert!(!program.tables.borrow().has_resources);
    let selected_entry = Cell::new(None);
    let on_vm = |budget| {
        let run = host_run_vm_observed(&program, budget, |machine, request| {
            let joined = machine.program().unwrap();
            if row_native_call(joined, request).0 == "host.after" {
                let (entry, _) = native_bound_selected(machine);
                if budget.is_none() {
                    selected_entry.set(Some(entry));
                }
            }
            native_bound_reply(joined, request, argument, hole)
        });
        println!("NATIVE_BOUND_VM hole={hole} argument={argument} budget={budget:?} observation={run:?}");
        run
    };
    let whole = on_vm(None);
    let entry = selected_entry
        .get()
        .expect("actual selected row observed before argument");
    let name = format!("bodyless-bound-order-{hole}-{argument}");
    let executable = common::executable(&name, &unit, &program);
    let on_native = |budget| {
        let run = host_run_native(&executable, budget, |request| {
            native_bound_reply(&program, request, argument, hole)
        });
        println!("NATIVE_BOUND_NATIVE hole={hole} argument={argument} budget={budget:?} observation={run:?}");
        run
    };
    // Measure both backends and all existing cuts before asserting the required timing.
    current_callable_assert_parity(&name, &whole, on_vm, on_native);
    native_bound_expected(&program, &source, &whole, argument, hole, entry);
    if let Some((original_program, original_run, original_entry)) = original {
        native_bound_expected(
            &original_program,
            NATIVE_BOUND_HOLE_MAIN,
            &original_run,
            argument,
            true,
            original_entry,
        );
    }
}

#[test]
fn bodyless_row_next_argument_accepts_in_source_order() {
    native_bound_case(false, 3);
}
#[test]
fn bodyless_row_next_argument_refuses_before_later() {
    native_bound_case(false, 300);
}
#[test]
fn bodyless_row_nonprefix_bool_sentinel_accepts_in_source_order() {
    native_bound_case(true, 3);
}
#[test]
fn bodyless_row_nonprefix_bool_sentinel_refuses_before_saved() {
    native_bound_case(true, 300);
}
// An intermediate value default must reach the runtime structural default walker.
const WRITTEN_DEFAULT_HEADER: &str = "pub type Key<T>: Eq & Ord & Show & Len & Add & Zero & Default { pub n: T }\nimpl Key<T>: Eq { eq: Self -> (self: Self) -> Bool }\nimpl Key<T>: Ord { cmp: Self -> (self: Self) -> Ordering }\nimpl Key<T>: Show { show: (self: Self) -> String }\nimpl Key<T>: Len { len: (self: Self) -> Natural }\nimpl Key<T>: Add { add: Self -> (self: Self) -> Self }\nimpl Key<T>: Zero { zero: Self }\nimpl Key<T>: Default { default: Self }\npub keep: Key<Integer> -> ()\npub keep_wrong: Key<String> -> ()\npub after: ()";
const WRITTEN_DEFAULT_SOURCE: &str = "use probe\nx: probe.Key<Integer> = probe.Key 7\nprobe.keep x\nanswer = with lang \"audit\" { x.default.n }\nprobe.after\nanswer";

fn assert_generic_written_default_producer(program: &Program) {
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("probe", WRITTEN_DEFAULT_HEADER)
        .unwrap();
    let module =
        fipe::parse::parse(fipe::lex::lex(WRITTEN_DEFAULT_SOURCE)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let start = WRITTEN_DEFAULT_SOURCE.find("x.default.n").unwrap();
    let site = (start, start + "x.default.n".len());
    let accesses = &typed.accesses[&site];
    assert!(
        matches!(
            accesses.as_slice(),
            [
                fipe::typed::Access {
                    selected: fipe::typed::Selected::Derived {
                        kind: fipe::ir::Derived::Default,
                        ty: fipe::types::Type::Named { .. }
                    },
                    ..
                },
                fipe::typed::Access {
                    selected: fipe::typed::Selected::Field { index: 0, .. },
                    ..
                }
            ]
        ),
        "actual intermediate default then n field"
    );
    assert!(
        !typed.derived_defaults.contains_key(&site),
        "intermediate default is not replaced by a prepared Host call"
    );
    let tables = program.tables.borrow();
    let actual = assert_written_default_rows(&tables);
    assert_written_default_dependency(program, &tables, &actual, site);
}

fn assert_written_default_rows(tables: &fipe::ir::Tables) -> fipe::ir::Type {
    let prelude = fipe::value::Prelude::new(tables);
    let key = tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == "Key")
        .unwrap();
    let actual = fipe::ir::Type::Meaning {
        index: key,
        arguments: vec![fipe::ir::Type::Meaning {
            index: prelude.integer.unwrap(),
            arguments: Vec::new(),
        }],
    };
    let rows: Vec<_> = tables.dispatch.iter()
        .filter(|member| member.label == "default")
        .flat_map(|member| &member.rows)
        .filter(|row| matches!(&row.receiver,
            fipe::ir::Type::Meaning { index, .. } if *index == key))
        .filter(|row| matches!(row.target, fipe::ir::Implementation::Declared(index)
            if tables.declarations[index].label == "default"
            && tables.namespaces[tables.declarations[index].namespace] == "probe"))
        .collect();
    let [generic, instantiated] = rows.as_slice() else {
        panic!("exact generic and instantiated Key-owned default rows")
    };
    let fipe::ir::Implementation::Declared(target) = generic.target else {
        unreachable!()
    };
    assert_eq!(
        instantiated.target, generic.target,
        "both checked receiver shapes select the same supplied declaration"
    );
    assert_eq!(generic.receiver_spot, None);
    assert_eq!(instantiated.receiver_spot, None);
    assert_eq!(
        generic.receiver, tables.declarations[target].ty,
        "generic row retains the actual declaration's full binder"
    );
    assert_eq!(
        instantiated.receiver, actual,
        "second row is the actual script-issued Key<Integer> shape"
    );
    let fipe::ir::Type::Meaning { arguments, .. } = &generic.receiver else {
        unreachable!()
    };
    let [fipe::ir::Type::Parameter(parameter)] = arguments.as_slice() else {
        panic!("the generic Key row retains exactly its own binder")
    };
    let integer = fipe::ir::Type::Meaning {
        index: prelude.integer.unwrap(),
        arguments: Vec::new(),
    };
    assert_eq!(
        generic.receiver.substituted(&[*parameter], &[integer]),
        actual,
        "generic first-match binding and instantiated row agree exactly"
    );
    actual
}

fn assert_written_default_dependency(
    program: &Program,
    tables: &fipe::ir::Tables,
    actual: &fipe::ir::Type,
    site: (usize, usize),
) {
    let start = site.0;
    assert!(
        tables.nodes.iter().any(|node| {
            if node.span.start != start || node.span.end != site.1 {
                return false;
            }
            let fipe::ir::Kind::Field { value, index: 0 } = node.kind else {
                return false;
            };
            let call = &tables.nodes[value];
            if call.span != node.span || call.ty != *actual {
                return false;
            }
            let fipe::ir::Kind::Call {
                callee,
                slots,
                supplies,
                defaults,
                open,
            } = &call.kind
            else {
                return false;
            };
            let derived = &tables.nodes[*callee];
            slots.is_empty()
                && supplies.is_empty()
                && defaults.is_empty()
                && open.is_empty()
                && derived.span == node.span
                && derived.ty == *actual
                && matches!(&derived.kind, fipe::ir::Kind::Derived {
                kind: fipe::ir::Derived::Default, ty } if ty == actual)
        }),
        "actual field reads its exact zero-argument derived-default call"
    );
    assert!(program.bodies.iter().any(|body| body.blocks.iter().any(|block| {
        let fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Derived {
            kind: fipe::ir::Derived::Default, ty }, dest, .. } = &block.terminator
        else { return false };
        ty == actual && body.blocks.iter().flat_map(|next| &next.statements).any(|statement|
            matches!(statement, fipe::mir::Statement::Assign(_, fipe::mir::Rvalue::Field(
                fipe::mir::Operand::Local(value), 0)) if value == dest))
    })), "MIR n field reads the same derived-default destination");
}

#[test]
fn intermediate_value_default_intercepts_the_written_host_default() {
    let (unit, program) = Unit::new(
        WRITTEN_DEFAULT_SOURCE,
        vec![("probe".into(), WRITTEN_DEFAULT_HEADER.into())],
        vec![],
    )
    .expect("intermediate value default is admitted");
    assert_generic_written_default_producer(&program);
    let tables = program.tables.borrow();
    let on_vm = |budget| {
        let mut saved = (None, None);
        host_run_vm(&program, budget, |request| {
            supplied_host_reply(&tables, request, &mut saved, false)
        })
    };
    let whole = on_vm(None);
    let labels: Vec<_> = whole
        .1
        .iter()
        .map(|(request, _, _)| {
            let Request::Call { declaration, .. } = request else {
                panic!("call")
            };
            tables.declarations[*declaration].label.as_str()
        })
        .collect();
    assert_eq!(labels, ["keep", "default", "after"]);
    assert_eq!(whole.0.as_ref().expect("default n result"), &Value::Int(7));
    let executable =
        common::executable("intermediate-written-default", &unit, &program);
    let on_native = |budget| {
        let mut saved = (None, None);
        host_run_native(&executable, budget, |request| {
            supplied_host_reply(&tables, request, &mut saved, false)
        })
    };
    assert_eq!(on_native(None), whole);
    for limit in [whole.3 / 3, whole.3 / 2, whole.3.saturating_sub(1)] {
        let vm = on_vm(Some(limit));
        assert!(matches!(&vm.0, Err(fault) if fault.reason == Reason::Budget));
        assert_eq!(on_native(Some(limit)), vm);
    }
}

const REPLY_WRITER: &str = r"
import os, struct, sys
mode = sys.argv[1]
def send(payload):
    os.write(1, struct.pack('<Q', len(payload)) + payload)
if mode == 'healthy':
    send(struct.pack('<QQq', 0, 1, 7))
    send(struct.pack('<QQq', 0, 1, 8))
elif mode == 'host-error':
    send(struct.pack('<QQ', 1, 6) + b'denied')
elif mode == 'wrong-type':
    send(struct.pack('<QQQ', 0, 5, 3) + b'bad')
elif mode == 'short-header':
    os.write(1, b'\0' * 4)
elif mode == 'short-body':
    os.write(1, struct.pack('<QQ', 24, 0))
elif mode == 'tag':
    send(struct.pack('<Q', 99))
elif mode == 'utf8':
    send(struct.pack('<QQ', 1, 1) + b'\xff')
else:
    length = (64 * 1024 * 1024 + 1) if mode == 'over' else (2**64 - 1)
    try:
        os.write(1, struct.pack('<Q', length))
        # Deliberately uncooperative peer: bounded actual writes, no huge object.
        for _ in range(1024):
            os.write(1, b'x' * 1024)
    except BrokenPipeError:
        sys.exit(0)
    sys.exit(3)  # No observed closure: never call this a passing shutdown.
";

fn wait_pipe_child(
    child: &mut PipeChild,
    context: &str,
) -> std::process::ExitStatus {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(child.wait().unwrap(), status);
            return status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{context}: process exceeded 10 seconds"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn reply_boundary_frames(bytes: Vec<u8>) -> Vec<Frame> {
    let mut input = std::io::Cursor::new(bytes);
    let mut frames = Vec::new();
    while input.position() < u64::try_from(input.get_ref().len()).unwrap() {
        let mut length = [0; 8];
        input.read_exact(&mut length).unwrap();
        let length = usize::try_from(u64::from_le_bytes(length)).unwrap();
        assert!(length <= 65536, "this scalar fixture emits small frames");
        let mut payload = vec![0; length];
        input.read_exact(&mut payload).unwrap();
        frames.push(Frame::decode(&payload).unwrap());
    }
    frames
}

fn reply_boundary_run(
    executable: &Path,
    mode: &str,
    writer_source: &str,
) -> (Vec<Frame>, i32) {
    let directory = common::scratch(&format!("reply-boundary-{mode}"));
    let writer_error = directory.join("writer.stderr");
    let runtime_error = directory.join("runtime.stderr");
    let mut writer = PipeChild::new(
        Command::new("python3")
            .args(["-c", writer_source, mode])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&writer_error).unwrap())
            .spawn()
            .unwrap(),
    );
    let mut runtime = PipeChild::new(
        Command::new("sh")
            .args(["-c", "exec \"$0\" 3>&1 4<&0 1>&2 0</dev/null"])
            .arg(executable)
            .env_remove("FIPE_BUDGET")
            .stdin(Stdio::from(writer.stdout.take().unwrap()))
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&runtime_error).unwrap())
            .spawn()
            .unwrap(),
    );
    let frames = runtime.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        frames.take(65537).read_to_end(&mut bytes).unwrap();
        bytes
    });
    let runtime_status = wait_pipe_child(&mut runtime, mode);
    let writer_status = wait_pipe_child(&mut writer, mode);
    let bytes = reader.join().unwrap();
    assert!(bytes.len() <= 65536);
    std::fs::write(directory.join("frames.bin"), &bytes).unwrap();
    assert!(std::fs::metadata(&writer_error).unwrap().len() <= 65536);
    assert!(std::fs::metadata(&runtime_error).unwrap().len() <= 65536);
    assert_eq!(std::fs::read(&writer_error).unwrap(), b"");
    assert_eq!(
        writer_status.code(),
        Some(0),
        "{mode}: writer must observe closure, not timeout"
    );
    let code = runtime_status
        .code()
        .expect("normal native exit, not abort");
    let error = std::fs::read_to_string(&runtime_error).unwrap();
    if code == 2 {
        assert_eq!(error, "fipe: the host pipe failed; transport closed\n");
    } else {
        assert!(error.is_empty(), "{mode}: {error}");
    }
    println!(
        "native reply {mode}: runtime={code} writer=0 bytes={}",
        bytes.len()
    );
    (reply_boundary_frames(bytes), code)
}

fn assert_reply_boundary_case(
    program: &Program,
    frames: &[Frame],
    mode: &str,
    code: i32,
) {
    let mut labels = Vec::new();
    let mut done = None;
    for frame in frames {
        match frame {
            Frame::Call {
                request:
                    Request::Call {
                        declaration,
                        arguments,
                        ..
                    },
                ..
            } => {
                let tables = program.tables.borrow();
                let declaration = &tables.declarations[*declaration];
                assert_eq!(tables.namespaces[declaration.namespace], "probe");
                labels.push(declaration.label.clone());
                assert_eq!(
                    arguments,
                    if declaration.label == "value" {
                        &[Value::Unit][..]
                    } else {
                        &[Value::Int(7)][..]
                    }
                );
            }
            Frame::Done { result, .. } => {
                assert!(done.is_none());
                done = Some(result);
            }
            other @ Frame::Call { .. } => {
                panic!("unexpected scalar protocol: {other:?}");
            }
        }
    }
    if mode == "healthy" {
        assert_eq!(labels, ["value", "after"]);
        assert_eq!(code, 0);
        assert_eq!(done, Some(&Ok(Value::Int(8))));
    } else {
        assert_eq!(labels, ["value"], "no later effect after refusal");
        if matches!(mode, "host-error" | "wrong-type") {
            assert_eq!(code, 1);
            let Some(Err(fault)) = done else {
                panic!("ordinary source Fault must send Done");
            };
            assert_eq!(
                fault.reason,
                Reason::Host(
                    if mode == "host-error" {
                        "denied"
                    } else {
                        "the reply is not an Integer"
                    }
                    .into()
                )
            );
            let source = "use probe\nx = probe.value ()\nprobe.after x";
            let start = source.find("probe.value ()").unwrap();
            assert_eq!(
                fault.span,
                fipe::token::Span {
                    start,
                    end: start + "probe.value ()".len()
                }
            );
            let tables = program.tables.borrow();
            let namespace =
                tables.nodes[program.bodies[program.main].node].namespace;
            assert_eq!(tables.namespaces[namespace], "main");
            assert_eq!(fault.namespace, namespace);
        } else {
            assert_eq!(code, 2);
            assert!(done.is_none(), "poisoned transport must not attempt Done");
        }
    }
}

#[test]
fn native_reply_failures_terminate_without_done_or_later_effects() {
    let (unit, program) = Unit::new(
        "use probe\nx = probe.value ()\nprobe.after x",
        vec![(
            "probe".into(),
            "pub value: () -> Integer\npub after: Integer -> Integer".into(),
        )],
        vec![],
    )
    .unwrap();
    let executable = common::executable("reply_boundaries", &unit, &program);
    for mode in [
        "healthy",
        "host-error",
        "wrong-type",
        "short-header",
        "short-body",
        "tag",
        "utf8",
        "over",
        "maximum",
    ] {
        let (frames, code) =
            reply_boundary_run(&executable, mode, REPLY_WRITER);
        assert_reply_boundary_case(&program, &frames, mode, code);
    }
}

#[test]
fn reply_sender_refuses_oversize_before_header_and_reaps_waiting_child() {
    struct CountedWriter {
        pipe: std::process::ChildStdin,
        attempted: usize,
    }
    impl Write for CountedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.attempted += bytes.len();
            self.pipe.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.pipe.flush()
        }
    }
    let mut child = PipeChild::new(
        Command::new("sh")
            .args(["-c", "cat >/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut writer = CountedWriter {
        pipe: child.stdin.take().unwrap(),
        attempted: 0,
    };
    let value = Value::Text(std::rc::Rc::new(
        "x".repeat(usize::try_from(codec::MAX_REPLY_BYTES).unwrap() - 23),
    ));
    let error = pipe_reply(&mut writer, &Ok(value)).unwrap_err();
    assert_eq!(
        error,
        codec::Error::TooLong(codec::MAX_REPLY_BYTES + 1).to_string()
    );
    assert_eq!(writer.attempted, 0, "not even a reply header was written");
    drop(writer);
    assert_eq!(
        wait_pipe_child(&mut child, "pre-header sender refusal").code(),
        Some(0)
    );
}

const FLOAT_REPLY_WRITER: &str = r"
import os, struct, sys
width, case, bits = sys.argv[1].split(':')
tag = 3 if width == 'F32' else 4
def send(payload):
    os.write(1, struct.pack('<Q', len(payload)) + payload)
send(struct.pack('<QQQ', 0, tag, int(bits)))
if case in ('finite', 'subnormal'):
    send(struct.pack('<QQq', 0, 1, 8))
";

fn assert_float_reply_width(program: &Program, width: &str) {
    let tables = program.tables.borrow();
    let declaration = |label| {
        tables
            .declarations
            .iter()
            .find(|declaration| {
                declaration.label == label
                    && tables.namespaces[declaration.namespace] == "probe"
            })
            .unwrap()
    };
    let fipe::ir::Type::Function { parameter, result } =
        &declaration("value").ty
    else {
        panic!("the actual value declaration is a function");
    };
    assert_eq!(**parameter, fipe::ir::Type::Unit);
    let fipe::ir::Type::Meaning { index, arguments } = &**result else {
        panic!("the checked float result is nominal");
    };
    assert!(arguments.is_empty());
    assert_eq!(tables.meanings[*index].label, width);
    let fipe::ir::Type::Function { parameter, .. } = &declaration("after").ty
    else {
        panic!("the actual later declaration is a function");
    };
    assert_eq!(parameter, result);
}

fn assert_float_reply_case(
    program: &Program,
    frames: &[Frame],
    width: &str,
    bits: u64,
    healthy: bool,
    code: i32,
) {
    let mut labels = Vec::new();
    let mut done = None;
    for frame in frames {
        match frame {
            Frame::Call {
                request:
                    Request::Call {
                        declaration,
                        arguments,
                        ..
                    },
                ..
            } => {
                let tables = program.tables.borrow();
                let declaration = &tables.declarations[*declaration];
                assert_eq!(tables.namespaces[declaration.namespace], "probe");
                labels.push(declaration.label.clone());
                if declaration.label == "value" {
                    assert_eq!(arguments, &[Value::Unit]);
                } else {
                    assert_eq!(declaration.label, "after");
                    match (width, arguments.as_slice()) {
                        ("F32", [Value::F32(value)]) => {
                            assert_eq!(u64::from(value.to_bits()), bits);
                        }
                        ("F64", [Value::F64(value)]) => {
                            assert_eq!(value.to_bits(), bits);
                        }
                        _ => panic!("actual later argument keeps its width"),
                    }
                }
            }
            Frame::Done { result, .. } => {
                assert!(done.is_none());
                done = Some(result);
            }
            other @ Frame::Call { .. } => {
                panic!("unexpected scalar protocol: {other:?}");
            }
        }
    }
    if healthy {
        assert_eq!(labels, ["value", "after"]);
        assert_eq!(code, 0);
        assert_eq!(done, Some(&Ok(Value::Int(8))));
    } else {
        assert_eq!(labels, ["value"], "no effect after a malformed float");
        assert_eq!(code, 2);
        assert!(done.is_none(), "poisoned transport must not attempt Done");
    }
}

#[test]
fn nonfinite_float_replies_refuse_before_later_effects() {
    for (width, cases) in [
        (
            "F32",
            [
                ("nan", 0x7fc0_0000_u64, false),
                ("positive-infinity", 0x7f80_0000, false),
                ("negative-infinity", 0xff80_0000, false),
                ("finite", 0x40e0_0000, true),
                ("subnormal", 1, true),
            ],
        ),
        (
            "F64",
            [
                ("nan", 0x7ff8_0000_0000_0000, false),
                ("positive-infinity", 0x7ff0_0000_0000_0000, false),
                ("negative-infinity", 0xfff0_0000_0000_0000, false),
                ("finite", 0x401c_0000_0000_0000, true),
                ("subnormal", 1, true),
            ],
        ),
    ] {
        let header =
            format!("pub value: () -> {width}\npub after: {width} -> Integer");
        let (unit, program) = Unit::new(
            "use probe\nx = probe.value ()\nprobe.after x",
            vec![("probe".into(), header)],
            vec![],
        )
        .unwrap();
        assert_float_reply_width(&program, width);
        let executable = common::executable("float_reply", &unit, &program);
        for (case, bits, healthy) in cases {
            let mode = format!("{width}:{case}:{bits}");
            let (frames, code) =
                reply_boundary_run(&executable, &mode, FLOAT_REPLY_WRITER);
            assert_float_reply_case(
                &program, &frames, width, bits, healthy, code,
            );
        }
    }
}

const RESOURCE_CONTRACT_MAIN: &str = "use api\nb = api.acquire ()\nalias = b\nmake: api.Buffer<()> -> (() -> api.Info) = x => (_ => api.info x)\nread = make alias\ni = read ()\n(i.label, i.length)";

struct ResourceContractOwner {
    domain: fipe::value::resource::ResourceDomain,
    witness: fipe::value::resource::ResourceType,
    key: std::rc::Rc<[u8]>,
    payload: Option<String>,
    live: bool,
    acquisitions: usize,
    accesses: usize,
}
impl ResourceContractOwner {
    fn acquire(witness: fipe::value::resource::ResourceType) -> (Self, Value) {
        let owner = Self {
            domain: fipe::value::resource::ResourceDomain::new(),
            witness,
            key: std::rc::Rc::from(&b"one-never-reused-entry"[..]),
            payload: Some(String::from("private buffer")),
            live: true,
            acquisitions: 1,
            accesses: 0,
        };
        let value = owner.domain.issue(&owner.witness, owner.key.clone());
        (owner, value)
    }
    fn info(
        &mut self,
        argument: &Value,
        program: &Program,
    ) -> Result<Value, String> {
        let Value::Resource(reference) = argument else {
            panic!("checked resource argument");
        };
        if reference.domain() != &self.domain {
            return Err(String::from("foreign buffer owner"));
        }
        if reference.resource_type() != &self.witness
            || reference.key() != self.key.as_ref()
        {
            return Err(String::from("wrong buffer entry"));
        }
        if !self.live {
            return Err(String::from("revoked buffer entry"));
        }
        let payload = self.payload.as_ref().expect("live owner payload");
        self.accesses += 1;
        let tables = program.tables.borrow();
        let found: Vec<_> = tables
            .meanings
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                tables.namespaces[m.namespace] == "api" && m.label == "Info"
            })
            .collect();
        let [(meaning, shape)] = found.as_slice() else {
            panic!("one checked api.Info meaning");
        };
        assert!(!shape.resource);
        assert_eq!(shape.constructors.len(), 1);
        let fields = &shape.constructors[0].fields;
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].label, "label");
        assert_eq!(fields[1].label, "length");
        for (field, label) in fields.iter().zip(["String", "Integer"]) {
            let fipe::ir::Type::Meaning { index, arguments } = &field.ty else {
                panic!("concrete checked Info field");
            };
            assert!(arguments.is_empty());
            assert_eq!(tables.meanings[*index].label, label);
            assert_eq!(
                tables.namespaces[tables.meanings[*index].namespace],
                "prelude"
            );
        }
        Ok(Value::Record(std::rc::Rc::new(fipe::value::Record {
            meaning: *meaning,
            constructor: 0,
            fields: vec![
                Value::text(payload),
                Value::Int(i64::try_from(payload.len()).unwrap()),
            ],
        })))
    }
    fn revoke(&mut self) {
        assert!(self.live);
        self.live = false;
        assert!(self.payload.take().is_some());
    }
}

fn resource_contract_line(
    session: &mut fipe::session::Session,
    source: &str,
) -> (fipe::session::Report, Program) {
    let Outcome::Done(report) = session.line(source) else {
        panic!("fixture has no file/provider requests");
    };
    assert!(
        report.diagnostics.is_empty(),
        "{source}: {:?}",
        report.diagnostics
    );
    assert!(report.accepted.is_some());
    let program = fipe::compile::compile(report.unit.as_ref().unwrap());
    (report, program)
}

fn resource_contract_request(
    outcome: Outcome<Result<Value, Fault>>,
    machine: &Machine,
    label: &str,
) -> Vec<Value> {
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = outcome
    else {
        panic!("expected {label} request: {outcome:?}");
    };
    let tables = machine.program().unwrap().tables.borrow();
    let declared = &tables.declarations[declaration];
    assert_eq!(tables.namespaces[declared.namespace], "api");
    assert_eq!(declared.label, label);
    arguments
}

fn resource_contract_same_alias(left: &Value, right: &Value) {
    let (Value::Resource(left), Value::Resource(right)) = (left, right) else {
        panic!("actual resource aliases");
    };
    assert!(std::rc::Rc::ptr_eq(left, right));
    assert_eq!(left.domain(), right.domain());
    assert_eq!(left.resource_type(), right.resource_type());
    assert_eq!(left.key(), right.key());
}

fn resource_contract_expressions(
    module: &fipe::ast::Module,
) -> Vec<&fipe::ast::Expr> {
    use fipe::ast::{ArgumentKind, Entry, ExprKind};
    let mut pending = Vec::new();
    for entry in &module.entries {
        if let Entry::Line(expression) = entry {
            pending.push(*expression);
        }
    }
    let mut expressions = Vec::new();
    while let Some(id) = pending.pop() {
        let expression = module.expr(id).expect("actual fixture expression");
        expressions.push(expression);
        match &expression.kind {
            ExprKind::Bind { value, .. } | ExprKind::Field { value, .. } => {
                pending.push(*value);
            }
            ExprKind::Call {
                callee, arguments, ..
            } => {
                pending.push(*callee);
                for argument in arguments {
                    match &argument.kind {
                        ArgumentKind::Positional(value) => pending.push(*value),
                        ArgumentKind::Flag { value, .. } => {
                            pending.extend(*value);
                        }
                    }
                }
            }
            ExprKind::Group(parts) | ExprKind::Tuple(parts) => {
                pending.extend(parts.iter().copied());
            }
            ExprKind::Arms(arms) => {
                for arm in arms {
                    pending.extend(arm.guard);
                    pending.push(arm.body);
                }
            }
            ExprKind::Name(_)
            | ExprKind::Literal(_)
            | ExprKind::Instance { .. } => {}
            other => {
                panic!("unexpected connected fixture expression: {other:?}")
            }
        }
    }
    expressions
}

fn resource_contract_site(
    report: &fipe::session::Report,
    program: &Program,
    label: &str,
) -> (fipe::token::Span, usize) {
    use fipe::ast::ExprKind;
    let module = report.module.as_ref().unwrap();
    let mut sites = Vec::new();
    for expression in resource_contract_expressions(module) {
        let ExprKind::Call { callee, .. } = &expression.kind else {
            continue;
        };
        let ExprKind::Name(name) = &module.expr(*callee).unwrap().kind else {
            continue;
        };
        if name.segments.as_slice() == ["api", label] {
            sites.push(expression.span);
        }
    }
    let [span] = sites.as_slice() else {
        panic!("one actual api.{label} AST call");
    };
    let tables = program.tables.borrow();
    let nodes: Vec<_> = tables
        .nodes
        .iter()
        .filter(|node| {
            if node.span != *span {
                return false;
            }
            let fipe::ir::Kind::Call { callee, .. } = node.kind else {
                return false;
            };
            let fipe::ir::Kind::Declared(index) = tables.nodes[callee].kind
            else {
                return false;
            };
            let declared = &tables.declarations[index];
            tables.namespaces[declared.namespace] == "api"
                && declared.label == label
        })
        .collect();
    let [node] = nodes.as_slice() else {
        panic!("one actual checked api.{label} call");
    };
    (*span, node.namespace)
}

fn resource_contract_fault(
    outcome: Outcome<Result<Value, Fault>>,
    site: (fipe::token::Span, usize),
    message: &str,
) {
    let Outcome::Done(Err(fault)) = outcome else {
        panic!("located refusal: {outcome:?}");
    };
    assert_eq!((fault.span, fault.namespace), site);
    assert_eq!(fault.reason, Reason::Host(message.to_owned()));
}

fn resource_contract_withdraw(
    session: &mut fipe::session::Session,
    machine: &mut Machine,
    report: &fipe::session::Report,
) {
    let undo = session.withdraw(report.accepted.unwrap()).unwrap();
    machine.withdraw(&undo);
}

fn resource_contract_wrong_replies(
    session: &mut fipe::session::Session,
    machine: &mut Machine,
    owner: &ResourceContractOwner,
    original: &Value,
) {
    for (label, message) in [
        ("wrong_nominal", "the reply is not an Other"),
        ("wrong_argument", "the reply is not a Buffer"),
    ] {
        let source = format!("_ = api.{label} ()\napi.after ()");
        let (report, program) = resource_contract_line(session, &source);
        let outcome = machine.run(&program);
        assert_eq!(
            resource_contract_request(outcome, machine, label),
            [Value::Unit]
        );
        let site =
            resource_contract_site(&report, machine.program().unwrap(), label);
        assert_ne!(machine.pending_resource_type().unwrap(), owner.witness);
        resource_contract_fault(
            machine.reply(&program, Reply::Value(Ok(original.clone()))),
            site,
            message,
        );
        resource_contract_withdraw(session, machine, &report);
        assert_eq!(owner.acquisitions, 1);
        assert_eq!(owner.accesses, 1);
    }
}

fn resource_contract_foreign(
    session: &mut fipe::session::Session,
    machine: &mut Machine,
    owner: &mut ResourceContractOwner,
    original: &Value,
) {
    let (report, program) =
        resource_contract_line(session, "api.info (api.echo alias)");
    let outcome = machine.run(&program);
    let arguments = resource_contract_request(outcome, machine, "echo");
    let [argument] = arguments.as_slice() else {
        panic!("echo receiver");
    };
    resource_contract_same_alias(argument, original);
    let site =
        resource_contract_site(&report, machine.program().unwrap(), "info");
    let pending = machine.pending_resource_type().unwrap();
    assert_eq!(pending, owner.witness);
    let foreign = fipe::value::resource::ResourceDomain::new()
        .issue(&pending, owner.key.clone());
    let outcome = machine.reply(&program, Reply::Value(Ok(foreign.clone())));
    let arguments = resource_contract_request(outcome, machine, "info");
    let [argument] = arguments.as_slice() else {
        panic!("foreign receiver");
    };
    resource_contract_same_alias(argument, &foreign);
    assert_eq!(
        owner.info(argument, machine.program().unwrap()),
        Err(String::from("foreign buffer owner"))
    );
    resource_contract_fault(
        machine.reply(
            &program,
            Reply::Value(Err(String::from("foreign buffer owner"))),
        ),
        site,
        "foreign buffer owner",
    );
    resource_contract_withdraw(session, machine, &report);
    assert_eq!(owner.accesses, 1);

    let (_, program) = resource_contract_line(session, "read ()");
    let outcome = machine.run(&program);
    let arguments = resource_contract_request(outcome, machine, "info");
    let [argument] = arguments.as_slice() else {
        panic!("healthy retained closure");
    };
    resource_contract_same_alias(argument, original);
    let reply = owner.info(argument, machine.program().unwrap());
    assert!(matches!(
        machine.reply(&program, Reply::Value(reply)),
        Outcome::Done(Ok(Value::Record(_)))
    ));
    assert_eq!(owner.accesses, 2);
}

fn resource_contract_revoked(
    session: &mut fipe::session::Session,
    machine: &mut Machine,
    owner: &mut ResourceContractOwner,
    original: &Value,
    info_site: (fipe::token::Span, usize),
) {
    owner.revoke();
    let (report, program) = resource_contract_line(session, "read ()");
    let outcome = machine.run(&program);
    let arguments = resource_contract_request(outcome, machine, "info");
    let [argument] = arguments.as_slice() else {
        panic!("revoked retained closure");
    };
    resource_contract_same_alias(argument, original);
    let reply = owner.info(argument, machine.program().unwrap());
    assert_eq!(reply, Err(String::from("revoked buffer entry")));
    resource_contract_fault(
        machine.reply(&program, Reply::Value(reply)),
        info_site,
        "revoked buffer entry",
    );
    resource_contract_withdraw(session, machine, &report);
    assert_eq!(owner.acquisitions, 1);
    assert_eq!(owner.accesses, 2);
    assert!(owner.payload.is_none());
}

fn resource_contract_persistence(
    declarations: &fipe::decl::Declarations,
    source: &str,
    program: &Program,
    original: &Value,
    saved: &Value,
) {
    let denied = fipe::parse::parse(fipe::lex::lex(
        "use api\nx: api.Buffer<()> = b\"x\"",
    ))
    .unwrap();
    assert!(fipe::check::check(
        &denied,
        declarations,
        &[],
        fipe::check::Options::default(),
    )
    .is_err());
    let ordinary =
        fipe::parse::parse(fipe::lex::lex("use api\np: api.Packet = b\"x\""))
            .unwrap();
    let unit = fipe::check::check(
        &ordinary,
        declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    assert!(unit.tables().nodes.iter().any(|node| matches!(
        node.kind,
        fipe::ir::Kind::Cast {
            row: fipe::ir::CastRow::Codec,
            ..
        }
    )));
    resource_value_boundary_assert_roots(original);
    // Done's displayed-function path must still scan the real capture.
    resource_value_boundary_assert_roots(saved);
    let unit = Unit {
        sources: vec![
            (String::from("main"), RESOURCE_CONTRACT_MAIN.to_owned()),
            (String::from("api"), source.to_owned()),
        ],
        files: vec![],
        fingerprint: codec::fingerprint(program),
    };
    let decoded = Unit::decode(&unit.encode()).unwrap();
    assert_eq!(decoded, unit);
    assert_eq!(decoded.source("api"), Some(source));
    let expected = "native execution does not support resource declarations";
    assert_eq!(
        Unit::new(
            RESOURCE_CONTRACT_MAIN,
            vec![(String::from("api"), source.to_owned())],
            vec![],
        )
        .unwrap_err(),
        expected
    );
    assert_eq!(unit.compile().unwrap_err(), expected);
    assert_eq!(decoded.compile().unwrap_err(), expected);
    assert_eq!(codec::ensure_resource_free(program).unwrap_err(), expected);
    assert_eq!(lower::lower(program).unwrap_err(), expected);
    assert_eq!(lower::object(program, b"").unwrap_err(), expected);
    assert_eq!(lower::listing(program).unwrap_err(), expected);
    assert_eq!(fipe_llvm::module(program, b"").unwrap_err(), expected);
    let (ordinary, control) = Unit::new(
        "use api\np: api.Packet = b\"x\"\n7",
        vec![(String::from("api"), String::from("pub type Packet"))],
        vec![],
    )
    .unwrap();
    assert!(!lower::lower(&control).unwrap().is_empty());
    assert!(Unit::decode(&ordinary.encode()).unwrap().compile().is_ok());
}

fn resource_contract_bindings(
    report: &fipe::session::Report,
    machine: &Machine,
    original: &Value,
) -> Value {
    let mut saved = None;
    let mut alias_names = Vec::new();
    for (index, value) in machine.globals() {
        let tables = machine.program().unwrap().tables.borrow();
        match tables.definitions[index].label.as_str() {
            name @ ("b" | "alias") => {
                resource_contract_same_alias(value, original);
                alias_names.push(name.to_owned());
            }
            "read" => {
                let Value::Function(closure) = value else {
                    panic!("actual read closure");
                };
                let program = machine.program().unwrap();
                let body = &program.bodies[closure.body];
                assert_eq!(body.arity, 1);
                assert_eq!(body.captures, closure.captures.len());
                assert_eq!(body.captures, 1);
                let node = &tables.nodes[body.node];
                assert!(matches!(node.kind, fipe::ir::Kind::Function { .. }));
                assert!(resource_contract_expressions(
                    report.module.as_ref().unwrap()
                )
                .iter()
                .any(|expression| expression.span == node.span
                    && matches!(
                        &expression.kind,
                        fipe::ast::ExprKind::Arms(_)
                    )));
                let aliases: Vec<_> = closure
                    .captures
                    .iter()
                    .chain(&closure.filled)
                    .filter(|value| matches!(value, Value::Resource(_)))
                    .collect();
                let [alias] = aliases.as_slice() else {
                    panic!("one real resource capture");
                };
                resource_contract_same_alias(alias, original);
                saved = Some(value.clone());
            }
            _ => {}
        }
    }
    alias_names.sort();
    assert_eq!(alias_names, ["alias", "b"]);
    saved.expect("stored original read closure")
}

#[test]
fn resource_fipi_decision_integration_smoke() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/resource-contract.fipi");
    let source = std::fs::read_to_string(path).unwrap();
    assert_eq!(
        source.as_bytes(),
        include_bytes!("fixtures/resource-contract.fipi")
    );
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("api", &source).unwrap();
    let namespace = declarations.get_module("api").unwrap();
    for (name, marked) in [("Buffer", true), ("Other", true), ("Packet", false)]
    {
        let declared =
            namespace.types.iter().find(|ty| ty.name == name).unwrap();
        assert!(
            matches!(declared.kind, fipe::decl::Kind::Type { resource, .. } if resource == marked)
        );
    }
    let mut session = fipe::session::Session::new(declarations.clone());
    let (report, program) =
        resource_contract_line(&mut session, RESOURCE_CONTRACT_MAIN);
    assert_eq!(report.offset, 0);
    let mut machine = Machine::new(Some(100_000));
    let outcome = machine.run(&program);
    assert_eq!(
        resource_contract_request(outcome, &machine, "acquire"),
        [Value::Unit]
    );
    let info_site =
        resource_contract_site(&report, machine.program().unwrap(), "info");
    let witness = machine.pending_resource_type().unwrap();
    let (mut owner, original) = ResourceContractOwner::acquire(witness.clone());
    assert_eq!(owner.witness, witness);
    let outcome = machine.reply(&program, Reply::Value(Ok(original.clone())));
    let arguments = resource_contract_request(outcome, &machine, "info");
    let [argument] = arguments.as_slice() else {
        panic!("info receiver");
    };
    resource_contract_same_alias(argument, &original);
    let reply = owner.info(argument, machine.program().unwrap());
    let encoded = codec::encode_reply(&reply).unwrap();
    assert_eq!(codec::decode_reply(&encoded).unwrap(), reply);
    let outcome = machine.reply(&program, Reply::Value(reply));
    assert_eq!(
        outcome,
        Outcome::Done(Ok(Value::Tuple(std::rc::Rc::new(vec![
            Value::text("private buffer"),
            Value::Int(14),
        ]))))
    );
    assert_eq!((owner.acquisitions, owner.accesses), (1, 1));
    let saved = resource_contract_bindings(&report, &machine, &original);
    resource_contract_persistence(
        &declarations,
        &source,
        &program,
        &original,
        &saved,
    );
    resource_contract_wrong_replies(
        &mut session,
        &mut machine,
        &owner,
        &original,
    );
    resource_contract_foreign(
        &mut session,
        &mut machine,
        &mut owner,
        &original,
    );
    resource_contract_revoked(
        &mut session,
        &mut machine,
        &mut owner,
        &original,
        info_site,
    );
}
