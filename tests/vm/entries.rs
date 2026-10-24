//! Entries at run time: a test runs when its value is bound and the
//! first failure Faults naming the place, the limit and the value.
use super::run::{budgeted, program, run, spanned};
use super::{check, lex, parse, Declarations, Machine, Outcome};
use fipe::{compile, value::Reason};

#[test]
fn a_refuted_entry_faults_naming_the_entry_and_the_value() {
    let source = "big = _ => 300\nx: U8 = big ()\nx";
    let program = program(source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "300");
    let tables = program.tables.borrow();
    let entry = &tables.entries[*entry];
    assert_eq!(entry.label, "U8");
    assert_eq!(spanned(source, &fault), "big ()");
}

#[test]
fn a_passed_entry_keeps_its_value() {
    assert_eq!(
        format!("{:?}", run("small = _ => 200\nx: U8 = small ()\nx")),
        "Ok(Int(200))"
    );
}

#[test]
fn a_proved_entry_charges_nothing() {
    let cost = |source| budgeted(&program(source), 1 << 20).1;
    assert_eq!(cost("x: U8 = 3\nx"), cost("x = 3\nx"));
}

#[test]
fn an_open_entry_charges_its_limit() {
    let cost = |source| budgeted(&program(source), 1 << 20).1;
    assert!(
        cost("small = _ => 200\nx: U8 = small ()\nx")
            > cost("small = _ => 200\nx = small ()\nx")
    );
}

#[test]
fn an_init_entry_is_refuted_when_the_module_runs() {
    let files = vec![(
        "m".into(),
        parse(lex("big = _ => 300\npub x: U8 = big ()")).unwrap(),
    )];
    let module = parse(lex("use m\n1")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    let Outcome::Done(Err(fault)) = machine.run(&program) else {
        panic!("the module's init Faults")
    };
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "300");
    let tables = program.tables.borrow();
    assert_eq!(tables.entries[*entry].phase, fipe::ir::Phase::Init);
    assert_eq!(tables.entries[*entry].label, "U8");
}

#[test]
fn an_element_entry_tests_each_element_and_names_the_first_failure() {
    // The body is read at `n: Integer`, so the elements are open.
    let source = "f: Integer -> Vec<Integer> = n => [1, n, n + 100]\n\
                  x: Vec<U8> = f 300\nx";
    let fault = budgeted(&program(source), 1 << 20).0.unwrap_err();
    assert!(
        matches!(&fault.reason, Reason::Failed { value, .. } if value == "300"),
        "{fault:?}"
    );
    assert_eq!(
        format!("{:?}", run("f: () -> Vec<Integer> = _ => [1, 2]\nx: Vec<U8> = f ()\nx | len")),
        "Ok(Int(2))"
    );
}

#[test]
fn a_type_s_test_reads_a_generic_function_of_its_own_unit() {
    let source = "pos = x => x > 0\n\
                  type P: Integer where pos self\n\
                  p: P = 3\n\
                  q = n => { r: P = n\n r }\n\
                  (p, q 4, pos 1.5)";
    assert_eq!(super::run::shown(source), "(3, 4, True)");
    let fault = run("pos = x => x > 0\n\
         type P: Integer where pos self\n\
         q = n => { r: P = n\n r }\n\
         q 0")
    .unwrap_err();
    assert!(matches!(fault.reason, Reason::Failed { .. }), "{fault:?}");
}

/// A host source run with `cli.port` answered from `ports` in order:
/// the keys asked and the value or the Fault's reason.
fn with_ports(source: &str, ports: &[i64]) -> (Vec<String>, String) {
    use super::host::{key, observe, Script};
    let program = super::host::program(source);
    let script = ports.iter().fold(Script::default(), |script, port| {
        script.reply("cli.port", Ok(super::Value::Int(*port)))
    });
    let observation = observe(&program, Some(1 << 20), script);
    let asked = observation
        .trace
        .iter()
        .map(|(request, _)| key(&program, request))
        .collect();
    let shown = match &observation.result {
        Ok(value) => super::structural::show(value, &program.tables.borrow()),
        Err(fault) => format!("fault: {}", fault.reason),
    };
    (asked, shown)
}

/// The entry a host source's run refutes with `cli.port` answered
/// `port`, after that one request: its label and the refuted value; the
/// entry's phase is init.
fn refuted_at_init(source: &str, port: i64) -> (String, String) {
    use super::host::{observe, Script};
    let program = super::host::program(source);
    let script =
        Script::default().reply("cli.port", Ok(super::Value::Int(port)));
    let observation = observe(&program, Some(1 << 20), script);
    assert_eq!(observation.trace.len(), 1);
    let fault = observation.result.unwrap_err();
    let Reason::Failed { entry, value } = fault.reason else {
        panic!("{fault:?}")
    };
    let tables = program.tables.borrow();
    assert_eq!(tables.entries[entry].phase, fipe::ir::Phase::Init);
    (tables.entries[entry].label.clone(), value)
}

#[test]
fn a_host_constant_at_a_limited_place_is_tested_at_init() {
    let source = "use cli\n\
                  type Port: Integer where 0 < self, self <= 65535\n\
                  port: Port = cli.port\n\
                  port";
    assert_eq!(
        with_ports(source, &[8080]),
        (vec!["cli.port".into()], "8080".into())
    );
    assert_eq!(
        refuted_at_init(source, 70000),
        ("Port".into(), "70000".into())
    );
}

#[test]
fn an_init_test_at_a_module_binding_is_tested_when_it_is_bound() {
    let source = "use cli\n\
                  type Start: Integer where Init (self > 0)\n\
                  s: Start = cli.port\n\
                  s";
    assert_eq!(
        with_ports(source, &[5]),
        (vec!["cli.port".into()], "5".into())
    );
    assert_eq!(refuted_at_init(source, 0), ("Start".into(), "0".into()));
}

#[test]
fn a_field_default_reads_the_host_once_at_a_binding_and_per_call_in_an_arm() {
    let declared = "use cli\ntype Rc { n: U8 = cli.port }\n";
    let none: Vec<String> = Vec::new();
    assert_eq!(with_ports(&format!("{declared}1"), &[]), (none, "1".into()));
    let source = format!("{declared}a = Rc\n(a.n, a.n)");
    assert_eq!(
        with_ports(&source, &[3]),
        (vec!["cli.port".into()], "(3, 3)".into())
    );
    let source = format!("{declared}g = k => Rc\n((g 1).n, (g 2).n)");
    assert_eq!(
        with_ports(&source, &[4, 5]),
        (vec!["cli.port".into(), "cli.port".into()], "(4, 5)".into())
    );
}

#[test]
fn a_unit_with_a_check_error_has_no_program_to_run() {
    // A host call precedes the error in source order; the unit is
    // refused whole, so no program exists and no request is made.
    let source = "use cli\np = cli.port\ntype R { a: Integer }\nr = R 1\nr.b";
    let module = parse(lex(source)).unwrap();
    let checked = check::check(
        &module,
        &super::host::declarations(),
        &[],
        check::Options::default(),
    );
    assert!(checked.is_err(), "the wrong field is a check error");
}

#[test]
fn an_open_function_entry_tests_each_call_of_the_value() {
    // `map`'s elements are not known at check: `f`'s parameter limit is
    // tested on each call `map` makes.
    let lines = "f: U8 -> U8 = n => n\ng = n => map f [n]\n";
    assert_eq!(
        format!("{:?}", run(&format!("{lines}g 3"))),
        format!("{:?}", run("[3]"))
    );
    let source = format!("{lines}g 300");
    let program = program(&source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "300");
    let tables = program.tables.borrow();
    assert!(tables.entries[*entry].label.starts_with("U8 ->"));
}

/// A run that Faults on 300 entering a `U8` parameter limit.
fn faults_on_300_at_u8(source: &str) {
    let program = program(source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{source}: {fault:?}")
    };
    assert_eq!(value, "300", "{source}");
    let tables = program.tables.borrow();
    let label = &tables.entries[*entry].label;
    assert!(
        label.starts_with("U8") || label.contains("-> U8"),
        "{label}"
    );
}

#[test]
fn a_function_whose_type_variable_drops_its_limits_is_still_tested() {
    let f = "f: U8 -> U8 = n => n\n";
    // What comes out of `log`, a constructor or a body is not
    // known as `f`: its limits are tested on each call of the value.
    faults_on_300_at_u8(&format!("{f}r = log f\nr 300"));
    faults_on_300_at_u8(&format!(
        "{f}m = Just f\nm | {{ Just g => g 300, Nothing => 0 }}"
    ));
    faults_on_300_at_u8(&format!("{f}mk = c => f\nk = n => (mk 1) n\nk 300"));
    // Known as `f` through a variable, an unknown argument is tested.
    faults_on_300_at_u8(&format!(
        "{f}id = x => x\nh2 = g x => g x\nk = n => h2 (id f) n\nk 300"
    ));
    faults_on_300_at_u8(&format!(
        "{f}pick = c f g => if c > 0 {{ f }} else {{ g }}\n\
         k = n => (pick 1 f f) n\nk 300"
    ));
    assert_eq!(
        format!("{:?}", run(&format!("{f}r = log f\nr 3"))),
        "Ok(Int(3))"
    );
}

#[test]
fn a_guard_past_a_body_tests_the_function_it_returns() {
    // `k`'s body takes one parameter; the open entry's second limit
    // tests the function it returns.
    let lines = "k: U8 -> U8 -> U8 = x => (y => y)\n\
                 apply2 = g a b => g a b\nq = n => apply2 k 1 n\n";
    faults_on_300_at_u8(&format!("{lines}q 300"));
    assert_eq!(format!("{:?}", run(&format!("{lines}q 3"))), "Ok(Int(3))");
}

// The original generic and concrete Sorted programs share this exact host.
const PREDICATE_WITNESS_HOST: &str =
    "pub items: () -> Vec<Integer>\npub after: () -> Integer\n";
const PREDICATE_WITNESS_GENERIC: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Sorted<a> -> Integer = xs => host.after ()\napply = f x => f x\ngo = xs => apply keep xs\ngo (host.items ())\n";
const PREDICATE_WITNESS_CONCRETE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nx: Sorted<Integer> = host.items ()\nhost.after ()\n";

fn sorted_runtime_test_identity(
    joined: &fipe::mir::Program,
    source: &str,
) -> (usize, usize) {
    let tables = joined.tables.borrow();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .expect("main source namespace");
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (test.namespace == main
                && source.get(test.span.start..test.span.end)
                    == Some("is_sorted self"))
            .then_some(index)
        })
        .collect();
    assert_eq!(tests.len(), 1, "one actual Sorted source test");
    let test = tests[0];
    (
        test,
        joined.tests[test].expect("actual lowered Sorted test body"),
    )
}

fn observe_sorted_integer_witnesses(
    machine: &Machine,
    sorted_body: usize,
    tested: &mut std::collections::BTreeSet<usize>,
) {
    use fipe::{ir, vm::Frame};
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().expect("active joined program");
        let tables = joined.tables.borrow();
        if frame.body == sorted_body {
            let body = &joined.bodies[frame.body];
            assert!(
                !body.hidden.is_empty(),
                "actual Sorted test has a generic witness"
            );
            assert_eq!(body.hidden.len(), frame.meanings.len());
            for meaning in &frame.meanings {
                let ir::Type::Meaning { index, arguments } = meaning else {
                    panic!("Sorted test received a template instead of Integer: {meaning:?}");
                };
                assert_eq!(tables.meanings[*index].label, "Integer");
                assert!(arguments.is_empty());
            }
            tested.insert(frame.body);
        }
    }
}

fn assert_sorted_runtime_failure(
    joined: &fipe::mir::Program,
    fault: &fipe::vm::Fault,
    sorted_test: usize,
) {
    use fipe::ir;
    let Reason::Failed { entry, .. } = &fault.reason else {
        panic!("{fault:?}")
    };
    let tables = joined.tables.borrow();
    let mut predicates = vec![&tables.entries[*entry].predicate];
    let mut names_sorted = false;
    while let Some(predicate) = predicates.pop() {
        match predicate {
            ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                predicates.extend(parts);
            }
            ir::Predicate::Not(inner) => predicates.push(inner),
            ir::Predicate::Function { parameter, result } => {
                predicates.push(result);
                predicates.push(parameter);
            }
            ir::Predicate::Test { index, .. } => {
                names_sorted |= *index == sorted_test;
            }
            ir::Predicate::Nonzero { .. } => {}
        }
    }
    assert!(
        names_sorted,
        "failed entry names the same semantic Sorted test"
    );
}

fn original_sorted_witness_case(source: &str, sorted: bool) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::Stop,
    };
    use std::{collections::BTreeSet, rc::Rc};
    let mut declarations = Declarations::new();
    declarations.module("host", PREDICATE_WITNESS_HOST).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let (sorted_test, sorted_body) = sorted_runtime_test_identity(
        machine.program().expect("active joined program"),
        source,
    );
    let mut requests = Vec::new();
    let mut tested = BTreeSet::new();
    let mut finished = false;
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(&machine, sorted_body, &mut tested);
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let Request::Call { declaration, .. } = request else {
                    panic!("{request:?}")
                };
                let label =
                    machine.program().unwrap().tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                requests.push(label.clone());
                let value = match label.as_str() {
                    "items" => Value::Seq(Rc::new(if sorted {
                        vec![Value::Int(1), Value::Int(2)]
                    } else {
                        vec![Value::Int(2), Value::Int(1)]
                    })),
                    "after" => {
                        assert!(
                            sorted,
                            "after must not run after a failed Sorted test"
                        );
                        Value::Int(7)
                    }
                    _ => panic!("unexpected host request {label}"),
                };
                machine.answer(Reply::Value(Ok(value)));
            }
            Stop::Done(result) => {
                assert!(
                    !tested.is_empty(),
                    "the actual joined Sorted test body must execute"
                );
                if sorted {
                    assert_eq!(result.unwrap(), Value::Int(7));
                    assert_eq!(requests, ["items", "after"]);
                } else {
                    let fault = result.unwrap_err();
                    assert_sorted_runtime_failure(
                        machine.program().expect("joined fault program"),
                        &fault,
                        sorted_test,
                    );
                    assert_eq!(requests, ["items"]);
                }
                finished = true;
                break;
            }
        }
    }
    assert!(finished, "bounded stepped Sorted execution must finish");
}

#[test]
fn generic_sorted_guard_captures_creator_witnesses() {
    original_sorted_witness_case(PREDICATE_WITNESS_GENERIC, true);
}
#[test]
fn generic_unsorted_guard_refuses_before_after() {
    original_sorted_witness_case(PREDICATE_WITNESS_GENERIC, false);
}
#[test]
fn concrete_sorted_entry_preserves_integer_witnesses() {
    original_sorted_witness_case(PREDICATE_WITNESS_CONCRETE, true);
}
#[test]
fn concrete_unsorted_entry_refuses_before_after() {
    original_sorted_witness_case(PREDICATE_WITNESS_CONCRETE, false);
}

#[test]
fn raw_host_helper_local_predicate_refuses_before_after() {
    use fipe::{
        embed::{Reply, Request},
        ir,
        value::Value,
    };
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub helper: Vec<Integer> -> Integer = xs => {\n checked: Nonempty<Integer> = xs\n 0\n}\npub after: Integer -> Integer\n";
    for empty in [true, false] {
        let input = if empty { "[]" } else { "[1]" };
        let source = format!("use host\nhost.after (host.helper {input})\n");
        let mut declarations = Declarations::new();
        declarations.module("host", HOST).unwrap();
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        let mut machine = Machine::new(Some(100_000));
        let outcome = machine.run(&program);
        if empty {
            let Outcome::Done(Err(fault)) = outcome else {
                panic!("invalid input must fail before after: {outcome:?}")
            };
            let Reason::Failed { entry, .. } = fault.reason else {
                panic!("actual Nonempty refusal: {fault:?}")
            };
            let joined = machine.program().unwrap();
            let tables = joined.tables.borrow();
            let tests: Vec<_> = tables
                .tests
                .iter()
                .enumerate()
                .filter_map(|(index, test)| {
                    (tables.namespaces[test.namespace] == "host"
                        && HOST.get(test.span.start..test.span.end)
                            == Some("self.len > 0"))
                    .then_some(index)
                })
                .collect();
            assert_eq!(tests.len(), 1);
            let mut predicates = vec![&tables.entries[entry].predicate];
            let mut same_test = false;
            while let Some(predicate) = predicates.pop() {
                match predicate {
                    ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                        predicates.extend(parts);
                    }
                    ir::Predicate::Not(inner) => predicates.push(inner),
                    ir::Predicate::Function { parameter, result } => {
                        predicates.push(result);
                        predicates.push(parameter);
                    }
                    ir::Predicate::Test { index, .. } => {
                        same_test |= *index == tests[0];
                    }
                    ir::Predicate::Nonzero { .. } => {}
                }
            }
            assert!(
                same_test,
                "failure names the actual supplied Nonempty test"
            );
        } else {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("healthy helper reaches after: {outcome:?}")
            };
            {
                let tables = machine.program().unwrap().tables.borrow();
                let declared = &tables.declarations[declaration];
                assert_eq!(tables.namespaces[declared.namespace], "host");
                assert_eq!(declared.label, "after");
            }
            assert_eq!(arguments, [Value::Int(0)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        }
    }
}
#[test]
fn raw_host_generic_helper_predicate_stays_inside_its_body() {
    use fipe::{
        embed::{Reply, Request},
        ir,
        value::Value,
    };
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub helper: Vec<a> -> Integer = xs => {\n checked: Nonempty<a> = xs\n 0\n}\npub after: Integer -> Integer\n";
    for empty in [true, false] {
        let input = if empty { "[]" } else { "[1]" };
        let source = format!("use host\nhost.after (host.helper {input})\n");
        let mut declarations = Declarations::new();
        declarations.module("host", HOST).unwrap();
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        let mut machine = Machine::new(Some(100_000));
        let outcome = machine.run(&program);
        if empty {
            let Outcome::Done(Err(fault)) = outcome else {
                panic!("invalid input must fail before after: {outcome:?}")
            };
            let Reason::Failed { entry, .. } = fault.reason else {
                panic!("actual Nonempty refusal: {fault:?}")
            };
            let joined = machine.program().unwrap();
            let tables = joined.tables.borrow();
            assert_eq!(
                tables.namespaces[fault.namespace], "host",
                "the helper body owns its refusal"
            );
            assert_eq!(
                HOST.get(fault.span.start..fault.span.end),
                Some("xs"),
                "the checked helper input is the failing expression"
            );
            let tests: Vec<_> = tables
                .tests
                .iter()
                .enumerate()
                .filter_map(|(index, test)| {
                    (tables.namespaces[test.namespace] == "host"
                        && HOST.get(test.span.start..test.span.end)
                            == Some("self.len > 0"))
                    .then_some(index)
                })
                .collect();
            assert_eq!(tests.len(), 1);
            let mut predicates = vec![&tables.entries[entry].predicate];
            let mut same_test = false;
            while let Some(predicate) = predicates.pop() {
                match predicate {
                    ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                        predicates.extend(parts);
                    }
                    ir::Predicate::Not(inner) => predicates.push(inner),
                    ir::Predicate::Function { parameter, result } => {
                        predicates.push(result);
                        predicates.push(parameter);
                    }
                    ir::Predicate::Test { index, .. } => {
                        same_test |= *index == tests[0];
                    }
                    ir::Predicate::Nonzero { .. } => {}
                }
            }
            assert!(
                same_test,
                "failure names the actual supplied Nonempty test"
            );
        } else {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("healthy helper reaches after: {outcome:?}")
            };
            {
                let tables = machine.program().unwrap().tables.borrow();
                let declared = &tables.declarations[declaration];
                assert_eq!(tables.namespaces[declared.namespace], "host");
                assert_eq!(declared.label, "after");
            }
            assert_eq!(arguments, [Value::Int(0)]);
            assert_eq!(
                machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                Outcome::Done(Ok(Value::Int(7)))
            );
        }
    }
}

#[test]
fn scalar_omitted_default_keeps_its_same_file_limit() {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let source = "use cli\ntype R { n: U8 = cli.port }\nr = R\nr.n\n";
    for port in [7, 300] {
        let program = super::host::program(source);
        let mut machine = Machine::new(Some(1 << 20));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            environment,
        }) = machine.run(&program)
        else {
            panic!("the real omitted default must request cli.port")
        };
        {
            let tables = machine.program().unwrap().tables.borrow();
            let declaration = &tables.declarations[declaration];
            assert_eq!(tables.namespaces[declaration.namespace], "cli");
            assert_eq!(declaration.label, "port");
        }
        assert!(arguments.is_empty());
        assert!(environment.is_empty());
        let Outcome::Done(result) =
            machine.reply(&program, Reply::Value(Ok(Value::Int(port))))
        else {
            panic!("the scalar default must finish without another effect")
        };
        if port == 7 {
            assert_eq!(result.unwrap(), Value::Int(7));
        } else {
            let fault = result.unwrap_err();
            let Reason::Failed { entry, .. } = fault.reason else {
                panic!("{fault:?}")
            };
            let tables = machine.program().unwrap().tables.borrow();
            assert_eq!(tables.entries[entry].label, "U8");
        }
    }
}

#[test]
fn returned_member_original_signed_limits_survive_generic_exposure() {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    const GENERIC: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\nget = m => m.op\nf = get (M 7)\nhost.after ()\nf (host.arg ())";
    const DIRECT: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\nf = (M 7).op\nhost.after ()\nf (host.arg ())";
    const PLAIN: &str = "use host\nk: Integer -> U8 -> U8 = self => (x => x)\nget = m => k m\nf = get 7\nhost.after ()\nf (host.arg ())";
    for source in [GENERIC, DIRECT, PLAIN] {
        for argument in [3, 300] {
            let mut declarations = Declarations::new();
            declarations.module("host", "pub tick: () -> Integer\npub after: () -> Integer\npub arg: () -> Integer").unwrap();
            let module = parse(lex(source)).unwrap();
            let unit = check::check(
                &module,
                &declarations,
                &[],
                check::Options::default(),
            )
            .unwrap();
            let program = compile::compile(&unit);
            let mut machine = Machine::new(Some(1 << 20));
            let mut outcome = machine.run(&program);
            let mut calls = Vec::new();
            let mut finished = false;
            for _ in 0..8 {
                match outcome {
                    Outcome::Request(Request::Call {
                        declaration,
                        arguments,
                        ..
                    }) => {
                        assert_eq!(arguments.as_slice(), [Value::Unit]);
                        let label = machine
                            .program()
                            .unwrap()
                            .tables
                            .borrow()
                            .declarations[declaration]
                            .label
                            .clone();
                        let value = match label.as_str() {
                            "after" => {
                                assert!(calls.is_empty());
                                Value::Int(99)
                            }
                            "arg" => {
                                assert_eq!(calls, ["after"]);
                                Value::Int(argument)
                            }
                            _ => panic!("unexpected {label}"),
                        };
                        calls.push(label);
                        outcome =
                            machine.reply(&program, Reply::Value(Ok(value)));
                    }
                    Outcome::Done(result) => {
                        assert_eq!(calls, ["after", "arg"]);
                        if argument == 3 {
                            assert_eq!(result.unwrap(), Value::Int(3));
                        } else {
                            let fault = result.unwrap_err();
                            assert!(
                                matches!(fault.reason, Reason::Failed { ref value, .. } if value == "300"),
                                "{fault:?}"
                            );
                        }
                        finished = true;
                        break;
                    }
                    other @ Outcome::Request(_) => {
                        panic!("unexpected {other:?}")
                    }
                }
            }
            assert!(finished, "bounded original exposure control");
        }
    }
}

#[test]
fn returned_member_predicate_only_witness_survives_dynamic_exposure() {
    const SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ntype M<T: Ord> { n: Integer }\nimpl M<a> { op: Self -> Sorted<a> -> Integer = self => (xs => host.after ()) }\nget = m => m.op\nx: M<Integer> = M 7\nf = get x\nf (host.items ())";
    original_sorted_witness_case(SOURCE, true);
    original_sorted_witness_case(SOURCE, false);
}

fn check_member_result_contract(
    source: &str,
    accepted: bool,
    supplied_name: &str,
) {
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub after: () -> Integer\npub arg: () -> Integer\n")
        .unwrap();
    let module = parse(lex(source)).unwrap();
    let checked =
        check::check(&module, &declarations, &[], check::Options::default());
    if !accepted {
        let errors = checked.unwrap_err();
        let start = source.rfind(supplied_name).unwrap();
        let expected = fipe::token::Span {
            start,
            end: start + supplied_name.len(),
        };
        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
        assert!(matches!(&errors[0], check::Error::Type(fipe::infer::Error {
            span, reason: fipe::infer::Reason::Limit { message },
        }) if *span == expected && (message == "refuted entry against Integer -> U8"
            || message == "unproved function entry against Integer -> U8")),
            "exact source receiving function must meet its result contract: {errors:?}");
        return;
    }
    let program = compile::compile(
        &checked.unwrap_or_else(|errors| panic!("{source}: {errors:?}")),
    );
    let mut machine = Machine::new(Some(1 << 20));
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("healthy contract must reach exactly one after request");
    };
    assert_eq!(arguments, vec![fipe::value::Value::Unit]);
    assert_eq!(
        machine.program().unwrap().tables.borrow().declarations[declaration]
            .label,
        "after"
    );
    let Outcome::Done(Ok(value)) = machine.reply(
        &program,
        fipe::embed::Reply::Value(Ok(fipe::value::Value::Int(7))),
    ) else {
        panic!("after is the only effect and result");
    };
    assert_eq!(value, fipe::value::Value::Int(7));
}

#[test]
fn member_callable_contracts_check_whole_and_prestructured_receiving_functions()
{
    const PREFIX: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()) }\n";
    for route in [
        "get = m => m.op\nf = get (M 7)\n",
        "invoke = m fs => m.op fs\n",
    ] {
        for (signature, body, accepted) in [
            ("Integer -> U8", "3", true),
            ("Integer -> Integer", "300", false),
            ("Integer -> Integer", "host.arg ()", false),
        ] {
            let call = if route.starts_with("get") {
                "f [provide]"
            } else {
                "invoke (M 7) [provide]"
            };
            let source = format!(
                "{PREFIX}{route}provide: {signature} = n => {body}\n{call}"
            );
            check_member_result_contract(&source, accepted, "provide");
        }
    }
}

#[test]
fn member_callable_contract_ports_follow_aliases_without_narrowing_same_var_siblings(
) {
    const PREFIX: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()) }\n";
    let alias = format!("{PREFIX}invoke = m fs => {{ copy = fs\n m.op copy }}\nprovide: Integer -> U8 = n => 3\ninvoke (M 7) [provide]");
    check_member_result_contract(&alias, true, "provide");
    let alias_bad = alias.replace(
        "provide: Integer -> U8 = n => 3",
        "provide: Integer -> Integer = n => 300",
    );
    check_member_result_contract(&alias_bad, false, "provide");
    let common = format!("{PREFIX}same = x y => if True {{ x }} else {{ y }}\ninvoke = m x y => {{ _ = same x y\n m.op x }}\ngood: Integer -> U8 = n => 3\nbad: Integer -> Integer = n => 300\n");
    check_member_result_contract(
        &format!("{common}invoke (M 7) [good] [bad]"),
        true,
        "good",
    );
    check_member_result_contract(
        &format!("{common}invoke (M 7) [bad] [good]"),
        false,
        "bad",
    );
}

#[test]
fn member_callable_contracts_keep_both_receiving_obligations_on_one_formal() {
    let prefix = "use host\ntype M { n: Integer }\nimpl M { small: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()),\n wide: Self -> Vec<Integer -> U16> -> Integer = self => (fs => host.after ()) }\n";
    for body in ["_ = m.small fs\n m.wide fs", "_ = m.wide fs\n m.small fs"] {
        for (value, accepted) in [(200, true), (300, false)] {
            let source = format!("{prefix}invoke = m fs => {{ {body} }}\nprovide: Integer -> Integer = n => {value}\ninvoke (M 7) [provide]");
            let mut declarations = Declarations::new();
            declarations
                .module("host", "pub after: () -> Integer\n")
                .unwrap();
            let module = parse(lex(&source)).unwrap();
            let checked = check::check(
                &module,
                &declarations,
                &[],
                check::Options::default(),
            );
            if accepted {
                let unit = checked
                    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
                let _ = compile::compile(&unit);
            } else {
                let errors = checked.unwrap_err();
                let start = source.rfind("provide").unwrap();
                let span = fipe::token::Span {
                    start,
                    end: start + "provide".len(),
                };
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                assert!(
                    matches!(&errors[0], check::Error::Type(fipe::infer::Error {
                    span: actual, reason: fipe::infer::Reason::Limit { message },
                }) if *actual == span && message == "refuted entry against Integer -> U8"),
                    "the U8 obligation survives either read order: {errors:?}"
                );
            }
        }
    }
}

const FULLY_APPLIED_PARTIAL_HOST: &str = "pub items: () -> Vec<Integer>\npub after: () -> Integer\npub relay: a -> a\n";
const FULLY_APPLIED_PARTIAL_SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => host.after ()\napply_one = fn x y => {\n partial = fn x\n next = host.relay partial\n next y\n}\ngo = xs => apply_one keep 7 xs\ngo (host.items ())\n";

fn fully_applied_partial_test_parameters(
    predicate: &fipe::ir::Predicate,
    test: usize,
) -> Option<Vec<usize>> {
    use fipe::ir::{Predicate, Type};
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                pending.extend(parts);
            }
            Predicate::Not(inner) => pending.push(inner),
            Predicate::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            Predicate::Test { index, meanings } if *index == test => {
                return Some(meanings.iter().map(|meaning| {
                    let Type::Parameter(parameter) = meaning else {
                        panic!("actual generic creator Test template: {meaning:?}")
                    };
                    *parameter
                }).collect());
            }
            Predicate::Nonzero { .. } | Predicate::Test { .. } => {}
        }
    }
    None
}

fn fully_applied_partial_bodies(
    program: &fipe::mir::Program,
) -> (usize, usize, usize) {
    use fipe::{ir, mir};
    let tables = program.tables.borrow();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let body = |name| {
        let definition = tables
            .definitions
            .iter()
            .position(|definition| {
                definition.namespace == main && definition.label == name
            })
            .unwrap();
        program.definitions[definition].expect("actual source body")
    };
    let apply = body("apply_one");
    let go = body("go");
    assert_eq!(program.bodies[apply].arity, 3);
    assert!(
        !program.bodies[apply]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(
                statement,
                mir::Statement::Assign(_, mir::Rvalue::Closure { .. })
            )),
        "the helper must not construct a compile-time open-call wrapper"
    );
    let mut calls = Vec::new();
    for block in &program.bodies[apply].blocks {
        let Some(node) = block.nodes.last().copied() else {
            continue;
        };
        let site = &tables.nodes[node];
        if site.namespace != main
            || FULLY_APPLIED_PARTIAL_SOURCE.get(site.span.start..site.span.end)
                != Some("fn x")
        {
            continue;
        }
        let mir::Terminator::Call {
            callee: mir::Callee::Value(_),
            args,
            dest,
            ..
        } = &block.terminator
        else {
            panic!("exact fn x must emit the actual dynamic Value call")
        };
        assert_eq!(args.len(), 1);
        assert!(
            matches!(
                program.bodies[apply].locals[*dest].ty,
                ir::Type::Function { .. }
            ),
            "later next y constrains the final destination to Function: {:?}",
            program.bodies[apply].locals[*dest].ty
        );
        let ir::Kind::Call { open, .. } = &site.kind else {
            panic!("actual closed IR call behind exact fn x")
        };
        assert!(
            open.is_empty(),
            "recorded call stays closed despite final Function result"
        );
        calls.push(node);
    }
    assert_eq!(calls.len(), 1, "one exact dynamic underarity producer");
    (apply, go, calls[0])
}

fn observe_fully_applied_partial_operand(
    machine: &Machine,
    apply: usize,
    go: usize,
    test: usize,
    partial_node: usize,
) -> Option<(usize, fipe::value::Guard)> {
    use fipe::{ir, mir, value::Value, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != apply {
        return None;
    }
    let program = machine.program().unwrap();
    let body = &program.bodies[apply];
    let block = &body.blocks[frame.block];
    if frame.statement != block.statements.len()
        || block.nodes.last() != Some(&partial_node)
    {
        return None;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(mir::Operand::Local(local)),
        args,
        ..
    } = &block.terminator
    else {
        return None;
    };
    assert_eq!(args.len(), 1);
    let Value::Function(function) = &frame.locals[*local] else {
        panic!("actual runtime function operand")
    };
    assert_eq!(program.bodies[function.body].arity, 2);
    assert!(function.filled.is_empty());
    let guard = function.guards.iter().find(|guard| {
        fully_applied_partial_test_parameters(&guard.predicate, test).is_some()
    }).unwrap_or_else(|| panic!("semantic Sorted guard must exist BEFORE runtime partial; actual guards={:?}", function.guards));
    assert_eq!(guard.spot, 1);
    assert!(!guard.witnesses.is_empty());
    let parameters =
        fully_applied_partial_test_parameters(&guard.predicate, test).unwrap();
    assert!(!parameters.is_empty());
    assert!(
        !body.hidden.is_empty(),
        "actual caller hidden={:?}, meanings={:?}",
        body.hidden,
        frame.meanings
    );
    assert_eq!(body.hidden.len(), frame.meanings.len());
    assert!(
        body.hidden.iter().all(|id| !parameters.contains(id)),
        "invoking IDs {:?} must differ from creator IDs {parameters:?}",
        body.hidden
    );
    let tables = program.tables.borrow();
    for parameter in parameters {
        assert!(
            program.bodies[go].hidden.contains(&parameter),
            "actual creator owns Test ID"
        );
        let (_, meaning) = guard
            .witnesses
            .iter()
            .find(|(id, _)| *id == parameter)
            .unwrap();
        let ir::Type::Meaning { index, arguments } = meaning.as_type() else {
            panic!("actual creator Integer witness: {meaning:?}")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert!(arguments.is_empty());
    }
    Some((function.body, guard.clone()))
}

fn assert_fully_applied_partial_result(
    program: &fipe::mir::Program,
    arguments: &[fipe::value::Value],
    before: &(usize, fipe::value::Guard),
    test: usize,
) -> fipe::value::Value {
    use fipe::value::Value;
    let [value @ Value::Function(partial)] = arguments else {
        panic!("relay receives the actual dynamic partial result")
    };
    assert_eq!(partial.body, before.0, "runtime partial retains raw body");
    assert_eq!(program.bodies[partial.body].arity, 2);
    assert_eq!(partial.filled, [Value::Int(7)]);
    let guard = partial
        .guards
        .iter()
        .find(|guard| {
            guard.entry == before.1.entry
                && guard.spot == before.1.spot
                && fully_applied_partial_test_parameters(&guard.predicate, test)
                    .is_some()
        })
        .expect("same semantic Sorted guard after underarity");
    assert!(
        std::rc::Rc::ptr_eq(&guard.witnesses, &before.1.witnesses),
        "same association allocation, not only equal types"
    );
    value.clone()
}

fn fully_applied_partial_reply(
    machine: &Machine,
    request: &fipe::embed::Request,
    sorted: bool,
    before: Option<&(usize, fipe::value::Guard)>,
    test: usize,
    requests: &mut Vec<String>,
) -> fipe::value::Value {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("actual declared host call: {request:?}")
    };
    let program = machine.program().unwrap();
    let label = program.tables.borrow().declarations[*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "items" => {
            assert_eq!(requests.as_slice(), ["items"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::seq(if sorted {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            })
        }
        "relay" => {
            assert_eq!(requests.as_slice(), ["items", "relay"]);
            assert_fully_applied_partial_result(
                program,
                arguments,
                before.expect("observe guard BEFORE partial"),
                test,
            )
        }
        "after" => {
            assert!(sorted, "unsorted refuses before after");
            assert_eq!(requests.as_slice(), ["items", "relay", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected effect {label}"),
    }
}

fn fully_applied_partial_guard_case(sorted: bool) {
    use fipe::{embed::Reply, value::Value, vm::Stop};
    let mut declarations = Declarations::new();
    declarations
        .module("host", FULLY_APPLIED_PARTIAL_HOST)
        .unwrap();
    let module = parse(lex(FULLY_APPLIED_PARTIAL_SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("public higher-order dynamic partial producer");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let joined = machine.program().unwrap();
    let (test, test_body) =
        sorted_runtime_test_identity(joined, FULLY_APPLIED_PARTIAL_SOURCE);
    let (apply, go, partial_node) = fully_applied_partial_bodies(joined);
    let mut before = None;
    let mut tested = std::collections::BTreeSet::new();
    let mut requests = Vec::new();
    let mut finished = false;
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(&machine, test_body, &mut tested);
        if matches!(&stop, Stop::Before(_)) {
            if let Some(observed) = observe_fully_applied_partial_operand(
                &machine,
                apply,
                go,
                test,
                partial_node,
            ) {
                assert!(
                    before.is_none(),
                    "one observed runtime partial transition"
                );
                before = Some(observed);
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = fully_applied_partial_reply(
                    &machine,
                    &request,
                    sorted,
                    before.as_ref(),
                    test,
                    &mut requests,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert!(
                    before.is_some(),
                    "actual guarded operand before underarity"
                );
                assert!(
                    !tested.is_empty(),
                    "actual Sorted test executes with creator Integer"
                );
                if sorted {
                    assert_eq!(result, Ok(Value::Int(7)));
                    assert_eq!(requests, ["items", "relay", "after"]);
                } else {
                    let fault = result.unwrap_err();
                    assert_sorted_runtime_failure(
                        machine.program().unwrap(),
                        &fault,
                        test,
                    );
                    assert_eq!(requests, ["items", "relay"]);
                }
                finished = true;
                break;
            }
        }
    }
    assert!(finished, "bounded actual dynamic partial proof finishes");
}

#[test]
fn fully_applied_partial_sorted_guard_shares_creator_witnesses() {
    fully_applied_partial_guard_case(true);
}

#[test]
fn fully_applied_partial_unsorted_guard_refuses_with_shared_creator_witnesses()
{
    fully_applied_partial_guard_case(false);
}

const BEYOND_RETURN_HOST: &str = "pub items: () -> Vec<Integer>\npub after: () -> Integer\npub relay: a -> a\npub made: () -> ()\n";
const BEYOND_RETURN_SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nfactory: Ord a => () -> Sorted<a> -> Integer = _ => {\n host.made ()\n result = xs => host.after ()\n result\n}\napply_return = fn x y => {\n returned = fn x\n next = host.relay returned\n next y\n}\ngo = xs => apply_return factory () xs\ngo (host.items ())\n";

fn beyond_return_bodies(program: &fipe::mir::Program) -> (usize, usize, usize) {
    use fipe::{ir, mir};
    let tables = program.tables.borrow();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let body = |name| {
        let definition = tables
            .definitions
            .iter()
            .position(|definition| {
                definition.namespace == main && definition.label == name
            })
            .unwrap();
        program.definitions[definition].expect("actual source body")
    };
    let apply = body("apply_return");
    let go = body("go");
    assert_eq!(program.bodies[apply].arity, 3);
    assert!(
        !program.bodies[apply]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(
                statement,
                mir::Statement::Assign(_, mir::Rvalue::Closure { .. })
            )),
        "the helper must not construct a compile-time open-call wrapper"
    );
    let mut calls = Vec::new();
    for block in &program.bodies[apply].blocks {
        let Some(node) = block.nodes.last().copied() else {
            continue;
        };
        let site = &tables.nodes[node];
        if site.namespace != main
            || BEYOND_RETURN_SOURCE.get(site.span.start..site.span.end)
                != Some("fn x")
        {
            continue;
        }
        let mir::Terminator::Call {
            callee: mir::Callee::Value(_),
            args,
            dest,
            ..
        } = &block.terminator
        else {
            panic!("exact fn x must emit the actual dynamic Value call")
        };
        assert_eq!(args.len(), 1);
        assert!(
            matches!(
                program.bodies[apply].locals[*dest].ty,
                ir::Type::Function { .. }
            ),
            "later next y constrains the final destination to Function: {:?}",
            program.bodies[apply].locals[*dest].ty
        );
        let ir::Kind::Call { open, .. } = &site.kind else {
            panic!("actual closed IR call behind exact fn x")
        };
        assert!(
            open.is_empty(),
            "recorded call stays closed despite final Function result"
        );
        calls.push(node);
    }
    assert_eq!(
        calls.len(),
        1,
        "one exact dynamic saturated factory producer"
    );
    (apply, go, calls[0])
}

fn observe_beyond_return_operand(
    machine: &Machine,
    apply: usize,
    go: usize,
    test: usize,
    partial_node: usize,
) -> Option<(usize, fipe::value::Guard)> {
    use fipe::{ir, mir, value::Value, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != apply {
        return None;
    }
    let program = machine.program().unwrap();
    let body = &program.bodies[apply];
    let block = &body.blocks[frame.block];
    if frame.statement != block.statements.len()
        || block.nodes.last() != Some(&partial_node)
    {
        return None;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(mir::Operand::Local(local)),
        args,
        ..
    } = &block.terminator
    else {
        return None;
    };
    assert_eq!(args.len(), 1);
    let Value::Function(function) = &frame.locals[*local] else {
        panic!("actual runtime function operand")
    };
    assert_eq!(program.bodies[function.body].arity, 1);
    assert!(function.filled.is_empty());
    let guard = function.guards.iter().find(|guard| {
        fully_applied_partial_test_parameters(&guard.predicate, test).is_some()
    }).unwrap_or_else(|| panic!("semantic Sorted guard must exist BEFORE saturated factory call; actual guards={:?}", function.guards));
    assert_eq!(guard.spot, 1);
    assert!(!guard.witnesses.is_empty());
    let parameters =
        fully_applied_partial_test_parameters(&guard.predicate, test).unwrap();
    assert!(!parameters.is_empty());
    assert!(
        !body.hidden.is_empty(),
        "actual caller hidden={:?}, meanings={:?}",
        body.hidden,
        frame.meanings
    );
    assert_eq!(body.hidden.len(), frame.meanings.len());
    assert!(
        body.hidden.iter().all(|id| !parameters.contains(id)),
        "invoking IDs {:?} must differ from creator IDs {parameters:?}",
        body.hidden
    );
    let tables = program.tables.borrow();
    for parameter in parameters {
        assert!(
            program.bodies[go].hidden.contains(&parameter),
            "actual creator owns Test ID"
        );
        let (_, meaning) = guard
            .witnesses
            .iter()
            .find(|(id, _)| *id == parameter)
            .unwrap();
        let ir::Type::Meaning { index, arguments } = meaning.as_type() else {
            panic!("actual creator Integer witness: {meaning:?}")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert!(arguments.is_empty());
    }
    Some((function.body, guard.clone()))
}

fn assert_beyond_return_value(
    program: &fipe::mir::Program,
    arguments: &[fipe::value::Value],
    before: &(usize, fipe::value::Guard),
    test: usize,
) -> fipe::value::Value {
    use fipe::value::Value;
    let [value @ Value::Function(returned)] = arguments else {
        panic!("relay receives the actual factory return")
    };
    assert_ne!(
        returned.body, before.0,
        "returned inner function is distinct from factory"
    );
    assert_eq!(program.bodies[returned.body].arity, 1);
    assert!(returned.filled.is_empty());
    let tables = program.tables.borrow();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let factory = &tables.nodes[program.bodies[before.0].node];
    assert_eq!(factory.namespace, main);
    assert_eq!(
        BEYOND_RETURN_SOURCE.get(factory.span.start..factory.span.end),
        Some(
            "_ => {\n host.made ()\n result = xs => host.after ()\n result\n}"
        )
    );
    let inner = &tables.nodes[program.bodies[returned.body].node];
    assert_eq!(inner.namespace, main);
    assert_eq!(
        BEYOND_RETURN_SOURCE.get(inner.span.start..inner.span.end),
        Some("xs => host.after ()")
    );
    let guard = returned
        .guards
        .iter()
        .find(|guard| {
            guard.entry == before.1.entry
                && guard.spot == 0
                && fully_applied_partial_test_parameters(&guard.predicate, test)
                    .is_some()
        })
        .expect("factory spot1 propagated to actual returned function spot0");
    assert_eq!(before.1.spot, 1);
    assert!(
        std::rc::Rc::ptr_eq(&guard.witnesses, &before.1.witnesses),
        "beyond transfer retains same captured association Rc"
    );
    value.clone()
}

fn beyond_return_reply(
    machine: &Machine,
    request: &fipe::embed::Request,
    sorted: bool,
    before: Option<&(usize, fipe::value::Guard)>,
    factory_seen: bool,
    test: usize,
    requests: &mut Vec<String>,
) -> fipe::value::Value {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("actual declared host call: {request:?}")
    };
    let program = machine.program().unwrap();
    let label = program.tables.borrow().declarations[*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "items" => {
            assert_eq!(requests.as_slice(), ["items"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::seq(if sorted {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            })
        }
        "made" => {
            assert_eq!(requests.as_slice(), ["items", "made"]);
            assert_eq!(arguments, &[Value::Unit]);
            assert!(
                factory_seen,
                "actual factory frame entered before its effect"
            );
            assert!(
                before.is_some(),
                "actual factory guard observed before its body"
            );
            Value::Unit
        }
        "relay" => {
            assert_eq!(requests.as_slice(), ["items", "made", "relay"]);
            assert!(factory_seen);
            assert_beyond_return_value(
                program,
                arguments,
                before.unwrap(),
                test,
            )
        }
        "after" => {
            assert!(sorted, "unsorted returned function refuses before after");
            assert_eq!(
                requests.as_slice(),
                ["items", "made", "relay", "after"]
            );
            assert_eq!(arguments, &[Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected effect {label}"),
    }
}

fn beyond_return_guard_case(sorted: bool) {
    use fipe::{
        embed::Reply,
        value::Value,
        vm::{Frame, Stop},
    };
    let mut declarations = Declarations::new();
    declarations.module("host", BEYOND_RETURN_HOST).unwrap();
    let module = parse(lex(BEYOND_RETURN_SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("public beyond-arity guarded factory producer");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let joined = machine.program().unwrap();
    let (test, test_body) =
        sorted_runtime_test_identity(joined, BEYOND_RETURN_SOURCE);
    let (apply, go, call_node) = beyond_return_bodies(joined);
    let mut before = None;
    let mut factory_seen = false;
    let mut tested = std::collections::BTreeSet::new();
    let mut requests = Vec::new();
    let mut finished = false;
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(&machine, test_body, &mut tested);
        if let (Some((factory, _)), Some(Frame::Body(frame))) =
            (&before, machine.frames().last())
        {
            if frame.body == *factory {
                factory_seen = true;
            }
        }
        if matches!(&stop, Stop::Before(_)) {
            if let Some(observed) = observe_beyond_return_operand(
                &machine, apply, go, test, call_node,
            ) {
                assert!(before.is_none(), "one saturated factory transition");
                before = Some(observed);
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                if requests.len() < 3 {
                    assert!(tested.is_empty(), "future Sorted argument is not tested before factory returns and relay");
                }
                let reply = beyond_return_reply(
                    &machine,
                    &request,
                    sorted,
                    before.as_ref(),
                    factory_seen,
                    test,
                    &mut requests,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert!(before.is_some(), "actual guarded factory operand");
                assert!(factory_seen, "actual factory body executed");
                assert!(!tested.is_empty(), "actual returned guard executes Sorted with creator Integer");
                if sorted {
                    assert_eq!(result, Ok(Value::Int(7)));
                    assert_eq!(requests, ["items", "made", "relay", "after"]);
                } else {
                    let fault = result.unwrap_err();
                    assert_sorted_runtime_failure(
                        machine.program().unwrap(),
                        &fault,
                        test,
                    );
                    assert_eq!(requests, ["items", "made", "relay"]);
                }
                finished = true;
                break;
            }
        }
    }
    assert!(finished, "bounded actual beyond-arity guard proof finishes");
}

#[test]
fn beyond_return_sorted_guard_shares_creator_witnesses() {
    beyond_return_guard_case(true);
}

#[test]
fn beyond_return_unsorted_guard_refuses_with_shared_creator_witnesses() {
    beyond_return_guard_case(false);
}

const EMPTY_CAPTURE_SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Integer -> Sorted<Integer> -> Integer = n xs => host.after ()\napply_one = fn x y => {\n partial = fn x\n next = host.relay partial\n next y\n}\ngo = xs => apply_one keep 7 xs\ngo (host.items ())\n";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuardBudgetCheckpoint {
    BeforeAttachment,
    Attached,
    BeforeInvocation,
    TestBody,
    AfterRequest,
}

#[derive(Debug)]
struct GuardBudgetReport {
    points: [Option<u64>; 5],
    requests: Vec<String>,
    budget_failed: bool,
    test_seen: bool,
    reached: Option<GuardBudgetCheckpoint>,
    total: u64,
}

fn mark_guard_budget_checkpoint(
    machine: &mut Machine,
    report: &mut GuardBudgetReport,
    stop_at: Option<GuardBudgetCheckpoint>,
    checkpoint: GuardBudgetCheckpoint,
) -> bool {
    let index = match checkpoint {
        GuardBudgetCheckpoint::BeforeAttachment => 0,
        GuardBudgetCheckpoint::Attached => 1,
        GuardBudgetCheckpoint::BeforeInvocation => 2,
        GuardBudgetCheckpoint::TestBody => 3,
        GuardBudgetCheckpoint::AfterRequest => 4,
    };
    assert!(report.points[index].replace(machine.charged()).is_none());
    if stop_at == Some(checkpoint) {
        report.reached = Some(checkpoint);
        report.total = machine.charged();
        assert!(
            machine.abandon(),
            "checkpoint run is deliberately abandoned, not complete"
        );
        return true;
    }
    false
}

fn guard_budget_attachment_position(
    machine: &Machine,
    go: usize,
    test: usize,
) -> Option<(usize, usize, usize, usize)> {
    use fipe::{mir, value::Value, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != go {
        return None;
    }
    let program = machine.program().unwrap();
    let body = &program.bodies[frame.body];
    let block = &body.blocks[frame.block];
    let Some(mir::Statement::Enter {
        entry,
        value: mir::Operand::Local(local),
    }) = block.statements.get(frame.statement)
    else {
        return None;
    };
    let tables = program.tables.borrow();
    fully_applied_partial_test_parameters(
        &tables.entries[*entry].predicate,
        test,
    )?;
    let Value::Function(function) = &frame.locals[*local] else {
        panic!("actual factory operand")
    };
    assert_eq!(program.bodies[function.body].arity, 1);
    assert!(
        !function.guards.iter().any(|guard| {
            fully_applied_partial_test_parameters(&guard.predicate, test)
                .is_some()
        }),
        "capture checkpoint precedes semantic guard attachment"
    );
    assert!(!matches!(block.statements.get(frame.statement + 1), Some(mir::Statement::Charge(_))),
        "adjacent attachment checkpoint must not include an unrelated block charge");
    Some((frame.body, frame.block, frame.statement, *local))
}

fn assert_guard_budget_attachment(
    machine: &Machine,
    position: (usize, usize, usize, usize),
    test: usize,
) {
    use fipe::{value::Value, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        panic!("adjacent body stop after attachment")
    };
    assert_eq!(
        (frame.body, frame.block, frame.statement),
        (position.0, position.1, position.2 + 1)
    );
    let Value::Function(function) = &frame.locals[position.3] else {
        panic!("same guarded local after attachment")
    };
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            fully_applied_partial_test_parameters(&guard.predicate, test)
                .is_some()
        })
        .expect("actual semantic guard installed at adjacent stop");
    assert_eq!(guard.spot, 1);
    assert!(!guard.witnesses.is_empty());
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    for parameter in
        fully_applied_partial_test_parameters(&guard.predicate, test).unwrap()
    {
        assert!(program.bodies[frame.body].hidden.contains(&parameter));
        let (_, meaning) = guard
            .witnesses
            .iter()
            .find(|(id, _)| *id == parameter)
            .unwrap();
        let fipe::ir::Type::Meaning { index, arguments } = meaning.as_type()
        else {
            panic!("actual captured Integer")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert!(arguments.is_empty());
    }
}

fn guard_budget_final_call(machine: &Machine, apply: usize) -> bool {
    use fipe::{mir, vm::Frame};

    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let block = &joined.bodies[frame.body].blocks[frame.block];
        let tables = joined.tables.borrow();
        frame.body == apply
            && frame.statement == block.statements.len()
            && block.nodes.last().is_some_and(|node| {
                let site = &tables.nodes[*node];
                tables.namespaces[site.namespace] == "main"
                    && BEYOND_RETURN_SOURCE.get(site.span.start..site.span.end)
                        == Some("next y")
            })
            && matches!(
                block.terminator,
                mir::Terminator::Call {
                    callee: mir::Callee::Value(_),
                    ..
                }
            )
    } else {
        false
    }
}

fn guard_budget_request(
    machine: &mut Machine,
    request: &fipe::embed::Request,
    report: &mut GuardBudgetReport,
    stop_at: Option<GuardBudgetCheckpoint>,
    before: Option<&(usize, fipe::value::Guard)>,
    factory_seen: bool,
    test: usize,
) -> bool {
    use fipe::embed::{Reply, Request};
    let Request::Call { declaration, .. } = request else {
        panic!("actual declared request")
    };
    let is_after = machine.program().unwrap().tables.borrow().declarations
        [*declaration]
        .label
        == "after";
    let reply = beyond_return_reply(
        machine,
        request,
        true,
        before,
        factory_seen,
        test,
        &mut report.requests,
    );
    if is_after
        && mark_guard_budget_checkpoint(
            machine,
            report,
            stop_at,
            GuardBudgetCheckpoint::AfterRequest,
        )
    {
        return true;
    }
    machine.answer(Reply::Value(Ok(reply)));
    false
}

fn guard_budget_done(
    machine: &Machine,
    report: &mut GuardBudgetReport,
    result: Result<fipe::value::Value, fipe::vm::Fault>,
    complete: bool,
) {
    use fipe::value::{Reason, Value};
    match result {
        Err(fault) => {
            assert_eq!(
                fault.reason,
                Reason::Budget,
                "only measured budget refusal is admitted"
            );
            assert_eq!(Some(machine.charged()), machine.budget());
            report.budget_failed = true;
        }
        Ok(value) => {
            assert_eq!(value, Value::Int(7));
            assert!(complete);
            assert!(
                report.points.iter().all(Option::is_some),
                "all semantic boundaries measured: {report:?}"
            );
            assert_eq!(report.requests, ["items", "made", "relay", "after"]);
        }
    }
}

fn guard_budget_test_body(
    machine: &mut Machine,
    report: &mut GuardBudgetReport,
    stop_at: Option<GuardBudgetCheckpoint>,
    test_body: usize,
) -> bool {
    use fipe::vm::Frame;
    if report.points[3].is_none()
        && matches!(machine.frames().last(), Some(Frame::Body(frame)) if frame.body == test_body)
    {
        assert!(
            machine.charged()
                > report.points[2].expect("observed final guarded call")
        );
        if mark_guard_budget_checkpoint(
            machine,
            report,
            stop_at,
            GuardBudgetCheckpoint::TestBody,
        ) {
            return true;
        }
    }
    false
}

const fn guard_budget_report() -> GuardBudgetReport {
    GuardBudgetReport {
        points: [None; 5],
        requests: Vec::new(),
        budget_failed: false,
        test_seen: false,
        reached: None,
        total: 0,
    }
}

fn guard_budget_factory_seen(
    machine: &Machine,
    before: Option<&(usize, fipe::value::Guard)>,
) -> bool {
    use fipe::vm::Frame;
    if let (Some((factory, _)), Some(Frame::Body(frame))) =
        (before, machine.frames().last())
    {
        frame.body == *factory
    } else {
        false
    }
}

fn guard_budget_run(
    machine: &mut Machine,
    program: &fipe::mir::Program,
    stop_at: Option<GuardBudgetCheckpoint>,
) -> GuardBudgetReport {
    use fipe::vm::Stop;
    machine.begin(program);
    let joined = machine.program().unwrap();
    let (test, test_body) =
        sorted_runtime_test_identity(joined, BEYOND_RETURN_SOURCE);
    let (apply, go, call_node) = beyond_return_bodies(joined);
    let mut report = guard_budget_report();
    let mut attachment = None;
    let mut before = None;
    let mut factory_seen = false;
    let mut tested = std::collections::BTreeSet::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(machine, test_body, &mut tested);
        report.test_seen |= !tested.is_empty();
        factory_seen |= guard_budget_factory_seen(machine, before.as_ref());
        if matches!(&stop, Stop::Before(_)) {
            if let Some(position) = attachment.take() {
                assert_guard_budget_attachment(machine, position, test);
                assert!(
                    machine.charged() > report.points[0].unwrap(),
                    "actual attachment work is nonzero"
                );
                if mark_guard_budget_checkpoint(
                    machine,
                    &mut report,
                    stop_at,
                    GuardBudgetCheckpoint::Attached,
                ) {
                    return report;
                }
            }
            if let Some(position) =
                guard_budget_attachment_position(machine, go, test)
            {
                attachment = Some(position);
                if mark_guard_budget_checkpoint(
                    machine,
                    &mut report,
                    stop_at,
                    GuardBudgetCheckpoint::BeforeAttachment,
                ) {
                    return report;
                }
            }
            if let Some(observed) = observe_beyond_return_operand(
                machine, apply, go, test, call_node,
            ) {
                assert!(before.replace(observed).is_none());
            }
            let final_call = guard_budget_final_call(machine, apply);
            if final_call {
                assert_eq!(report.requests, ["items", "made", "relay"]);
                assert!(before.is_some());
                if mark_guard_budget_checkpoint(
                    machine,
                    &mut report,
                    stop_at,
                    GuardBudgetCheckpoint::BeforeInvocation,
                ) {
                    return report;
                }
            }
            if guard_budget_test_body(machine, &mut report, stop_at, test_body)
            {
                return report;
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                if guard_budget_request(
                    machine,
                    &request,
                    &mut report,
                    stop_at,
                    before.as_ref(),
                    factory_seen,
                    test,
                ) {
                    return report;
                }
            }
            Stop::Done(result) => {
                report.total = machine.charged();
                guard_budget_done(
                    machine,
                    &mut report,
                    result,
                    before.is_some() && factory_seen && !tested.is_empty(),
                );
                return report;
            }
        }
    }
    panic!("bounded budget producer must finish or reach its chosen checkpoint")
}

#[test]
fn captured_guard_budget_boundaries_precede_effects_and_allow_healthy_retry() {
    use GuardBudgetCheckpoint::{AfterRequest, Attached, TestBody};
    let mut declarations = Declarations::new();
    declarations.module("host", BEYOND_RETURN_HOST).unwrap();
    let module = parse(lex(BEYOND_RETURN_SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let calibration = guard_budget_run(&mut Machine::new(None), &program, None);
    let repeat = guard_budget_run(&mut Machine::new(None), &program, None);
    assert_eq!(repeat.points, calibration.points);
    assert_eq!(repeat.total, calibration.total);
    let [c0, c1, i0, i1, e0] = calibration.points.map(Option::unwrap);
    assert!(
        c0 < c1 && c1 <= i0 && i0 < i1 && i1 < e0 && e0 <= calibration.total
    );
    eprintln!("GUARD_BUDGET attachment={c0}..{c1}, invocation={i0}..{i1}, after={e0}, complete={}", calibration.total);
    for (allowance, prefix, forbidden_point) in [
        (c0, vec!["items"], 1),
        (c1 - 1, vec!["items"], 1),
        (i0, vec!["items", "made", "relay"], 3),
        (i1 - 1, vec!["items", "made", "relay"], 3),
        (e0 - 1, vec!["items", "made", "relay"], 4),
    ] {
        let mut machine = Machine::new(Some(allowance));
        let failed = guard_budget_run(&mut machine, &program, None);
        assert!(failed.budget_failed, "allowance {allowance}: {failed:?}");
        assert_eq!(failed.total, allowance);
        assert_eq!(failed.requests, prefix);
        assert!(failed.points[forbidden_point].is_none(),
            "allowance {allowance} must not reach checkpoint {forbidden_point}: {failed:?}");
        if allowance == i0 {
            assert!(
                !failed.test_seen,
                "zero-remaining invocation must not execute the semantic test"
            );
        }
        machine.set_budget(None);
        let healthy = guard_budget_run(&mut machine, &program, None);
        assert!(!healthy.budget_failed);
        assert!(healthy.points.iter().all(Option::is_some));
        // A retried Machine retains legitimate globals; only its fresh
        // semantic checkpoints and result, not cold-run cost, are required.
    }
    for (allowance, checkpoint) in
        [(c1, Attached), (i1, TestBody), (e0, AfterRequest)]
    {
        let mut machine = Machine::new(Some(allowance));
        let reached =
            guard_budget_run(&mut machine, &program, Some(checkpoint));
        assert_eq!(reached.reached, Some(checkpoint));
        assert_eq!(reached.total, allowance);
        assert!(
            !reached.budget_failed,
            "checkpoint reach is distinct from whole-program completion"
        );
    }
    let complete = guard_budget_run(
        &mut Machine::new(Some(calibration.total)),
        &program,
        None,
    );
    assert!(!complete.budget_failed);
    assert_eq!(complete.total, calibration.total);
}
fn empty_capture_bodies(program: &fipe::mir::Program) -> (usize, usize, usize) {
    use fipe::{ir, mir};
    let tables = program.tables.borrow();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let body = |name| {
        let definition = tables
            .definitions
            .iter()
            .position(|definition| {
                definition.namespace == main && definition.label == name
            })
            .unwrap();
        program.definitions[definition].expect("actual source body")
    };
    let apply = body("apply_one");
    let go = body("go");
    assert_eq!(program.bodies[apply].arity, 3);
    assert!(
        !program.bodies[apply]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(
                statement,
                mir::Statement::Assign(_, mir::Rvalue::Closure { .. })
            )),
        "the helper must not construct a compile-time open-call wrapper"
    );
    let mut calls = Vec::new();
    for block in &program.bodies[apply].blocks {
        let Some(node) = block.nodes.last().copied() else {
            continue;
        };
        let site = &tables.nodes[node];
        if site.namespace != main
            || EMPTY_CAPTURE_SOURCE.get(site.span.start..site.span.end)
                != Some("fn x")
        {
            continue;
        }
        let mir::Terminator::Call {
            callee: mir::Callee::Value(_),
            args,
            dest,
            ..
        } = &block.terminator
        else {
            panic!("exact fn x must emit the actual dynamic Value call")
        };
        assert_eq!(args.len(), 1);
        assert!(
            matches!(
                program.bodies[apply].locals[*dest].ty,
                ir::Type::Function { .. }
            ),
            "later next y constrains the final destination to Function: {:?}",
            program.bodies[apply].locals[*dest].ty
        );
        let ir::Kind::Call { open, .. } = &site.kind else {
            panic!("actual closed IR call behind exact fn x")
        };
        assert!(
            open.is_empty(),
            "recorded call stays closed despite final Function result"
        );
        calls.push(node);
    }
    assert_eq!(calls.len(), 1, "one exact dynamic underarity producer");
    (apply, go, calls[0])
}

fn empty_capture_test_meanings(
    predicate: &fipe::ir::Predicate,
    test: usize,
) -> Option<&[fipe::ir::Type]> {
    use fipe::ir::Predicate;
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                pending.extend(parts);
            }
            Predicate::Not(inner) => pending.push(inner),
            Predicate::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            Predicate::Test { index, meanings } if *index == test => {
                return Some(meanings)
            }
            Predicate::Nonzero { .. } | Predicate::Test { .. } => {}
        }
    }
    None
}

fn observe_empty_capture_operand(
    machine: &Machine,
    apply: usize,
    call_node: usize,
    test: usize,
) -> Option<(usize, fipe::value::Guard)> {
    use fipe::{ir, mir, value::Value, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != apply {
        return None;
    }
    let program = machine.program().unwrap();
    let block = &program.bodies[apply].blocks[frame.block];
    if frame.statement != block.statements.len()
        || block.nodes.last() != Some(&call_node)
    {
        return None;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(mir::Operand::Local(local)),
        args,
        ..
    } = &block.terminator
    else {
        panic!("actual closed local function call")
    };
    assert_eq!(args.len(), 1);
    let Value::Function(function) = &frame.locals[*local] else {
        panic!("actual function operand")
    };
    assert_eq!(program.bodies[function.body].arity, 2);
    assert!(function.filled.is_empty());
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            empty_capture_test_meanings(&guard.predicate, test).is_some()
        })
        .expect(
            "nonempty semantic Sorted Guard even though its context is empty",
        );
    assert_eq!(guard.spot, 1);
    assert!(
        guard.witnesses.is_empty(),
        "monomorphic guard must not capture phantom witness IDs"
    );
    let meanings = empty_capture_test_meanings(&guard.predicate, test).unwrap();
    assert!(
        !meanings.is_empty(),
        "the real Test still has a concrete type argument"
    );
    let tables = program.tables.borrow();
    for meaning in meanings {
        let ir::Type::Meaning { index, arguments } = meaning else {
            panic!("concrete Integer, not a captured Parameter: {meaning:?}")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert!(arguments.is_empty());
    }
    Some((function.body, guard.clone()))
}

fn empty_capture_reply(
    machine: &Machine,
    request: &fipe::embed::Request,
    sorted: bool,
    before: Option<&(usize, fipe::value::Guard)>,
    test: usize,
    requests: &mut Vec<String>,
) -> fipe::value::Value {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("actual declared request")
    };
    let program = machine.program().unwrap();
    let label = program.tables.borrow().declarations[*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "items" => {
            assert_eq!(requests.as_slice(), ["items"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::seq(if sorted {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            })
        }
        "relay" => {
            assert_eq!(requests.as_slice(), ["items", "relay"]);
            let before = before
                .as_ref()
                .expect("observe actual empty-context Guard before partial");
            let [value @ Value::Function(partial)] = arguments.as_slice()
            else {
                panic!("actual dynamic partial")
            };
            assert_eq!(partial.body, before.0);
            assert_eq!(partial.filled, [Value::Int(7)]);
            let guard = partial
                .guards
                .iter()
                .find(|guard| {
                    guard.entry == before.1.entry
                        && guard.spot == 1
                        && empty_capture_test_meanings(&guard.predicate, test)
                            .is_some()
                })
                .expect("same semantic guard after partial");
            assert!(guard.witnesses.is_empty());
            assert!(
                std::rc::Rc::ptr_eq(&guard.witnesses, &before.1.witnesses),
                "same empty association allocation across partial"
            );
            value.clone()
        }
        "after" => {
            assert!(sorted);
            assert_eq!(requests.as_slice(), ["items", "relay", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected effect {label}"),
    }
}

fn empty_capture_guard_case(sorted: bool) {
    use fipe::{embed::Reply, value::Value, vm::Stop};
    let mut declarations = Declarations::new();
    declarations
        .module("host", FULLY_APPLIED_PARTIAL_HOST)
        .unwrap();
    let module = parse(lex(EMPTY_CAPTURE_SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("public monomorphic semantic Guard producer");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(None);
    machine.begin(&program);
    let joined = machine.program().unwrap();
    let (test, test_body) =
        sorted_runtime_test_identity(joined, EMPTY_CAPTURE_SOURCE);
    let (apply, _, call_node) = empty_capture_bodies(joined);
    let mut before = None;
    let mut tested = std::collections::BTreeSet::new();
    let mut requests = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(&machine, test_body, &mut tested);
        if matches!(&stop, Stop::Before(_)) {
            if let Some(observed) =
                observe_empty_capture_operand(&machine, apply, call_node, test)
            {
                assert!(before.replace(observed).is_none());
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = empty_capture_reply(
                    &machine,
                    &request,
                    sorted,
                    before.as_ref(),
                    test,
                    &mut requests,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert!(
                    before.is_some() && !tested.is_empty(),
                    "real guarded partial and concrete Integer test execute"
                );
                if sorted {
                    assert_eq!(result, Ok(Value::Int(7)));
                    assert_eq!(requests, ["items", "relay", "after"]);
                } else {
                    assert_sorted_runtime_failure(
                        machine.program().unwrap(),
                        &result.unwrap_err(),
                        test,
                    );
                    assert_eq!(requests, ["items", "relay"]);
                }
                return;
            }
        }
    }
    panic!("bounded empty-context semantic Guard run finishes")
}

#[test]
fn monomorphic_guard_empty_context_preserves_sorted_partial() {
    empty_capture_guard_case(true);
}

#[test]
fn monomorphic_guard_empty_context_refuses_unsorted_partial() {
    empty_capture_guard_case(false);
}

const PROJECTED_COMPONENT_HOST: &str =
    "pub batches: a -> Vec<Vec<a>>\npub after: () -> Integer\n";
const PROJECTED_COMPONENT_SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ngo: Ord a => a -> Integer = seed => {\n xs: Vec<Sorted<a>> = host.batches seed\n host.after ()\n}\ngo 7\n";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectedComponentCase {
    Empty,
    Sorted,
    Unsorted,
}

fn projected_component_program() -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations
        .module("host", PROJECTED_COMPONENT_HOST)
        .expect("projected entry Host schema admission");
    let module = parse(lex(PROJECTED_COMPONENT_SOURCE)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("public generic projected entry producer admission");
    compile::compile(&unit)
}

fn projected_component_go(program: &fipe::mir::Program) -> usize {
    let tables = program.tables.borrow();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "go"
                && tables.namespaces[definition.namespace] == "main"
        })
        .unwrap();
    let body = program.definitions[definition].expect("actual go body");
    assert_eq!(program.bodies[body].arity, 1);
    let site = &tables.nodes[program.bodies[body].node];
    assert_eq!(tables.namespaces[site.namespace], "main");
    assert_eq!(PROJECTED_COMPONENT_SOURCE.get(site.span.start..site.span.end),
        Some("seed => {\n xs: Vec<Sorted<a>> = host.batches seed\n host.after ()\n}"));
    body
}

fn observe_projected_component_entry(
    machine: &Machine,
    go: usize,
    test: usize,
    case: ProjectedComponentCase,
) -> Option<(usize, usize, usize, u64)> {
    use fipe::{ir, mir, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != go {
        return None;
    }
    let program = machine.program().unwrap();
    let body = &program.bodies[go];
    let block = &body.blocks[frame.block];
    let Some(mir::Statement::Enter {
        entry,
        value: mir::Operand::Local(local),
    }) = block.statements.get(frame.statement)
    else {
        return None;
    };
    let tables = program.tables.borrow();
    let entry = &tables.entries[*entry];
    let parameters =
        fully_applied_partial_test_parameters(&entry.predicate, test)?;
    assert_eq!(
        entry.path,
        [ir::Projection::Element],
        "actual ordinary entry projection, not a whole-vector guard"
    );
    assert!(!matches!(entry.predicate, ir::Predicate::Function { .. }));
    let fipe::value::Value::Seq(outer) = &frame.locals[*local] else {
        panic!("actual ordinary outer vector, not a Function")
    };
    assert_eq!(
        outer.len(),
        usize::from(case != ProjectedComponentCase::Empty)
    );
    assert!(
        !parameters.is_empty(),
        "nonvacuous demanded generic Test argument"
    );
    assert!(!body.hidden.is_empty());
    assert_eq!(body.hidden.len(), frame.meanings.len());
    for parameter in parameters {
        let position = body
            .hidden
            .iter()
            .position(|id| *id == parameter)
            .expect("actual go owns each demanded Test parameter");
        let ir::Type::Meaning { index, arguments } = &frame.meanings[position]
        else {
            panic!("actual creator Integer")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert!(arguments.is_empty());
    }
    let ir::Predicate::All(parts) = &entry.predicate else {
        panic!("the actual projected Sorted entry")
    };
    let [ir::Predicate::All(base), ir::Predicate::Test { index, meanings }] =
        parts.as_slice()
    else {
        panic!("the empty base and one actual Sorted Test")
    };
    assert!(base.is_empty());
    assert_eq!(*index, test);
    assert!(matches!(meanings.as_slice(), [ir::Type::Parameter(_)]));
    // Snapshot syntax: outer All Read/2edges/Finish; empty All Read/Finish;
    // Test Read/meaning-slot/raw Parameter Read/Finish. No capture or run.
    let snapshot_work = (1_u64 + 2 + 1) + (1 + 1) + (1 + 1 + 1 + 1);
    if case == ProjectedComponentCase::Empty {
        assert!(!matches!(block.statements.get(frame.statement + 1), Some(mir::Statement::Charge(_))),
            "empty projection adjacent checkpoint must not include unrelated charge");
    }
    Some((frame.body, frame.block, frame.statement, snapshot_work))
}

fn projected_component_reply(
    machine: &Machine,
    request: &fipe::embed::Request,
    case: ProjectedComponentCase,
    requests: &mut Vec<String>,
    entered: bool,
    tested: bool,
) -> fipe::value::Value {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("actual declared request")
    };
    let label = machine.program().unwrap().tables.borrow().declarations
        [*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "batches" => {
            assert_eq!(requests.as_slice(), ["batches"]);
            assert_eq!(arguments, &[Value::Int(7)]);
            let outer = match case {
                ProjectedComponentCase::Empty => Vec::new(),
                ProjectedComponentCase::Sorted => {
                    vec![Value::seq(vec![Value::Int(1), Value::Int(2)])]
                }
                ProjectedComponentCase::Unsorted => {
                    vec![Value::seq(vec![Value::Int(2), Value::Int(1)])]
                }
            };
            Value::seq(outer)
        }
        "after" => {
            assert!(
                entered,
                "actual projected entry must execute before after"
            );
            assert_ne!(
                case,
                ProjectedComponentCase::Unsorted,
                "unsorted refuses before after"
            );
            assert_eq!(tested, case != ProjectedComponentCase::Empty);
            assert_eq!(requests.as_slice(), ["batches", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected projected-entry effect {label}"),
    }
}

fn assert_projected_component_result(
    machine: &Machine,
    case: ProjectedComponentCase,
    test: usize,
    result: Result<fipe::value::Value, fipe::vm::Fault>,
    requests: &[String],
    tested: bool,
) {
    if case == ProjectedComponentCase::Unsorted {
        assert!(tested, "nonempty projected Test executes");
        assert_sorted_runtime_failure(
            machine.program().unwrap(),
            &result.unwrap_err(),
            test,
        );
        assert_eq!(requests, ["batches"]);
    } else {
        assert_eq!(result, Ok(fipe::value::Value::Int(7)));
        assert_eq!(requests, ["batches", "after"]);
        assert_eq!(tested, case != ProjectedComponentCase::Empty);
    }
}

fn projected_empty_checkpoint(
    machine: &Machine,
    stop: &fipe::vm::Stop,
    position: (usize, usize, usize, u64),
    entered: Option<u64>,
    requests: &[String],
    tests_empty: bool,
) -> u64 {
    use fipe::vm::{Frame, Stop};
    assert!(
        matches!(stop, Stop::Before(_)),
        "next stop must be adjacent go instruction"
    );
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        panic!("same go frame after empty ordinary entry")
    };
    assert_eq!(
        (frame.body, frame.block, frame.statement),
        (position.0, position.1, position.2 + 1)
    );
    assert_eq!(
        machine.charged(),
        entered
            .expect("before snapshot")
            .checked_add(position.3)
            .unwrap(),
        "empty projection pays only the exact raw snapshot"
    );
    assert!(tests_empty);
    assert_eq!(requests, ["batches"]);
    machine.charged()
}

fn projected_component_run(
    program: &fipe::mir::Program,
    case: ProjectedComponentCase,
    budget: Option<u64>,
    abandon_after_empty_entry: bool,
) -> u64 {
    use fipe::{embed::Reply, vm::Stop};
    let mut machine = Machine::new(budget);
    machine.begin(program);
    let joined = machine.program().unwrap();
    let (test, test_body) =
        sorted_runtime_test_identity(joined, PROJECTED_COMPONENT_SOURCE);
    let go = projected_component_go(joined);
    let mut entered: Option<u64> = None;
    let mut adjacent: Option<(usize, usize, usize, u64)> = None;
    let mut after_empty_snapshot = None;
    let mut empty_boundary_seen = false;
    let mut tested = std::collections::BTreeSet::new();
    let mut requests = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_sorted_integer_witnesses(&machine, test_body, &mut tested);
        if let Some(position) = adjacent.take() {
            let snapshot_charge = projected_empty_checkpoint(
                &machine,
                &stop,
                position,
                entered,
                &requests,
                tested.is_empty(),
            );
            empty_boundary_seen = true;
            after_empty_snapshot = Some(snapshot_charge);
            if abandon_after_empty_entry {
                assert_eq!(budget, Some(machine.charged()), "exact measured allowance has zero remaining after the snapshot");
                assert!(machine.abandon(), "checkpoint reached then deliberately abandoned, not whole-program success");
                return machine.charged();
            }
        }
        if matches!(&stop, Stop::Before(_)) {
            if let Some(position) =
                observe_projected_component_entry(&machine, go, test, case)
            {
                assert!(
                    entered.replace(machine.charged()).is_none(),
                    "one actual semantic projected entry"
                );
                if case == ProjectedComponentCase::Empty {
                    adjacent = Some(position);
                }
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = projected_component_reply(
                    &machine,
                    &request,
                    case,
                    &mut requests,
                    entered.is_some(),
                    !tested.is_empty(),
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                let charge =
                    entered.expect("actual emitted projected entry executed");
                assert!(
                    !abandon_after_empty_entry,
                    "finite run must reach its exact adjacent checkpoint"
                );
                assert_eq!(
                    empty_boundary_seen,
                    case == ProjectedComponentCase::Empty
                );
                assert_projected_component_result(
                    &machine,
                    case,
                    test,
                    result,
                    &requests,
                    !tested.is_empty(),
                );
                return if case == ProjectedComponentCase::Empty {
                    after_empty_snapshot
                        .expect("actual post-snapshot checkpoint")
                } else {
                    charge
                };
            }
        }
    }
    panic!("bounded public projected-entry run finishes")
}

#[test]
fn empty_projected_predicate_entry_skips_capture_work_at_exact_budget() {
    let program = projected_component_program();
    let measured = projected_component_run(
        &program,
        ProjectedComponentCase::Empty,
        None,
        false,
    );
    assert_eq!(
        projected_component_run(
            &program,
            ProjectedComponentCase::Empty,
            Some(measured),
            true
        ),
        measured
    );
}

#[test]
fn sorted_projected_predicate_entry_runs_actual_integer_test() {
    projected_component_run(
        &projected_component_program(),
        ProjectedComponentCase::Sorted,
        None,
        false,
    );
}

#[test]
fn unsorted_projected_predicate_entry_refuses_before_after() {
    projected_component_run(
        &projected_component_program(),
        ProjectedComponentCase::Unsorted,
        None,
        false,
    );
}

const WRITTEN_BOUNDARY_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: Self -> Integer -> Integer = self => (x => self.n + x), flat: Self -> Integer -> Integer = self x => self.n + x, last: Integer -> Self -> Integer = x self => self.n + x, labeled: (self: Self) -> Integer -> Integer = self => (x => self.n + x), defaulted: Self -> (x: Integer = 3) -> Integer = self (x = 3) => self.n + x }\npub k: Integer -> Integer -> Integer = n => (x => n + x)\npub identity: a -> a -> a = ignored => (x => x)\n";

fn written_boundary_result(expression: &str) {
    let mut declarations = Declarations::new();
    declarations
        .module("methods", WRITTEN_BOUNDARY_METHODS)
        .unwrap();
    let source = format!("use methods\n{expression}\n");
    let module = parse(lex(&source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    let Outcome::Done(result) = machine.run(&program) else {
        panic!("written methods execute without a Host callback: {expression}");
    };
    assert_eq!(result.unwrap(), super::Value::Int(10), "{expression}");
}

#[test]
fn external_written_members_preserve_receiver_and_raw_spots() {
    for expression in [
        "(methods.M 7).op 3",
        "get = m => m.op\nf = get (methods.M 7)\nf 3",
        "(methods.M 7).flat 3",
        "(methods.M 7).last 3",
        "(methods.M 7).labeled 3",
        "(methods.M 7).defaulted",
        "f = (methods.M 7).op\nf 3",
    ] {
        written_boundary_result(expression);
    }
}

#[test]
fn external_written_function_keeps_returned_function_boundary() {
    for expression in [
        "methods.k 7 3",
        "f = methods.k 7\nf 3",
        "get = n => methods.k n\nf = get 7\nf 3",
        "methods.identity 0 10",
    ] {
        written_boundary_result(expression);
    }
}

#[test]
fn external_written_receiver_metadata_matches_selected_body() {
    for (member, arity, receiver) in
        [("op", 1, 0), ("flat", 2, 0), ("last", 2, 1)]
    {
        let mut declarations = Declarations::new();
        declarations
            .module("methods", WRITTEN_BOUNDARY_METHODS)
            .unwrap();
        let source = format!("use methods\n(methods.M 7).{member} 3\n");
        let module = parse(lex(&source)).unwrap();
        let resolved =
            fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed =
            fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
        let start = WRITTEN_BOUNDARY_METHODS
            .find(&format!("{member}:"))
            .unwrap();
        let site = fipe::token::Span {
            start,
            end: start + member.len(),
        };
        let mut matches = 0;
        for (stage, selected) in &typed.selected {
            let fipe::typed::Selected::Value {
                origin,
                site: actual,
            } = selected
            else {
                continue;
            };
            if *origin != fipe::resolve::Origin::Host("methods".into())
                || *actual != site
            {
                continue;
            }
            let Some(bound) = typed.bound.get(stage) else {
                continue;
            };
            assert_eq!(bound.spots.len(), arity, "{member} raw checked spots");
            assert!(
                bound.supplies.iter().any(|(spot, supply)| *spot == receiver
                    && matches!(supply, fipe::typed::Supply::Head(_))),
                "{member} actual receiver supply"
            );
            if member == "op" {
                assert_eq!(
                    bound.applied.len(),
                    1,
                    "argument applies returned function"
                );
            } else {
                assert!(
                    bound.applied.is_empty(),
                    "two-head body binds argument directly"
                );
            }
            matches += 1;
        }
        assert!(
            matches > 0,
            "source-linked selected member has actual Bound metadata"
        );
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        let tables = program.tables.borrow();
        let definitions: Vec<_> = tables
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(index, definition)| {
                (tables.namespaces[definition.namespace] == "methods"
                    && definition.span == site)
                    .then_some(index)
            })
            .collect();
        assert_eq!(definitions.len(), 1, "exact selected source definition");
        let body =
            program.definitions[definitions[0]].expect("written callable body");
        assert_eq!(
            program.bodies[body].arity, arity,
            "selected raw body arity"
        );
    }
}

const PROVIDER_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\npub k: Integer -> U8 -> U8 = self => (x => x)\n";
const PROVIDER_FILE_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { pub op: Self -> U8 -> U8 = self => (x => x) }\npub k: Integer -> U8 -> U8 = self => (x => x)\n";
const PROVIDER_CALLBACKS: &str =
    "pub after: () -> Integer\npub arg: () -> Integer\n";
const PROVIDER_MAIN: [&str; 3] = ["use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ())","use host\nuse methods\nf = (methods.M 7).op\nhost.after ()\nf (host.arg ())","use host\nuse methods\nget = m => methods.k m\nf = get 7\nhost.after ()\nf (host.arg ())"];
const PROVIDER_LINE_MAIN: [&str; 3] = [
    "use host\nget = m => m.op\nf = get (M 7)\nhost.after ()\nf (host.arg ())",
    "use host\nf = (M 7).op\nhost.after ()\nf (host.arg ())",
    "use host\nget = m => k m\nf = get 7\nhost.after ()\nf (host.arg ())",
];

fn assert_external_provider(
    typed: &fipe::typed::Typed,
    origin: &fipe::resolve::Origin,
    methods: &str,
) {
    let start = methods.find("op:").unwrap();
    let contracts: Vec<_> = typed.member_contracts.values().filter(|contract| {
        matches!(&contract.selected, fipe::typed::Selected::Value { origin: actual, site }
            if actual == origin && site.start == start && site.end == start + 2)
    }).collect();
    assert!(
        !contracts.is_empty(),
        "actual external written provider contract: {origin:?}"
    );
    for contract in contracts {
        assert_eq!(&contract.home, origin);
        assert!(
            contract.meanings.is_empty(),
            "monomorphic provider has no invented meanings"
        );
        assert!(
            !contract.reads.is_empty(),
            "actual generic member-read association"
        );
        let fipe::types::Type::Function { parameter, result } =
            &contract.callable
        else {
            panic!("returned callable boundary")
        };
        for ty in [parameter.as_ref(), result.as_ref()] {
            let fipe::types::Type::Limited { name, .. } = ty else {
                panic!("declared U8 place retained")
            };
            assert!(
                matches!(name.as_ref(), fipe::types::Type::Named { name, .. }
                if name.origin == fipe::resolve::Origin::Prelude && name.name == "U8")
            );
        }
    }
}

fn run_external_provider_case(
    program: &fipe::mir::Program,
    machine: &mut Machine,
    argument: i64,
) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let mut outcome = machine.run(program);
    let mut calls = Vec::new();
    loop {
        match outcome {
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                let label = {
                    let tables = machine.program().unwrap().tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    assert_eq!(
                        tables.namespaces[declaration.namespace],
                        "host"
                    );
                    declaration.label.clone()
                };
                assert_eq!(arguments, [Value::Unit]);
                assert!(environment.is_empty());
                let reply = match label.as_str() {
                    "after" => 7,
                    "arg" => argument,
                    _ => panic!("unexpected callback"),
                };
                calls.push(label);
                assert!(calls.len() <= 2);
                outcome =
                    machine.reply(program, Reply::Value(Ok(Value::Int(reply))));
            }
            Outcome::Done(result) => {
                assert_eq!(calls, ["after", "arg"]);
                if argument == 3 {
                    assert_eq!(result.unwrap(), Value::Int(3));
                } else {
                    let fault = result
                        .expect_err("300 must not escape returned U8 contract");
                    let Reason::Failed { entry, value } = fault.reason else {
                        panic!("semantic U8 refusal")
                    };
                    assert_eq!(value, "300");
                    let tables = machine.program().unwrap().tables.borrow();
                    let entry = &tables.entries[entry];
                    assert_eq!(entry.label, "U8");
                    let mut pending = vec![&entry.predicate];
                    let mut prelude_test = false;
                    while let Some(predicate) = pending.pop() {
                        match predicate {
                            fipe::ir::Predicate::All(parts)
                            | fipe::ir::Predicate::Any(parts) => {
                                pending.extend(parts);
                            }
                            fipe::ir::Predicate::Not(inner) => {
                                pending.push(inner);
                            }
                            fipe::ir::Predicate::Test { index, .. } => {
                                prelude_test |= tables.namespaces
                                    [tables.tests[*index].namespace]
                                    == "prelude";
                            }
                            _ => {}
                        }
                    }
                    assert!(
                        prelude_test,
                        "actual prelude Test in located U8 entry"
                    );
                }
                return;
            }
            Outcome::Request(_) => panic!("unexpected noncall request"),
        }
    }
}

fn external_provider_module_case(file: bool) {
    for (route, source) in PROVIDER_MAIN.iter().enumerate() {
        for argument in [3, 300] {
            let mut declarations = Declarations::new();
            declarations.module("host", PROVIDER_CALLBACKS).unwrap();
            let mut files = Vec::new();
            let (origin, methods) = if file {
                files.push((
                    "methods".into(),
                    parse(lex(PROVIDER_FILE_METHODS)).unwrap(),
                ));
                (
                    fipe::resolve::Origin::File("methods".into()),
                    PROVIDER_FILE_METHODS,
                )
            } else {
                declarations.module("methods", PROVIDER_METHODS).unwrap();
                (
                    fipe::resolve::Origin::Host("methods".into()),
                    PROVIDER_METHODS,
                )
            };
            let module = parse(lex(source)).unwrap();
            let resolved =
                fipe::resolve::resolve(&module, &declarations, &files).unwrap();
            let typed =
                fipe::infer::infer(&module, &resolved, &declarations, &files)
                    .unwrap();
            if route == 0 {
                assert_external_provider(&typed, &origin, methods);
            }
            let unit = check::check(
                &module,
                &declarations,
                &files,
                check::Options::default(),
            )
            .unwrap();
            let program = compile::compile(&unit);
            run_external_provider_case(
                &program,
                &mut Machine::new(Some(1 << 20)),
                argument,
            );
            println!(
                "PROVIDER_END file={file} route={route} argument={argument}"
            );
        }
    }
}

#[test]
fn checked_host_member_providers_enforce_returned_u8() {
    external_provider_module_case(false);
}

#[test]
fn checked_file_member_providers_enforce_returned_u8() {
    external_provider_module_case(true);
}

#[test]
fn accepted_line_member_providers_enforce_returned_u8() {
    for (route, source) in PROVIDER_LINE_MAIN.iter().enumerate() {
        for argument in [3, 300] {
            let mut declarations = Declarations::new();
            declarations.module("host", PROVIDER_CALLBACKS).unwrap();
            let mut session = fipe::session::Session::new(declarations);
            let mut machine = Machine::new(Some(1 << 20));
            let Outcome::Done(setup) = session.line(PROVIDER_METHODS) else {
                panic!("setup admission")
            };
            assert!(setup.diagnostics.is_empty());
            assert!(setup.accepted.is_some());
            let program = compile::compile(&setup.unit.unwrap());
            assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
            let Outcome::Done(report) = session.line(source) else {
                panic!("caller admission")
            };
            assert!(report.diagnostics.is_empty());
            assert!(report.accepted.is_some());
            if route == 0 {
                assert_external_provider(
                    report.typed.as_ref().unwrap(),
                    &fipe::resolve::Origin::Line(0),
                    PROVIDER_METHODS,
                );
            }
            let program = compile::compile(&report.unit.unwrap());
            run_external_provider_case(&program, &mut machine, argument);
            println!("PROVIDER_LINE_END route={route} argument={argument}");
        }
    }
}

const PUBLIC_DEEP_HOST: &str = "pub type Box<T>: Ord { pub payload: Vec<T>, pub rank: Integer }\nimpl Box<a>: Eq { eq: Self -> (self: Self) -> Bool = other self => self.rank == other.rank }\nimpl Box<a>: Ord { cmp: Self -> (self: Self) -> Ordering = other self => self.rank.cmp other.rank }\npub relay: a -> a\npub items: [a; 2] -> Vec<a>\npub after: () -> Integer\n";
const PUBLIC_DEEP_SAVED: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => host.after ()\nmake_guard: Ord a => [a; 2] -> (Integer -> Sorted<a> -> Integer) = sample => host.relay keep<a>\nv0: Integer = 7\nv1 = [v0]\nv2 = [v1]\nv3 = [v2]\nv4 = [v3]\nv5 = [v4]\nv6 = [v5]\nv7 = [v6]\nv8 = [v7]\nv9 = [v8]\nv10 = [v9]\nv11 = [v10]\nv12 = [v11]\nv13 = [v12]\nv14 = [v13]\nv15 = [v14]\nv16 = [v15]\nv17 = [v16]\nv18 = [v17]\nv19 = [v18]\nv20 = [v19]\nv21 = [v20]\nv22 = [v21]\nv23 = [v22]\nv24 = [v23]\nv25 = [v24]\nv26 = [v25]\nv27 = [v26]\nv28 = [v27]\nv29 = [v28]\nv30 = [v29]\nv31 = [v30]\nv32 = [v31]\nv33 = [v32]\nv34 = [v33]\nv35 = [v34]\nv36 = [v35]\nv37 = [v36]\nv38 = [v37]\nv39 = [v38]\nv40 = [v39]\nv41 = [v40]\nv42 = [v41]\nv43 = [v42]\nv44 = [v43]\nv45 = [v44]\nv46 = [v45]\nv47 = [v46]\nv48 = [v47]\nv49 = [v48]\nv50 = [v49]\nv51 = [v50]\nv52 = [v51]\nv53 = [v52]\nv54 = [v53]\nv55 = [v54]\nv56 = [v55]\nv57 = [v56]\nv58 = [v57]\nsample = [host.Box [v58] 1, host.Box [v58] 2]\nmake_guard sample 7\n";
const PUBLIC_DEEP_ACTIVE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => host.after ()\nmake_guard: Ord a => [a; 2] -> (Integer -> Sorted<a> -> Integer) = sample => host.relay keep<a>\nv0: Integer = 7\nv1 = [v0]\nv2 = [v1]\nv3 = [v2]\nv4 = [v3]\nv5 = [v4]\nv6 = [v5]\nv7 = [v6]\nv8 = [v7]\nv9 = [v8]\nv10 = [v9]\nv11 = [v10]\nv12 = [v11]\nv13 = [v12]\nv14 = [v13]\nv15 = [v14]\nv16 = [v15]\nv17 = [v16]\nv18 = [v17]\nv19 = [v18]\nv20 = [v19]\nv21 = [v20]\nv22 = [v21]\nv23 = [v22]\nv24 = [v23]\nv25 = [v24]\nv26 = [v25]\nv27 = [v26]\nv28 = [v27]\nv29 = [v28]\nv30 = [v29]\nv31 = [v30]\nv32 = [v31]\nv33 = [v32]\nv34 = [v33]\nv35 = [v34]\nv36 = [v35]\nv37 = [v36]\nv38 = [v37]\nv39 = [v38]\nv40 = [v39]\nv41 = [v40]\nv42 = [v41]\nv43 = [v42]\nv44 = [v43]\nv45 = [v44]\nv46 = [v45]\nv47 = [v46]\nv48 = [v47]\nv49 = [v48]\nv50 = [v49]\nv51 = [v50]\nv52 = [v51]\nv53 = [v52]\nv54 = [v53]\nv55 = [v54]\nv56 = [v55]\nv57 = [v56]\nv58 = [v57]\nsample = [host.Box [v58] 1, host.Box [v58] 2]\nmake_guard sample 7 (host.items sample)\n";

struct PublicDeepGuard {
    association: std::rc::Weak<[(usize, std::rc::Rc<fipe::value::Meaning>)]>,
    roots: Vec<std::rc::Weak<fipe::value::Meaning>>,
    test: usize,
    test_body: usize,
    keep_body: usize,
}

fn public_deep_type(ty: &fipe::ir::Type, tables: &fipe::ir::Tables) {
    use fipe::ir::Type;
    let Type::Meaning { index, arguments } = ty else {
        panic!("captured root must be nominal Box")
    };
    assert_eq!(tables.namespaces[tables.meanings[*index].namespace], "host");
    assert_eq!(tables.meanings[*index].label, "Box");
    let [inner] = arguments.as_slice() else {
        panic!("Box has exactly one actual argument")
    };
    let mut inner = inner;
    for depth in 0..58 {
        let Type::Array { element, length } = inner else {
            panic!("expected source Array layer {depth}")
        };
        assert!(
            matches!(length.as_ref(), Type::Number(n) if n == &1u64.into())
        );
        inner = element;
    }
    let Type::Meaning { index, arguments } = inner else {
        panic!("Array58 must terminate in nominal Integer")
    };
    assert_eq!(
        tables.namespaces[tables.meanings[*index].namespace],
        "prelude"
    );
    assert_eq!(tables.meanings[*index].label, "Integer");
    assert!(arguments.is_empty());
}

fn public_deep_creator(module: &fipe::ast::Module) -> fipe::token::Span {
    use fipe::ast::{Entry, ExprKind, PatternKind};
    let sites: Vec<_> = module
        .entries
        .iter()
        .filter_map(|entry| {
            let Entry::Line(id) = entry else {
                return None;
            };
            let ExprKind::Bind { pattern, .. } = module.expr(*id).unwrap().kind
            else {
                return None;
            };
            let PatternKind::Name(name) =
                &module.pattern(pattern).unwrap().kind
            else {
                return None;
            };
            (name
                .segments
                .last()
                .is_some_and(|name| name == "make_guard"))
            .then_some(name.span)
        })
        .collect();
    let [site] = sites.as_slice() else {
        panic!("one actual make_guard binding");
    };
    *site
}

fn public_deep_capture(
    machine: &Machine,
    arguments: &[fipe::value::Value],
    source: &str,
    creator_site: fipe::token::Span,
) -> PublicDeepGuard {
    use fipe::{value::Value, vm::Frame};
    let [Value::Function(function)] = arguments else {
        panic!("relay receives actual guarded callable");
    };
    let program = machine.program().unwrap();
    let (test, test_body) = sorted_runtime_test_identity(program, source);
    let tables = program.tables.borrow();
    let keep = &program.bodies[function.body];
    let node = &tables.nodes[keep.node];
    assert_eq!(tables.namespaces[node.namespace], "main");
    assert_eq!(
        source.get(node.span.start..node.span.end),
        Some("n xs => host.after ()")
    );
    assert_eq!(keep.arity, 2);
    assert!(function.filled.is_empty());
    let guards: Vec<_> = function
        .guards
        .iter()
        .filter_map(|guard| {
            fully_applied_partial_test_parameters(&guard.predicate, test)
                .map(|ids| (guard, ids))
        })
        .collect();
    let [(guard, ids)] = guards.as_slice() else {
        panic!("one source-linked Sorted Guard");
    };
    assert_eq!(guard.spot, 1);
    assert_eq!(ids.len(), 1);
    assert_eq!(guard.witnesses.len(), 1);
    let creators: Vec<_> = tables
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(index, definition)| {
            (definition.span == creator_site
                && tables.namespaces[definition.namespace] == "main")
                .then_some(index)
        })
        .collect();
    let [creator] = creators.as_slice() else {
        panic!("exact creator source owns one definition");
    };
    let creator = program.definitions[*creator].unwrap();
    let frames: Vec<_> = machine
        .frames()
        .iter()
        .filter_map(|frame| match frame {
            Frame::Body(frame) if frame.body == creator => Some(frame),
            _ => None,
        })
        .collect();
    let [frame] = frames.as_slice() else {
        panic!("actual creator frame at relay");
    };
    assert_eq!(program.bodies[creator].hidden.len(), frame.meanings.len());
    let mut roots = Vec::new();
    for id in ids {
        let position = program.bodies[creator]
            .hidden
            .iter()
            .position(|known| known == id)
            .unwrap();
        public_deep_type(&frame.meanings[position], &tables);
        let captured: Vec<_> = guard
            .witnesses
            .iter()
            .filter(|(known, _)| known == id)
            .collect();
        let [(_, meaning)] = captured.as_slice() else {
            panic!("exact demanded creator association");
        };
        public_deep_type(meaning.as_type(), &tables);
        roots.push(std::rc::Rc::downgrade(meaning));
    }
    PublicDeepGuard {
        association: std::rc::Rc::downgrade(&guard.witnesses),
        roots,
        test,
        test_body,
        keep_body: function.body,
    }
}

fn public_deep_test_started(
    machine: &Machine,
    observed: &PublicDeepGuard,
) -> bool {
    use fipe::vm::Frame;
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return false;
    };
    if frame.body != observed.test_body {
        return false;
    }
    let program = machine.program().unwrap();
    assert_eq!(program.tests[observed.test], Some(frame.body));
    assert_eq!(program.bodies[frame.body].hidden.len(), 1);
    assert_eq!(frame.meanings.len(), 1);
    public_deep_type(&frame.meanings[0], &program.tables.borrow());
    assert!(observed.association.strong_count() > 0);
    assert!(observed.roots.iter().all(|root| root.strong_count() > 0));
    true
}

fn public_deep_dead(observed: &PublicDeepGuard) {
    assert!(
        observed.association.upgrade().is_none(),
        "captured association retained"
    );
    for root in &observed.roots {
        assert!(root.upgrade().is_none(), "captured root retained");
    }
}

fn public_deep_saved(value: &fipe::value::Value, observed: &PublicDeepGuard) {
    use fipe::value::Value;
    let Value::Function(function) = value else {
        panic!("saved actual partial");
    };
    assert_eq!(function.body, observed.keep_body);
    assert_eq!(function.filled, [Value::Int(7)]);
    let guards: Vec<_> = function
        .guards
        .iter()
        .filter(|guard| {
            fully_applied_partial_test_parameters(
                &guard.predicate,
                observed.test,
            )
            .is_some()
        })
        .collect();
    let [guard] = guards.as_slice() else {
        panic!("saved semantic Sorted Guard");
    };
    assert_eq!(guard.spot, 1);
    assert!(std::ptr::eq(
        std::rc::Rc::as_ptr(&guard.witnesses),
        observed.association.as_ptr()
    ));
}

fn public_deep_finish_saved(
    value: fipe::value::Value,
    observed: &PublicDeepGuard,
    callbacks: &[String],
) {
    assert_eq!(callbacks, ["relay"]);
    public_deep_saved(&value, observed);
    assert_eq!(
        observed.association.strong_count(),
        1,
        "only returned Guard owns association"
    );
    for root in &observed.roots {
        assert_eq!(
            root.strong_count(),
            1,
            "only association owns demanded root"
        );
    }
    eprintln!("PUBLIC_DEEP FINAL_OWNER_READY DROP_BEGIN");
    drop(value);
    public_deep_dead(observed);
    eprintln!("PUBLIC_DEEP DROP_END");
}

fn public_deep_setup(
    source: &str,
) -> (
    fipe::ast::Module,
    Declarations,
    fipe::ir::Unit,
    fipe::mir::Program,
) {
    let mut declarations = Declarations::new();
    declarations.module("host", PUBLIC_DEEP_HOST).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit = check::check(&module, &declarations, &[], check::Options::default()).unwrap_or_else(|errors| {
        for error in &errors {
            match error {
                check::Error::Type(error) => eprintln!("PUBLIC_DEEP CHECK_ERROR span={:?} TypeDepth={} reason_variant={:?}", error.span, matches!(error.reason, fipe::infer::Reason::TypeDepth), std::mem::discriminant(&error.reason)),
                other => eprintln!("PUBLIC_DEEP CHECK_ERROR variant={:?}", std::mem::discriminant(other)),
            }
        }
        panic!("public58 admission failed; preserve fixture, do not shorten")
    });
    let program = compile::compile(&unit);
    eprintln!("PUBLIC_DEEP CHECKED");
    (module, declarations, unit, program)
}

fn public_deep_reply(
    machine: &Machine,
    request: fipe::embed::Request,
    source: &str,
    creator_site: fipe::token::Span,
    observed: &mut Option<PublicDeepGuard>,
    callbacks: &mut Vec<String>,
    test_seen: bool,
) -> fipe::value::Value {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("only declared callback");
    };
    assert!(environment.is_empty());
    let (namespace, label) = {
        let tables = machine.program().unwrap().tables.borrow();
        let declaration = &tables.declarations[declaration];
        (
            tables.namespaces[declaration.namespace].clone(),
            declaration.label.clone(),
        )
    };
    assert_eq!(namespace, "host");
    callbacks.push(label.clone());
    match label.as_str() {
        "relay" => {
            assert_eq!(callbacks, &["relay"]);
            assert!(observed.is_none());
            *observed = Some(public_deep_capture(
                machine,
                &arguments,
                source,
                creator_site,
            ));
            eprintln!("PUBLIC_DEEP CAPTURED");
            arguments.into_iter().next().unwrap()
        }
        "items" => {
            assert_eq!(callbacks, &["relay", "items"]);
            let [Value::Seq(sample)] = arguments.as_slice() else {
                panic!("actual source Array sample");
            };
            assert_eq!(sample.len(), 2);
            for (rank, value) in sample.iter().enumerate() {
                let Value::Record(record) = value else {
                    panic!("actual Box sample record");
                };
                let tables = machine.program().unwrap().tables.borrow();
                assert_eq!(tables.meanings[record.meaning].label, "Box");
                assert_eq!(
                    tables.namespaces
                        [tables.meanings[record.meaning].namespace],
                    "host"
                );
                assert_eq!(
                    record.fields[1],
                    Value::Int(i64::try_from(rank + 1).unwrap())
                );
            }
            arguments.into_iter().next().unwrap()
        }
        "after" => {
            assert!(test_seen, "actual semantic Test must execute");
            assert_eq!(callbacks, &["relay", "items", "after"]);
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected public deep effect"),
    }
}

fn public_deep_run(mode: &'static str) {
    use fipe::{embed::Reply, value::Value, vm::Stop};
    let source = if mode == "saved" {
        PUBLIC_DEEP_SAVED
    } else {
        PUBLIC_DEEP_ACTIVE
    };
    let (module, declarations, unit, program) = public_deep_setup(source);
    let creator_site = public_deep_creator(&module);
    let mut machine = Machine::new(None);
    machine.begin(&program);
    let mut observed = None;
    let mut callbacks = Vec::new();
    let mut test_seen = false;
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(stop, Stop::Before(_)) {
            if let Some(guard) = &observed {
                if public_deep_test_started(&machine, guard) {
                    test_seen = true;
                    if mode == "abandon" {
                        eprintln!("PUBLIC_DEEP TEST_STARTED DROP_BEGIN");
                        drop(stop);
                        assert_eq!(callbacks, ["relay", "items"]);
                        let charged = machine.charged();
                        assert!(machine.abandon());
                        assert!(machine.frames().is_empty());
                        assert_eq!(machine.charged(), charged);
                        public_deep_dead(guard);
                        assert!(!machine.abandon());
                        eprintln!("PUBLIC_DEEP DROP_END");
                        return;
                    }
                }
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = public_deep_reply(
                    &machine,
                    request,
                    source,
                    creator_site,
                    &mut observed,
                    &mut callbacks,
                    test_seen,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                let value = result.unwrap_or_else(|fault| panic!("public deep runtime fault namespace={} span={:?} reason_variant={:?}", fault.namespace, fault.span, std::mem::discriminant(&fault.reason)));
                let observed = observed.unwrap();
                if mode == "saved" {
                    assert!(!test_seen);
                    public_deep_saved(&value, &observed);
                    drop(machine);
                    drop(program);
                    drop(unit);
                    drop(module);
                    drop(declarations);
                    public_deep_finish_saved(value, &observed, &callbacks);
                } else {
                    assert_eq!(mode, "finish");
                    assert!(test_seen);
                    assert_eq!(value, Value::Int(7));
                    assert_eq!(callbacks, ["relay", "items", "after"]);
                    drop(value);
                    assert!(!machine.abandon());
                    public_deep_dead(&observed);
                    eprintln!("PUBLIC_DEEP HEALTHY_DROP_END");
                }
                return;
            }
        }
    }
    panic!("bounded public deep driver did not terminate");
}

fn public_deep_probe(mode: &'static str, selector: &'static str) {
    const CHILD: &str = "FIPE_PUBLIC_PREDICATE_DEEP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut passed = true;
        for stack in ["normal", "small"] {
            let mut child =
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        selector,
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env(CHILD, stack)
                    .spawn()
                    .unwrap();
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(15);
            let (status, timed_out) = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break (status, false);
                }
                if std::time::Instant::now() >= deadline {
                    let killed = child.kill();
                    let status = child.wait().unwrap();
                    eprintln!(
                        "PUBLIC_DEEP child deadline reached; kill={killed:?}"
                    );
                    break (status, true);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            eprintln!("PUBLIC_DEEP mode={mode} stack={stack} status={status} timed_out={timed_out}");
            passed &= !timed_out && status.success();
        }
        assert!(passed, "public58 child failure; preserve reached phase");
        return;
    }
    let stack = if std::env::var(CHILD).unwrap() == "normal" {
        8 * 1024 * 1024
    } else {
        512 * 1024
    };
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || public_deep_run(mode))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn public_deep_guard_saved_partial_is_the_last_captured_root_owner() {
    public_deep_probe("saved", "entries::public_deep_guard_saved_partial_is_the_last_captured_root_owner");
}
#[test]
fn public_deep_active_predicate_abandon_releases_machine_owned_roots() {
    public_deep_probe("abandon", "entries::public_deep_active_predicate_abandon_releases_machine_owned_roots");
}
#[test]
fn public_deep_healthy_predicate_finish_releases_machine_owned_roots() {
    public_deep_probe("finish", "entries::public_deep_healthy_predicate_finish_releases_machine_owned_roots");
}

const BODYLESS_LABELED_METHODS: &str =
    "pub type M { pub n: Integer }\nimpl M { op: U8 -> (self: Self) -> U8 }\n";
const PROVIDER_ALIAS_METHODS: &str = "pub k: Integer -> U8 -> U8 = n => (x => x)\npub alias: Integer -> U8 -> U8 = k\n";
const PROVIDER_ALIAS_MAIN: &str = "use host\nuse methods\nget = n => methods.alias n\nf = get 7\nhost.after ()\nf (host.arg ())\n";
const RETURNED_DEFAULT_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: (self: Self) -> (x: Integer = 3) -> Integer = self => ((x = 3) => self.n + x) }\n";
const RETURNED_DEFAULT_MAIN: &str = "use methods\nf = (methods.M 7).op\nf 4\n";

fn compatibility_call_reply(
    machine: &Machine,
    request: &fipe::embed::Request,
    argument: i64,
) -> (String, super::Value) {
    use fipe::{embed::Request, value::Value};
    let Request::Call {
        declaration,
        arguments,
        environment,
    } = request
    else {
        panic!("actual callback")
    };
    assert!(environment.is_empty());
    let tables = machine.program().unwrap().tables.borrow();
    let declaration = &tables.declarations[*declaration];
    let namespace = &tables.namespaces[declaration.namespace];
    if namespace == "host" {
        assert_eq!(arguments, &[Value::Unit]);
        let value = match declaration.label.as_str() {
            "after" => 7,
            "arg" => argument,
            _ => panic!("unknown callback"),
        };
        return (declaration.label.clone(), Value::Int(value));
    }
    assert_eq!(namespace, "methods");
    assert_eq!(declaration.label, "op");
    let [Value::Int(operand), Value::Record(receiver)] = arguments.as_slice()
    else {
        panic!("two real declared spots")
    };
    assert_eq!(tables.meanings[receiver.meaning].label, "M");
    assert_eq!(
        tables.namespaces[tables.meanings[receiver.meaning].namespace],
        "methods"
    );
    assert_eq!(receiver.fields, [Value::Int(7)]);
    assert_eq!(*operand, 3);
    ("op".into(), Value::Int(*operand))
}

fn compatibility_run(
    program: &fipe::mir::Program,
    machine: &mut Machine,
    argument: i64,
    limit: &str,
    bodyless: bool,
) {
    use fipe::{embed::Reply, value::Value};
    let mut outcome = machine.run(program);
    let mut calls = Vec::new();
    for _ in 0..8 {
        match outcome {
            Outcome::Request(ref request) => {
                let (label, value) =
                    compatibility_call_reply(machine, request, argument);
                calls.push(label);
                outcome = machine.reply(program, Reply::Value(Ok(value)));
            }
            Outcome::Done(result) => {
                let expected = if bodyless && argument == 3 {
                    vec!["after", "arg", "op"]
                } else {
                    vec!["after", "arg"]
                };
                assert_eq!(calls, expected);
                if argument == 3 {
                    assert_eq!(result.unwrap(), Value::Int(3));
                } else {
                    let fault = result.expect_err("declared limit must reject");
                    let Reason::Failed { entry, value } = fault.reason else {
                        panic!("semantic limit refusal")
                    };
                    assert_eq!(value, argument.to_string());
                    assert_eq!(
                        machine.program().unwrap().tables.borrow().entries
                            [entry]
                            .label,
                        limit
                    );
                }
                return;
            }
        }
    }
    panic!("bounded callback sequence must finish");
}

fn assert_bodyless_dispatch_identity(
    typed: &fipe::typed::Typed,
    program: &fipe::mir::Program,
) {
    use fipe::{resolve::Origin, typed::Selected};
    let read = PROVIDER_MAIN[0].find("m.op").unwrap();
    assert!(matches!(typed.selected.get(&(read, read + 4)),
        Some(Selected::Member { name, .. }) if name == "op"));
    let start = BODYLESS_LABELED_METHODS.find("op:").unwrap();
    let rows: Vec<_> = typed.dispatch.iter().filter(|dispatch| dispatch.name == "op")
        .flat_map(|dispatch| &dispatch.rows)
        .filter(|row| matches!(&row.selected, Selected::Declared { origin: Origin::Host(name), site }
            if name == "methods" && site.start == start && site.end == start + 2)).collect();
    assert_eq!(rows.len(), 1, "exact selected bodyless method dispatch row");
    assert_eq!(rows[0].receiver_spot, Some(1));
    assert!(
        matches!(&rows[0].receiver, fipe::types::Type::Named { name, arguments }
        if name.origin == Origin::Host("methods".into()) && name.name == "M" && arguments.is_empty())
    );
    let tables = program.tables.borrow();
    let rows: Vec<_> = tables
        .dispatch
        .iter()
        .flat_map(|dispatch| &dispatch.rows)
        .filter_map(|row| {
            let fipe::ir::Implementation::Declared(index) = &row.target else {
                return None;
            };
            let declaration = &tables.declarations[*index];
            (tables.namespaces[declaration.namespace] == "methods"
                && declaration.label == "op")
                .then_some((row, declaration))
        })
        .collect();
    assert_eq!(rows.len(), 1, "corresponding compiled declared operation");
    let (row, declaration) = rows[0];
    assert_eq!(row.receiver_spot, Some(1));
    let mut ty = &declaration.ty;
    let mut parameters = 0;
    while let fipe::ir::Type::Function { result, .. } = ty {
        parameters += 1;
        ty = result;
    }
    assert_eq!(
        parameters, 2,
        "bodyless callback has two declared parameters"
    );
    assert!(!matches!(ty, fipe::ir::Type::Function { .. }));
}

#[test]
fn labeled_bodyless_member_keeps_two_supplied_spots_without_written_provider() {
    for argument in [3, 300] {
        let mut declarations = Declarations::new();
        declarations
            .module("methods", BODYLESS_LABELED_METHODS)
            .unwrap();
        declarations.module("host", PROVIDER_CALLBACKS).unwrap();
        let module = parse(lex(PROVIDER_MAIN[0])).unwrap();
        let resolved =
            fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed =
            fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
        assert!(typed.member_contracts.is_empty());
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        assert_bodyless_dispatch_identity(&typed, &program);
        compatibility_run(
            &program,
            &mut Machine::new(Some(1 << 20)),
            argument,
            "U8",
            true,
        );
    }
}

#[test]
fn annotated_external_alias_keeps_full_declared_signature() {
    for argument in [3, 300] {
        let mut declarations = Declarations::new();
        declarations
            .module("methods", PROVIDER_ALIAS_METHODS)
            .unwrap();
        declarations.module("host", PROVIDER_CALLBACKS).unwrap();
        let module = parse(lex(PROVIDER_ALIAS_MAIN)).unwrap();
        let resolved =
            fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed =
            fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
        assert!(typed.member_contracts.is_empty());
        let start = PROVIDER_ALIAS_METHODS.find("alias:").unwrap();
        assert!(typed.selected.iter().any(|(stage, selected)| matches!(selected,
            fipe::typed::Selected::Value { origin: fipe::resolve::Origin::Host(name), site }
            if name == "methods" && site.start == start && site.end == start + 5)
            && typed.bound.get(stage).is_some_and(|bound| bound.spots.len() == 2)));
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        compatibility_run(
            &compile::compile(&unit),
            &mut Machine::new(Some(1 << 20)),
            argument,
            "U8",
            false,
        );
    }
}

#[test]
fn returned_labeled_default_stays_outside_the_written_outer_boundary() {
    let mut declarations = Declarations::new();
    declarations
        .module("methods", RETURNED_DEFAULT_METHODS)
        .unwrap();
    let module = parse(lex(RETURNED_DEFAULT_MAIN)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let start = RETURNED_DEFAULT_METHODS.find("op:").unwrap();
    let bounds: Vec<_> = typed.selected.iter().filter_map(|(stage, selected)| {
        matches!(selected, fipe::typed::Selected::Value { origin: fipe::resolve::Origin::Host(name), site }
            if name == "methods" && site.start == start && site.end == start + 2)
            .then(|| typed.bound.get(stage)).flatten()
    }).collect();
    assert!(!bounds.is_empty());
    for bound in bounds {
        assert_eq!(bound.spots.len(), 1);
        assert!(bound.defaults.is_empty());
        assert!(bound.supplies.iter().any(|(spot, supply)| *spot == 0
            && matches!(supply, fipe::typed::Supply::Head(_))));
    }
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let tables = program.tables.borrow();
    let definitions: Vec<_> = tables
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(index, definition)| {
            (tables.namespaces[definition.namespace] == "methods"
                && definition.span.start == start
                && definition.span.end == start + 2)
                .then_some(index)
        })
        .collect();
    assert_eq!(definitions.len(), 1);
    assert_eq!(
        program.bodies[program.definitions[definitions[0]].unwrap()].arity,
        1
    );
    drop(tables);
    let mut machine = Machine::new(Some(1 << 20));
    assert!(matches!(
        machine.run(&program),
        Outcome::Done(Ok(super::Value::Int(11)))
    ));
    let tables = machine.program().unwrap().tables.borrow();
    let saved: Vec<_> = machine
        .globals()
        .filter_map(|(index, value)| {
            (tables.definitions[index].label == "f").then_some(value)
        })
        .collect();
    assert_eq!(saved.len(), 1);
    let super::Value::Function(function) = saved[0] else {
        panic!("actual returned function")
    };
    assert_eq!(machine.program().unwrap().bodies[function.body].arity, 1);
}

#[test]
fn withdrawn_line_provider_does_not_reuse_old_same_site_contract() {
    let mut declarations = Declarations::new();
    declarations.module("host", PROVIDER_CALLBACKS).unwrap();
    let mut session = fipe::session::Session::new(declarations);
    let mut machine = Machine::new(Some(1 << 20));
    let Outcome::Done(setup) = session.line(PROVIDER_METHODS) else {
        panic!("provider admission")
    };
    assert!(setup.diagnostics.is_empty());
    let token = setup.accepted.unwrap();
    let program = compile::compile(setup.unit.as_ref().unwrap());
    assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
    let before: Vec<_> = session.lines().map(str::to_owned).collect();
    let Outcome::Done(query) = session.query(PROVIDER_LINE_MAIN[0]) else {
        panic!("query admission")
    };
    assert!(query.diagnostics.is_empty());
    assert!(query.accepted.is_none());
    assert_external_provider(
        query.typed.as_ref().unwrap(),
        &fipe::resolve::Origin::Line(0),
        PROVIDER_METHODS,
    );
    assert_eq!(
        session.lines().map(str::to_owned).collect::<Vec<_>>(),
        before
    );
    machine.withdraw(
        &session
            .withdraw(token)
            .expect("latest provider is coherently withdrawn"),
    );
    let replacement = PROVIDER_METHODS.replace("U8", "I8");
    assert_eq!(replacement.len(), PROVIDER_METHODS.len());
    let Outcome::Done(setup) = session.line(&replacement) else {
        panic!("replacement admission")
    };
    assert!(setup.diagnostics.is_empty());
    let program = compile::compile(setup.unit.as_ref().unwrap());
    assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
    let Outcome::Done(report) = session.line(PROVIDER_LINE_MAIN[0]) else {
        panic!("replacement caller admission")
    };
    assert!(report.diagnostics.is_empty());
    let typed = report.typed.as_ref().unwrap();
    let contracts: Vec<_> = typed.member_contracts.values().collect();
    assert!(!contracts.is_empty());
    let start = replacement.find("op:").unwrap();
    for contract in contracts {
        assert!(
            matches!(&contract.selected, fipe::typed::Selected::Value { origin: fipe::resolve::Origin::Line(0), site }
            if site.start == start && site.end == start + 2)
        );
        let fipe::types::Type::Function { parameter, result } =
            &contract.callable
        else {
            panic!("replacement callable")
        };
        for ty in [parameter.as_ref(), result.as_ref()] {
            assert!(matches!(ty, fipe::types::Type::Limited { name, .. }
                if matches!(name.as_ref(), fipe::types::Type::Named { name, .. }
                    if name.origin == fipe::resolve::Origin::Prelude && name.name == "I8")));
        }
    }
    compatibility_run(
        &compile::compile(report.unit.as_ref().unwrap()),
        &mut machine,
        200,
        "I8",
        false,
    );
}

const DECLARED_SORTED_METHODS: &str = "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\npub type Sorted<T: Ord>: Vec<T> where is_sorted self\npub type M<T: Ord> { pub token: T }\nimpl M { op: Sorted<T> -> (self: Self) -> Integer }\n";

fn declared_sorted_witness(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    test: usize,
    meaning: &str,
) {
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let parameter = tables.tests[test].parameters[0];
    let position = joined.bodies[frame.body]
        .hidden
        .iter()
        .position(|id| *id == parameter)
        .unwrap();
    let fipe::ir::Type::Meaning { index, arguments } =
        &frame.meanings[position]
    else {
        panic!("real predicate type witness")
    };
    assert!(arguments.is_empty());
    assert_eq!(
        tables.namespaces[tables.meanings[*index].namespace],
        "prelude"
    );
    assert_eq!(tables.meanings[*index].label, meaning);
}

fn declared_sorted_operation(
    machine: &Machine,
    arguments: &[super::Value],
    items: &[super::Value],
    meaning: &str,
) {
    use fipe::value::Value;
    assert_eq!(arguments.len(), 2);
    assert_eq!(arguments[0], Value::seq(items.to_vec()));
    let Value::Record(receiver) = &arguments[1] else {
        panic!("raw self is last")
    };
    let tables = machine.program().unwrap().tables.borrow();
    assert_eq!(
        tables.namespaces[tables.meanings[receiver.meaning].namespace],
        "methods"
    );
    assert_eq!(tables.meanings[receiver.meaning].label, "M");
    assert_eq!(
        receiver.fields,
        [if meaning == "Integer" {
            Value::Int(7)
        } else {
            Value::text("tag")
        }]
    );
}

fn declared_sorted_result(
    machine: &Machine,
    good: bool,
    tested: bool,
    test: usize,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
) {
    use fipe::value::Value;
    assert!(
        tested,
        "actual Sorted test executes with its real T witness"
    );
    if good {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert_eq!(calls, ["after", "arg", "op"]);
    } else {
        let fault = result.unwrap_err();
        let Reason::Failed { entry, .. } = fault.reason else {
            panic!("semantic Sorted refusal")
        };
        let tables = machine.program().unwrap().tables.borrow();
        assert!(sorted_predicate_contains_test(
            &tables.entries[entry].predicate,
            test
        ));
        assert_eq!(calls, ["after", "arg"]);
    }
}

fn declared_sorted_test(machine: &Machine) -> (usize, usize) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        let tests: Vec<_> = tables
            .tests
            .iter()
            .enumerate()
            .filter_map(|(index, test)| {
                (tables.namespaces[test.namespace] == "methods"
                    && DECLARED_SORTED_METHODS
                        .get(test.span.start..test.span.end)
                        == Some("is_sorted self"))
                .then_some(index)
            })
            .collect();
        assert_eq!(tests.len(), 1);
        (
            tests[0],
            joined.tests[tests[0]].expect("actual source Test body reached"),
        )
    }
}

fn declared_row_sorted_case(
    marker: &str,
    items: &[super::Value],
    good: bool,
    meaning: &str,
) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::{Frame, Stop},
    };
    let mut declarations = Declarations::new();
    declarations
        .module("methods", DECLARED_SORTED_METHODS)
        .unwrap();
    declarations
        .module(
            "host",
            &format!(
                "pub after: () -> Integer\npub arg: () -> Vec<{meaning}>\n"
            ),
        )
        .unwrap();
    let source = format!("use host\nuse methods\nget = m => m.op\nf = get (methods.M {marker})\nhost.after ()\nf (host.arg ())\n");
    let module = parse(lex(&source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let (test, body) = declared_sorted_test(&machine);
    let mut tested = false;
    let mut calls = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if let Some(Frame::Body(frame)) = machine.frames().last() {
            if frame.body == body {
                declared_sorted_witness(&machine, frame, test, meaning);
                tested = true;
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let (namespace, label) = {
                    let tables = machine.program().unwrap().tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    (
                        tables.namespaces[declaration.namespace].clone(),
                        declaration.label.clone(),
                    )
                };
                let reply = if namespace == "host" {
                    assert_eq!(arguments, [Value::Unit]);
                    match label.as_str() {
                        "after" => Value::Int(7),
                        "arg" => Value::seq(items.to_vec()),
                        _ => panic!("callback"),
                    }
                } else {
                    assert!(
                        good,
                        "refuted input cannot cross methods.op boundary"
                    );
                    assert!(tested);
                    assert_eq!(namespace, "methods");
                    assert_eq!(label, "op");
                    declared_sorted_operation(
                        &machine, &arguments, items, meaning,
                    );
                    Value::Int(7)
                };
                calls.push(label);
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                declared_sorted_result(
                    &machine, good, tested, test, result, &calls,
                );
                return;
            }
            Stop::Request(_) => panic!("unexpected request"),
        }
    }
    panic!("bounded stepped Test must finish");
}

fn sorted_predicate_contains_test(
    predicate: &fipe::ir::Predicate,
    test: usize,
) -> bool {
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            fipe::ir::Predicate::All(parts)
            | fipe::ir::Predicate::Any(parts) => pending.extend(parts),
            fipe::ir::Predicate::Not(inner) => pending.push(inner),
            fipe::ir::Predicate::Function { parameter, result } => {
                pending.extend([parameter.as_ref(), result.as_ref()]);
            }
            fipe::ir::Predicate::Test { index, .. } if *index == test => {
                return true
            }
            _ => {}
        }
    }
    false
}

#[test]
fn declared_row_sorted_input_uses_integer_and_string_witnesses() {
    use super::Value;
    for good in [true, false] {
        declared_row_sorted_case(
            "7",
            &if good {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            },
            good,
            "Integer",
        );
        declared_row_sorted_case(
            "\"tag\"",
            &if good {
                vec![Value::text("a"), Value::text("b")]
            } else {
                vec![Value::text("b"), Value::text("a")]
            },
            good,
            "String",
        );
    }
}

#[test]
fn declared_row_unbound_predicate_witness_faults_before_host_effects() {
    let mut declarations = Declarations::new();
    declarations.module("methods", "pub type Tagged<T>: Integer where self < 10\npub type M { pub token: Integer }\nimpl M { op: Tagged<a> -> (self: Self) -> Integer }\n").unwrap();
    declarations.module("host", PROVIDER_CALLBACKS).unwrap();
    let module = parse(lex(PROVIDER_MAIN[0])).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    let Outcome::Done(Err(fault)) = machine.run(&program) else {
        panic!("missing actual witness must fail before any callback")
    };
    assert_eq!(
        fault.reason,
        Reason::Host("missing predicate witness".into())
    );
}

fn declared_scalar_row_case(
    header: &str,
    source: &str,
    argument: i64,
    good: bool,
    limit: &str,
    receiver_name: &str,
) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let mut declarations = Declarations::new();
    declarations.module("methods", header).unwrap();
    declarations.module("host", PROVIDER_CALLBACKS).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    let mut outcome = machine.run(&program);
    let mut calls = Vec::new();
    for _ in 0..8 {
        match outcome {
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let tables = machine.program().unwrap().tables.borrow();
                let declaration = &tables.declarations[declaration];
                let label = declaration.label.clone();
                let reply =
                    if tables.namespaces[declaration.namespace] == "host" {
                        assert_eq!(arguments, [Value::Unit]);
                        match label.as_str() {
                            "after" => 7,
                            "arg" => argument,
                            _ => panic!("callback"),
                        }
                    } else {
                        assert!(
                            good,
                            "refinement gate runs before selected Host effect"
                        );
                        assert_eq!(label, "op");
                        let [Value::Int(value), Value::Record(receiver)] =
                            arguments.as_slice()
                        else {
                            panic!("raw declared inputs")
                        };
                        assert_eq!(*value, argument);
                        assert_eq!(
                            tables.meanings[receiver.meaning].label,
                            receiver_name
                        );
                        7
                    };
                drop(tables);
                calls.push(label);
                outcome = machine
                    .reply(&program, Reply::Value(Ok(Value::Int(reply))));
            }
            Outcome::Done(result) => {
                if good {
                    assert_eq!(result.unwrap(), Value::Int(7));
                    assert_eq!(calls, ["after", "arg", "op"]);
                } else {
                    let fault = result.unwrap_err();
                    let Reason::Failed { entry, .. } = fault.reason else {
                        panic!("selected semantic input limit")
                    };
                    assert!(machine.program().unwrap().tables.borrow().entries
                        [entry]
                        .label
                        .contains(limit));
                    assert_eq!(calls, ["after", "arg"]);
                }
                return;
            }
            Outcome::Request(_) => panic!("unexpected request"),
        }
    }
    panic!("bounded scalar row must finish");
}

#[test]
fn declared_row_input_uses_selected_non_u8_predicate() {
    const METHODS: &str = "pub type Tagged<T>: Integer where self < 10\npub type M<T> { pub token: T }\nimpl M { op: Tagged<T> -> (self: Self) -> Integer }\n";
    const ROWS: &str = "pub type A { pub n: Integer }\npub type B { pub n: Integer }\nimpl A { op: U8 -> (self: Self) -> Integer }\nimpl B { op: I8 -> (self: Self) -> Integer }\n";
    for (argument, good) in [(3, true), (11, false)] {
        declared_scalar_row_case(
            METHODS,
            PROVIDER_MAIN[0],
            argument,
            good,
            "Tagged",
            "M",
        );
    }
    for (receiver, good, limit) in [("a", true, "U8"), ("b", false, "I8")] {
        let source = format!("use host\nuse methods\nget = m => m.op\na = methods.A 7\nb = methods.B 7\nf = get {receiver}\nhost.after ()\nf (host.arg ())\n");
        declared_scalar_row_case(
            ROWS,
            &source,
            200,
            good,
            limit,
            if good { "A" } else { "B" },
        );
    }
}

fn assert_declared_partial_call(program: &fipe::mir::Program, source: &str) {
    let site = source.find("f (host.arg ())").unwrap();
    assert!(
        program
            .bodies
            .iter()
            .any(|body| body.blocks.iter().any(|block| {
                let fipe::mir::Terminator::Call {
                    callee: fipe::mir::Callee::Value(_),
                    args,
                    ..
                } = &block.terminator
                else {
                    return false;
                };
                let tables = program.tables.borrow();
                args.len() == 1
                    && block.nodes.last().is_some_and(|node| {
                        let node = &tables.nodes[*node];
                        tables.namespaces[node.namespace] == "main"
                            && node.span.start == site
                            && node.span.end == site + "f (host.arg ())".len()
                    })
            })),
        "actual source-owned closed Value call supplies only one argument"
    );
}

fn assert_declared_saved_partial(
    joined: &fipe::mir::Program,
    partial: &fipe::value::Closure,
) {
    use fipe::value::Value;
    assert!(joined
        .receiver_adapters
        .iter()
        .any(|adapter| adapter.body == partial.body));
    assert_eq!(joined.bodies[partial.body].arity, 3);
    assert_eq!(
        partial.filled.len(),
        2,
        "same adapter, not an open-spot wrapper"
    );
    assert!(
        matches!(&partial.filled[0], Value::Record(receiver) if receiver.fields == [Value::Int(7)])
    );
    assert_eq!(partial.filled[1], Value::Int(3));
}

fn assert_declared_partial_result(
    machine: &Machine,
    argument: i64,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
) {
    use fipe::value::Value;
    if argument == 3 {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert_eq!(calls, ["after", "arg", "saved", "op"]);
    } else {
        let fault = result.unwrap_err();
        let Reason::Failed { entry, value } = fault.reason else {
            panic!("U8 partial refusal")
        };
        assert_eq!(value, "300");
        assert_eq!(
            machine.program().unwrap().tables.borrow().entries[entry].label,
            "U8"
        );
        assert_eq!(calls, ["after", "arg"]);
    }
}

#[test]
fn declared_row_partial_input_refuses_before_saved_and_host_operation() {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    const METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: U8 -> Integer -> (self: Self) -> Integer }\n";
    const HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub saved: (Integer -> Integer) -> Integer\n";
    const MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\ng = f (host.arg ())\nhost.saved g\ng 9\n";
    for argument in [3, 300] {
        let mut declarations = Declarations::new();
        declarations.module("methods", METHODS).unwrap();
        declarations.module("host", HOST).unwrap();
        let module = parse(lex(MAIN)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        assert_declared_partial_call(&program, MAIN);
        let mut machine = Machine::new(Some(1 << 20));
        let mut outcome = machine.run(&program);
        let mut calls = Vec::new();
        let mut finished = false;
        for _ in 0..10 {
            match outcome {
                Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    environment,
                }) => {
                    assert!(environment.is_empty());
                    let tables = machine.program().unwrap().tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    let label = declaration.label.clone();
                    let reply = match label.as_str() {
                        "after" => {
                            assert_eq!(arguments, [Value::Unit]);
                            7
                        }
                        "arg" => {
                            assert_eq!(arguments, [Value::Unit]);
                            argument
                        }
                        "saved" => {
                            assert_eq!(
                                argument, 3,
                                "invalid input faults before partial escapes"
                            );
                            let [Value::Function(partial)] =
                                arguments.as_slice()
                            else {
                                panic!("actual saved partial")
                            };
                            assert_declared_saved_partial(
                                machine.program().unwrap(),
                                partial,
                            );
                            0
                        }
                        "op" => {
                            assert_eq!(argument, 3);
                            let [Value::Int(3), Value::Int(9), Value::Record(receiver)] =
                                arguments.as_slice()
                            else {
                                panic!("raw full spots")
                            };
                            assert_eq!(receiver.fields, [Value::Int(7)]);
                            7
                        }
                        _ => panic!("unexpected callback"),
                    };
                    drop(tables);
                    calls.push(label);
                    outcome = machine
                        .reply(&program, Reply::Value(Ok(Value::Int(reply))));
                }
                Outcome::Done(result) => {
                    assert_declared_partial_result(
                        &machine, argument, result, &calls,
                    );
                    finished = true;
                    break;
                }
                Outcome::Request(_) => panic!("unexpected request"),
            }
        }
        assert!(finished, "bounded partial case must finish");
    }
}

const PREFIX_SEMANTICS_HOST: &str = "pub raw: () -> Integer\npub arg: () -> Integer\npub saved: (Integer -> Integer) -> Integer\npub saved2: (Integer -> Integer -> Integer) -> Integer\n";
const PREFIX_RAW_RETURN: &str = "use host\nouter: Integer -> Integer -> Integer = x => { ignored = host.raw ()\n y => x + y }\nid = f => f\nf = id outer\ng = f (host.arg ())\nhost.saved g\ng 9\n";
const PREFIX_ZERO_SUPPLY: &str = "use host\nouter: Integer -> Integer -> Integer = x => { ignored = host.raw ()\n y => x + y }\nkeep = f => f\ng = keep outer\nhost.saved2 g\ng 3 9\n";

fn prefix_semantics_program(source: &str) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("host", PREFIX_SEMANTICS_HOST).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    compile::compile(&unit)
}

fn prefix_semantics_run(
    program: &fipe::mir::Program,
    expected: &[&str],
    argument: i64,
    mut observe: impl FnMut(&Machine, &str, &[fipe::value::Value]),
) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let mut machine = Machine::new(Some(1 << 20));
    let mut outcome = machine.run(program);
    let mut calls = Vec::new();
    for _ in 0..8 {
        match outcome {
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let label = {
                    let tables = machine.program().unwrap().tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    assert_eq!(
                        tables.namespaces[declaration.namespace],
                        "host"
                    );
                    declaration.label.clone()
                };
                observe(&machine, &label, &arguments);
                let reply = match label.as_str() {
                    "arg" => {
                        assert_eq!(arguments, [Value::Unit]);
                        argument
                    }
                    "raw" => {
                        assert_eq!(arguments, [Value::Unit]);
                        7
                    }
                    "saved" | "saved2" => {
                        assert!(matches!(
                            arguments.as_slice(),
                            [Value::Function(_)]
                        ));
                        7
                    }
                    _ => panic!("unexpected callback {label}"),
                };
                calls.push(label);
                assert!(calls.len() <= expected.len());
                outcome =
                    machine.reply(program, Reply::Value(Ok(Value::Int(reply))));
            }
            Outcome::Done(result) => {
                assert_eq!(calls, expected);
                assert_eq!(result.unwrap(), Value::Int(12));
                return;
            }
            Outcome::Request(other) => {
                panic!("typed callbacks only: {other:?}")
            }
        }
    }
    panic!("bounded prefix effect sequence must finish");
}

fn prefix_semantics_calls_at(
    program: &fipe::mir::Program,
    source: &str,
    text: &str,
) -> Vec<usize> {
    let start = source.find(text).unwrap();
    let tables = program.tables.borrow();
    program
        .bodies
        .iter()
        .flat_map(|body| &body.blocks)
        .filter_map(|block| {
            let fipe::mir::Terminator::Call {
                callee: fipe::mir::Callee::Value(_),
                args,
                ..
            } = &block.terminator
            else {
                return None;
            };
            block
                .nodes
                .last()
                .is_some_and(|node| {
                    let node = &tables.nodes[*node];
                    tables.namespaces[node.namespace] == "main"
                        && node.span.start == start
                        && node.span.end == start + text.len()
                })
                .then_some(args.len())
        })
        .collect()
}

#[test]
fn a_positive_value_prefix_enters_raw_arity_one_before_saving_its_result() {
    use fipe::value::Value;
    let call = "f (host.arg ())";
    assert_eq!(PREFIX_RAW_RETURN.matches(call).count(), 1);
    let start = PREFIX_RAW_RETURN.find(call).unwrap();
    let stage = (start, start + call.len());
    let mut declarations = Declarations::new();
    declarations.module("host", PREFIX_SEMANTICS_HOST).unwrap();
    let module = parse(lex(PREFIX_RAW_RETURN)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let bound = typed
        .bound
        .get(&stage)
        .expect("actual concrete function-value call");
    assert_eq!(bound.spots.len(), 2);
    assert_eq!(bound.open, [1]);
    assert!(bound.defaults.is_empty());
    assert_eq!(bound.supplies.len(), 1);
    assert_eq!(bound.supplies[0].0, 0);
    let program = prefix_semantics_program(PREFIX_RAW_RETURN);
    assert_eq!(
        prefix_semantics_calls_at(&program, PREFIX_RAW_RETURN, call),
        [1]
    );
    let mut raw_body = None;
    prefix_semantics_run(
        &program,
        &["arg", "raw", "saved"],
        3,
        |machine, label, arguments| {
            let joined = machine.program().unwrap();
            if label == "raw" {
                let fipe::vm::Frame::Body(frame) =
                    machine.frames().last().unwrap()
                else {
                    panic!("actual raw body")
                };
                assert_eq!(joined.bodies[frame.body].arity, 1);
                raw_body = Some(frame.body);
            } else if label == "saved" {
                let [Value::Function(function)] = arguments else {
                    panic!("actual returned closure")
                };
                assert_ne!(Some(function.body), raw_body);
                assert!(raw_body.is_some(), "outer raw body already executed");
                assert_eq!(joined.bodies[function.body].arity, 1);
                assert!(function.filled.is_empty());
                assert_eq!(function.captures, [Value::Int(3)]);
            }
        },
    );
}

#[test]
fn a_zero_supply_value_read_does_not_synthesize_a_call() {
    use fipe::value::Value;
    let program = prefix_semantics_program(PREFIX_ZERO_SUPPLY);
    let start = PREFIX_ZERO_SUPPLY.find("keep = f => f").unwrap()
        + "keep = f => ".len();
    let tables = program.tables.borrow();
    assert!(!program.bodies.iter().flat_map(|body| &body.blocks).any(|block| {
        matches!(&block.terminator, fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Value(_), args, .. } if args.is_empty())
            && block.nodes.last().is_some_and(|node| {
                let node = &tables.nodes[*node];
                tables.namespaces[node.namespace] == "main" && node.span.start == start && node.span.end == start + 1
            })
    }), "plain zero-supply read must not introduce Value Call0");
    drop(tables);
    let mut saved_body = None;
    prefix_semantics_run(
        &program,
        &["saved2", "raw"],
        3,
        |machine, label, arguments| {
            let joined = machine.program().unwrap();
            if label == "saved2" {
                let [Value::Function(function)] = arguments else {
                    panic!("unentered outer function")
                };
                assert_eq!(joined.bodies[function.body].arity, 1);
                assert!(function.filled.is_empty());
                assert!(function.captures.is_empty());
                saved_body = Some(function.body);
            } else if label == "raw" {
                let fipe::vm::Frame::Body(frame) =
                    machine.frames().last().unwrap()
                else {
                    panic!("actual raw body")
                };
                assert_eq!(Some(frame.body), saved_body);
            }
        },
    );
}

const BOUND_ORDER_NEXT_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ()) (host.later ())\n";
const BOUND_ORDER_NEXT_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: U8 -> Integer -> (self: Self) -> Integer }\n";
const BOUND_ORDER_NEXT_HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub later: () -> Integer\n";
const BOUND_ORDER_HOLE_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\ng = f (host.arg ())\nhost.saved g\nTrue | g\n";
const BOUND_ORDER_HOLE_METHODS: &str = "pub type M { pub n: Integer }\nimpl M { op: (enabled: Bool) -> (n: U8) -> (self: Self) -> Integer }\n";
const BOUND_ORDER_HOLE_HOST: &str = "pub after: () -> Integer\npub arg: () -> Integer\npub saved: (Bool -> Integer) -> Integer\n";

fn bound_order_metadata(
    module: &fipe::ast::Module,
    typed: &fipe::typed::Typed,
    program: &fipe::mir::Program,
    source: &str,
    hole: bool,
) {
    use fipe::ast::{Entry, ExprKind};
    let text = if hole {
        "f (host.arg ())"
    } else {
        "f (host.arg ()) (host.later ())"
    };
    let start = source.find(text).unwrap();
    let matching: Vec<_> = module
        .entries
        .iter()
        .filter_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let expression = module.expr(*id).unwrap();
            let expression =
                if let ExprKind::Bind { value, .. } = &expression.kind {
                    module.expr(*value).unwrap()
                } else {
                    expression
                };
            (expression.span.start == start
                && expression.span.end == start + text.len())
            .then_some(expression)
        })
        .collect();
    println!(
        "BOUND_ORDER_AST matching={} target={text:?}",
        matching.len()
    );
    let [expression] = matching.as_slice() else {
        panic!("one exact source-owned call")
    };
    let ExprKind::Call {
        callee,
        arguments,
        style,
    } = &expression.kind
    else {
        panic!("actual ungrouped AST Call")
    };
    let callee = module.expr(*callee).unwrap();
    println!("BOUND_ORDER_AST_CALL callee={:?} count={} style={style:?} args={arguments:?}", source.get(callee.span.start..callee.span.end), arguments.len());
    assert_eq!(source.get(callee.span.start..callee.span.end), Some("f"));
    assert_eq!(arguments.len(), if hole { 1 } else { 2 });
    let key = (expression.span.start, expression.span.end);
    let bound = typed.bound.get(&key).expect("actual call Bound");
    println!("BOUND_ORDER_TYPED key={key:?} bound={bound:?} selected={:?} access={:?} callee_type={:?}", typed.selected.get(&key), typed.accesses.get(&key), typed.callee_types.get(&key));
    assert!(typed.member_contracts.is_empty(), "no written-provider contract substitutes for the selected bodyless row");
    assert!(bound.defaults.is_empty());
    assert!(bound.applied.is_empty());
    assert_eq!(bound.open, if hole { vec![0] } else { vec![] });
    assert_eq!(
        bound
            .supplies
            .iter()
            .map(|(spot, _)| *spot)
            .collect::<Vec<_>>(),
        if hole { vec![1] } else { vec![0, 1] }
    );
    let tables = program.tables.borrow();
    let calls: Vec<_> = tables
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let fipe::ir::Kind::Call {
                callee,
                slots,
                supplies,
                defaults,
                open,
            } = &node.kind
            else {
                return None;
            };
            (tables.namespaces[node.namespace] == "main"
                && node.span.start == key.0
                && node.span.end == key.1)
                .then_some((index, *callee, slots, supplies, defaults, open))
        })
        .collect();
    println!("BOUND_ORDER_IR count={} calls={calls:?}", calls.len());
    assert!(!calls.is_empty(), "actual matching IR call must exist");
    for (index, callee, slots, supplies, defaults, open) in calls {
        println!("BOUND_ORDER_IR_CALL node={index} callee_kind={:?} slots={slots:?} supplies={supplies:?} defaults={defaults:?} open={open:?}", tables.nodes[callee].kind);
        assert!(defaults.is_empty());
        assert_eq!(*open, bound.open);
        assert_eq!(
            supplies
                .iter()
                .map(|supply| supply.spot)
                .collect::<Vec<_>>(),
            bound
                .supplies
                .iter()
                .map(|(spot, _)| *spot)
                .collect::<Vec<_>>()
        );
        assert!(supplies.iter().all(|supply| !supply.element));
    }
}

fn bound_order_selected_entry(machine: &Machine) -> usize {
    use fipe::value::Value;
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
        panic!("actual current bound method f")
    };
    let adapters: Vec<_> = joined
        .receiver_adapters
        .iter()
        .filter(|adapter| adapter.body == function.body)
        .collect();
    let [adapter] = adapters.as_slice() else {
        panic!("actual f identifies one joined adapter")
    };
    let row = &tables.dispatch[adapter.member].rows[adapter.row];
    let fipe::ir::Implementation::Declared(declaration) = &row.target else {
        panic!("actual selected bodyless row")
    };
    let declaration = &tables.declarations[*declaration];
    assert_eq!(tables.namespaces[declaration.namespace], "methods");
    assert_eq!(declaration.label, "op");
    assert_eq!(row.receiver_spot, Some(2));
    let context = joined.receiver_contexts[function.body].as_ref().unwrap();
    let entry = row
        .input_contract
        .expect("actual selected row owns an input contract");
    assert_eq!(context.input_contract, Some(entry));
    println!("BOUND_ORDER_ACTUAL_ROW member={} row={} body={} raw_arity={} filled={:?} self_spot={} entry={entry} parameters={:?} predicate={:?}", adapter.member, adapter.row, function.body, joined.bodies[function.body].arity, function.filled, adapter.receiver_spot, context.input_parameters, tables.entries[entry].predicate);
    assert_eq!(joined.bodies[function.body].arity, 3);
    assert_eq!(function.filled.len(), 1);
    assert!(
        matches!(&function.filled[0], Value::Record(receiver) if receiver.fields == [Value::Int(7)])
    );
    entry
}

fn bound_order_test_ids(predicate: &fipe::ir::Predicate) -> Vec<usize> {
    use fipe::ir::Predicate;
    let mut pending = vec![predicate];
    let mut tests = Vec::new();
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::Test { index, .. } => {
                if !tests.contains(index) {
                    tests.push(*index);
                }
            }
            Predicate::All(parts) | Predicate::Any(parts) => {
                pending.extend(parts);
            }
            Predicate::Not(inner) => pending.push(inner),
            Predicate::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            Predicate::Nonzero { .. } => {}
        }
    }
    tests
}

fn bound_order_operation(
    joined: &fipe::mir::Program,
    arguments: &[super::Value],
    hole: bool,
    argument: i64,
) {
    use fipe::value::Value;
    let [first, second, Value::Record(receiver)] = arguments else {
        panic!("actual three raw spots")
    };
    assert_eq!(receiver.fields, [Value::Int(7)]);
    if hole {
        let Value::Record(boolean) = first else {
            panic!("actual Bool record")
        };
        let tables = joined.tables.borrow();
        let meaning = &tables.meanings[boolean.meaning];
        assert_eq!(tables.namespaces[meaning.namespace], "prelude");
        assert_eq!(meaning.label, "Bool");
        assert_eq!(meaning.constructors[boolean.constructor].label, "True");
        assert!(boolean.fields.is_empty());
        assert_eq!(second, &Value::Int(argument));
    } else {
        assert_eq!(first, &Value::Int(argument));
        assert_eq!(second, &Value::Int(9));
    }
}

fn bound_order_result(
    machine: &Machine,
    hole: bool,
    argument: i64,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
    selected_entry: Option<usize>,
) {
    use fipe::value::Value;
    println!("BOUND_ORDER_DONE hole={hole} argument={argument} result={result:?} calls={calls:?} charged={}", machine.charged());
    let entry =
        selected_entry.expect("actual row captured before argument effects");
    if argument == 3 {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert_eq!(
            calls,
            if hole {
                vec!["host.after", "host.arg", "host.saved", "methods.op"]
            } else {
                vec!["host.after", "host.arg", "host.later", "methods.op"]
            }
        );
    } else {
        assert_eq!(argument, 300);
        let fault =
            result.expect_err("the exact selected U8 contract refuses 300");
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        println!(
            "BOUND_ORDER_FAULT namespace={} span={:?} reason={:?}",
            tables.namespaces[fault.namespace], fault.span, fault.reason
        );
        let Reason::Failed {
            entry: failed,
            value,
        } = &fault.reason
        else {
            panic!("actual semantic U8 refusal")
        };
        assert_eq!(value, "300");
        assert_eq!(
            *failed, entry,
            "actual failure belongs to the selected row input contract"
        );
        assert_eq!(tables.entries[*failed].label, "U8");
        let row_tests = bound_order_test_ids(&tables.entries[entry].predicate);
        let failed_tests =
            bound_order_test_ids(&tables.entries[*failed].predicate);
        println!("BOUND_ORDER_TEST_IDENTITY row={row_tests:?} failed={failed_tests:?}");
        assert!(!failed_tests.is_empty());
        assert!(
            failed_tests.iter().all(|test| row_tests.contains(test)),
            "failure uses the selected row's actual semantic test"
        );
        assert_eq!(calls, ["host.after", "host.arg"], "required refusal before later/saved/op, not merely before operation");
    }
}

fn bound_order_program(
    source: &str,
    methods: &str,
    host: &str,
    hole: bool,
) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("methods", methods).unwrap();
    declarations.module("host", host).unwrap();
    let module = parse(lex(source)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    bound_order_metadata(&module, &typed, &program, source, hole);
    program
}

fn bound_order_saved(joined: &fipe::mir::Program, arguments: &[super::Value]) {
    use fipe::value::Value;
    let [Value::Function(function)] = arguments else {
        panic!("actual remaining Bool function")
    };
    println!(
        "BOUND_ORDER_SAVED body={} arity={} filled={:?} captures={:?}",
        function.body,
        joined.bodies[function.body].arity,
        function.filled,
        function.captures
    );
    assert_eq!(joined.bodies[function.body].arity, 1);
    assert!(function.captures.iter().any(|capture| matches!(capture, Value::Function(original) if joined.receiver_adapters.iter().any(|adapter| adapter.body == original.body))), "nonprefix wrapper captures the actual selected adapter");
}

fn bound_order_case(hole: bool, argument: i64) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let (source, methods, host) = if hole {
        (
            BOUND_ORDER_HOLE_MAIN,
            BOUND_ORDER_HOLE_METHODS,
            BOUND_ORDER_HOLE_HOST,
        )
    } else {
        (
            BOUND_ORDER_NEXT_MAIN,
            BOUND_ORDER_NEXT_METHODS,
            BOUND_ORDER_NEXT_HOST,
        )
    };
    println!(
        "BOUND_ORDER_CASE hole={hole} argument={argument} source={source:?}"
    );
    let program = bound_order_program(source, methods, host, hole);
    let mut machine = Machine::new(Some(1 << 20));
    let mut outcome = machine.run(&program);
    let mut calls = Vec::new();
    let mut selected_entry = None;
    for _ in 0..10 {
        match outcome {
            Outcome::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                let joined = machine.program().unwrap();
                let (namespace, label) = {
                    let tables = joined.tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    (
                        tables.namespaces[declaration.namespace].clone(),
                        declaration.label.clone(),
                    )
                };
                println!("BOUND_ORDER_CALLBACK hole={hole} argument={argument} namespace={namespace} label={label} args={arguments:?} env={environment:?} charged={}", machine.charged());
                assert!(environment.is_empty());
                let reply = if namespace == "host" {
                    match label.as_str() {
                        "after" => {
                            assert_eq!(arguments, [Value::Unit]);
                            selected_entry =
                                Some(bound_order_selected_entry(&machine));
                            7
                        }
                        "arg" => {
                            assert_eq!(arguments, [Value::Unit]);
                            argument
                        }
                        "later" => {
                            assert!(!hole);
                            assert_eq!(arguments, [Value::Unit]);
                            9
                        }
                        "saved" => {
                            assert!(hole);
                            bound_order_saved(joined, &arguments);
                            7
                        }
                        _ => panic!("unexpected callback"),
                    }
                } else {
                    assert_eq!(namespace, "methods");
                    assert_eq!(label, "op");
                    bound_order_operation(joined, &arguments, hole, argument);
                    7
                };
                calls.push(format!("{namespace}.{label}"));
                outcome = machine
                    .reply(&program, Reply::Value(Ok(Value::Int(reply))));
            }
            Outcome::Done(result) => {
                bound_order_result(
                    &machine,
                    hole,
                    argument,
                    result,
                    &calls,
                    selected_entry,
                );
                return;
            }
            Outcome::Request(other) => {
                panic!("typed callbacks only: {other:?}")
            }
        }
    }
    panic!("bounded diagnostic must finish");
}

#[test]
fn declared_row_next_argument_healthy_diagnostic() {
    bound_order_case(false, 3);
}
#[test]
fn declared_row_next_argument_refuses_before_later_diagnostic() {
    bound_order_case(false, 300);
}
#[test]
fn declared_row_nonprefix_bool_hole_healthy_diagnostic() {
    bound_order_case(true, 3);
}
#[test]
fn declared_row_nonprefix_bool_hole_refuses_before_saved_diagnostic() {
    bound_order_case(true, 300);
}

const NESTED_ONCE_METHODS: &str = "pub type Checked: Integer where self < 10\npub type M { pub n: Integer }\nimpl M { op: (enabled: Bool) -> (a: Integer) -> (b: Integer) -> (n: Checked) -> (self: Self) -> Integer }\n";
const NESTED_ONCE_HOST: &str = "pub arg: () -> Integer\npub again: () -> Integer\npub seen: (Bool -> Integer -> Integer) -> Integer\npub saved: (Bool -> Integer) -> Integer\n";
const NESTED_ONCE_MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\ng = f 2\nh = g 3\nseen = host.seen h\np = h (host.arg ())\nsaved_p = host.saved p\nfirst = True | p\nq = h (host.again ())\nsaved_q = host.saved q\nfirst + (True | q)\n";

fn nested_once_path(
    program: &fipe::mir::Program,
    root: &std::rc::Rc<fipe::value::Closure>,
) -> Vec<std::rc::Rc<fipe::value::Closure>> {
    use fipe::value::Value;
    let mut path = vec![root.clone()];
    for _ in 0..8 {
        let current = path.last().unwrap();
        let Some(binding) = &program.bodies[current.body].binding else {
            return path;
        };
        assert_eq!(program.bodies[current.body].arity, binding.open.len());
        let Some(Value::Function(child)) =
            current.captures.get(binding.callee_capture)
        else {
            panic!("actual generated wrapper captures its callable")
        };
        path.push(child.clone());
    }
    panic!("bounded source wrapper path must terminate")
}

fn nested_once_original(
    program: &fipe::mir::Program,
    path: &[std::rc::Rc<fipe::value::Closure>],
) -> (usize, usize) {
    use fipe::value::Value;
    assert_eq!(path.len(), 3, "h must have two actual open-wrapper edges");
    assert!(path[..2].iter().all(|closure| closure.filled.is_empty()));
    let h = program.bodies[path[0].body].binding.as_ref().unwrap();
    let g = program.bodies[path[1].body].binding.as_ref().unwrap();
    assert_eq!(h.open, [0, 2]);
    assert_eq!(g.open, [0, 2, 3]);
    let position = g.open[h.open[1]] + path[2].filled.len();
    assert_eq!(position, 4, "positional n must map through both wrappers");
    let leaf = &path[2];
    assert_eq!(program.bodies[leaf.body].arity, 5);
    assert_eq!(leaf.filled.len(), 1);
    assert!(
        matches!(&leaf.filled[0], Value::Record(receiver) if receiver.fields == [Value::Int(7)])
    );
    let context = program.receiver_contexts[leaf.body].as_ref().unwrap();
    let entry = context
        .input_contract
        .expect("selected declared row contract");
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "methods"
                && NESTED_ONCE_METHODS.get(test.span.start..test.span.end)
                    == Some("self < 10"))
            .then_some(index)
        })
        .collect();
    assert_eq!(tests.len(), 1, "actual source-linked pure Test");
    let guard: Vec<_> = leaf
        .guards
        .iter()
        .filter(|guard| guard.entry == entry && guard.spot == position)
        .collect();
    assert_eq!(guard.len(), 1, "actual refined leaf guard before binding");
    assert!(bound_order_test_ids(&guard[0].predicate).contains(&tests[0]));
    assert!(bound_order_test_ids(&tables.entries[entry].predicate)
        .contains(&tests[0]));
    (entry, position)
}

fn nested_once_private(
    program: &fipe::mir::Program,
    original: &[std::rc::Rc<fipe::value::Closure>],
    partial: &std::rc::Rc<fipe::value::Closure>,
    entry: usize,
    position: usize,
) {
    use std::rc::Rc;
    let private = nested_once_path(program, partial);
    assert_eq!(
        private.len(),
        4,
        "p/q adds one wrapper above private h path"
    );
    assert_eq!(
        program.bodies[private[0].body]
            .binding
            .as_ref()
            .unwrap()
            .open,
        [0]
    );
    for (old, new) in original.iter().zip(&private[1..]) {
        assert_eq!(old.body, new.body);
        assert!(
            !Rc::ptr_eq(old, new),
            "each captured edge must be copied privately"
        );
        assert_eq!(old.filled, new.filled);
        let capture = program.bodies[old.body]
            .binding
            .as_ref()
            .map(|binding| binding.callee_capture);
        for (index, (a, b)) in
            old.captures.iter().zip(&new.captures).enumerate()
        {
            if Some(index) != capture {
                assert_eq!(a, b);
            }
        }
        for guard in &new.guards {
            let same = old
                .guards
                .iter()
                .find(|prior| {
                    prior.entry == guard.entry && prior.spot == guard.spot
                })
                .unwrap();
            assert!(Rc::ptr_eq(&same.witnesses, &guard.witnesses));
        }
    }
    assert!(
        original[2]
            .guards
            .iter()
            .any(|guard| guard.entry == entry && guard.spot == position),
        "original alias still owns its guard"
    );
    assert!(
        !private[3]
            .guards
            .iter()
            .any(|guard| guard.entry == entry && guard.spot == position),
        "successful early guard is discharged only in private leaf"
    );
}

fn nested_once_test_position(program: &fipe::mir::Program) -> (usize, usize) {
    use fipe::mir::{Statement, Terminator};
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "methods"
                && NESTED_ONCE_METHODS.get(test.span.start..test.span.end)
                    == Some("self < 10"))
            .then_some(index)
        })
        .collect();
    assert_eq!(tests.len(), 1, "unique actual pure source Test");
    let body = program.tests[tests[0]].expect("actual compiled pure Test body");
    let blocks = &program.bodies[body].blocks;
    for block in blocks {
        let edges = match &block.terminator {
            Terminator::Goto(target) | Terminator::Call { target, .. } => {
                vec![*target]
            }
            Terminator::Switch {
                cases, otherwise, ..
            } => cases
                .iter()
                .map(|(_, target)| *target)
                .chain(otherwise.iter().copied())
                .collect(),
            Terminator::Test { pass, fail, .. } => vec![*pass, *fail],
            Terminator::Return | Terminator::Unreachable => vec![],
        };
        assert!(
            !edges.contains(&0),
            "simple source Test must not jump back to entry"
        );
    }
    let first = blocks[0]
        .statements
        .iter()
        .position(|statement| !matches!(statement, Statement::Charge(_)))
        .unwrap_or(blocks[0].statements.len());
    assert!(
        blocks[0].nodes.get(first).is_some(),
        "actual first announced entry position"
    );
    (body, first)
}

fn nested_once_program() -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations
        .module("methods", NESTED_ONCE_METHODS)
        .expect("stage 1 methods declaration admission");
    declarations
        .module("host", NESTED_ONCE_HOST)
        .expect("stage 1 host declaration admission");
    let module =
        parse(lex(NESTED_ONCE_MAIN)).expect("stage 1 checked public syntax");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("stage 2 positional Bool-hole source admission");
    compile::compile(&unit)
}

fn nested_once_operation(
    joined: &fipe::mir::Program,
    arguments: &[super::Value],
) {
    use fipe::value::Value;
    let [Value::Record(boolean), Value::Int(2), Value::Int(3), Value::Int(3), Value::Record(receiver)] =
        arguments
    else {
        panic!("actual original raw row arguments")
    };
    let tables = joined.tables.borrow();
    assert_eq!(tables.meanings[boolean.meaning].label, "Bool");
    assert_eq!(
        tables.meanings[boolean.meaning].constructors[boolean.constructor]
            .label,
        "True"
    );
    assert!(boolean.fields.is_empty());
    assert_eq!(receiver.fields, [Value::Int(7)]);
}

fn nested_once_saved(
    joined: &fipe::mir::Program,
    arguments: &[super::Value],
    test_entries: usize,
    saved: usize,
    contract: (usize, usize),
    original: &[std::rc::Rc<fipe::value::Closure>],
) {
    use fipe::value::Value;
    assert_eq!(
        test_entries, saved,
        "exactly one predicate before each save"
    );
    let [Value::Function(partial)] = arguments else {
        panic!("actual saved Bool partial")
    };
    let (entry, position) = contract;
    nested_once_private(joined, original, partial, entry, position);
}

fn assert_nested_once_result(
    first: i64,
    second: i64,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
    contract: Option<(usize, usize)>,
) {
    use fipe::value::Value;
    if first < 10 && second < 10 {
        assert_eq!(result.unwrap(), Value::Int(14));
        assert_eq!(
            calls,
            ["seen", "arg", "saved", "op", "again", "saved", "op"]
        );
    } else {
        let fault = result.unwrap_err();
        let Reason::Failed { entry, value } = fault.reason else {
            panic!("actual Checked refusal: {fault:?}")
        };
        assert_eq!(Some(entry), contract.map(|(entry, _)| entry));
        assert_eq!(value, "300");
        if first >= 10 {
            assert_eq!(calls, ["seen", "arg",]);
        } else {
            assert_eq!(calls, ["seen", "arg", "saved", "op", "again",]);
        }
    }
}

fn nested_once_seen(
    joined: &fipe::mir::Program,
    arguments: &[super::Value],
    test_entries: usize,
) -> Vec<std::rc::Rc<fipe::value::Closure>> {
    use fipe::value::Value;
    assert_eq!(test_entries, 0, "no Test before checked input binding");
    let [Value::Function(function)] = arguments else {
        panic!("actual source h function")
    };
    nested_once_path(joined, function)
}

fn nested_once_test_entry(
    machine: &Machine,
    test_body: usize,
    first_position: usize,
) -> Option<(usize, usize, usize)> {
    if let Some(fipe::vm::Frame::Body(frame)) = machine.frames().last() {
        if frame.body == test_body
            && frame.block == 0
            && frame.statement == first_position
        {
            return Some((frame.body, frame.block, frame.statement));
        }
    }
    None
}

fn assert_nested_once_counts(
    first: i64,
    second: i64,
    test_entries: usize,
    saved: usize,
    operations: usize,
) {
    if first < 10 && second < 10 {
        assert_eq!((test_entries, saved, operations), (2, 2, 2));
    } else if first >= 10 {
        assert_eq!((test_entries, saved, operations), (1, 0, 0));
    } else {
        assert_eq!((test_entries, saved, operations), (2, 1, 1));
    }
}

fn nested_once_argument(
    label: &str,
    arguments: &[super::Value],
    original_present: bool,
    test_entries: usize,
    first: i64,
    second: i64,
) -> super::Value {
    use fipe::value::Value;
    assert_eq!(arguments, [Value::Unit]);
    assert!(
        original_present,
        "stage 3 nested maps observed before admission"
    );
    assert_eq!(
        test_entries,
        usize::from(label != "arg"),
        "Test count before source argument effect"
    );
    Value::Int(if label == "arg" { first } else { second })
}

fn nested_once_request_identity(
    joined: &fipe::mir::Program,
    declaration: usize,
) -> (String, String) {
    {
        let tables = joined.tables.borrow();
        let declaration = &tables.declarations[declaration];
        (
            tables.namespaces[declaration.namespace].clone(),
            declaration.label.clone(),
        )
    }
}

fn nested_once_case(first: i64, second: i64) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    let program = nested_once_program();
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let (test_body, first_position) =
        nested_once_test_position(machine.program().unwrap());
    let mut calls = Vec::new();
    let mut original = None;
    let mut contract = None;
    let mut test_entries = 0;
    let mut saved = 0;
    let mut operations = 0;
    for _ in 0..20_000 {
        match machine.next_stop() {
            fipe::vm::Stop::Before(node) => {
                if let Some((body, block, statement)) =
                    nested_once_test_entry(&machine, test_body, first_position)
                {
                    test_entries += 1;
                    println!("NESTED_ONCE_TEST_ENTER count={test_entries} body={body} block={block} statement={statement} node={node}");
                }
            }
            fipe::vm::Stop::After(..) => {}
            fipe::vm::Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let joined = machine.program().unwrap();
                let (namespace, label) =
                    nested_once_request_identity(joined, declaration);
                println!("NESTED_ONCE_REQUEST {namespace}.{label} args={arguments:?}");
                let reply = match (namespace.as_str(), label.as_str()) {
                    ("host", "seen") => {
                        let path =
                            nested_once_seen(joined, &arguments, test_entries);
                        contract = Some(nested_once_original(joined, &path));
                        original = Some(path);
                        Value::Int(0)
                    }
                    ("host", "arg" | "again") => nested_once_argument(
                        &label,
                        &arguments,
                        original.is_some(),
                        test_entries,
                        first,
                        second,
                    ),
                    ("host", "saved") => {
                        saved += 1;
                        nested_once_saved(
                            joined,
                            &arguments,
                            test_entries,
                            saved,
                            contract.unwrap(),
                            original.as_ref().unwrap(),
                        );
                        Value::Int(0)
                    }
                    ("methods", "op") => {
                        operations += 1;
                        assert_eq!(test_entries, operations, "final invocation must not replay admitted predicate");
                        nested_once_operation(joined, &arguments);
                        Value::Int(7)
                    }
                    _ => panic!("unexpected callback {namespace}.{label}"),
                };
                calls.push(label);
                machine.answer(Reply::Value(Ok(reply)));
            }
            fipe::vm::Stop::Done(result) => {
                println!("NESTED_ONCE_DONE result={result:?} calls={calls:?}");
                assert_nested_once_result(
                    first, second, result, &calls, contract,
                );
                assert_nested_once_counts(
                    first,
                    second,
                    test_entries,
                    saved,
                    operations,
                );
                return;
            }
            fipe::vm::Stop::Request(other) => {
                panic!("unexpected request {other:?}")
            }
        }
    }
    panic!("bounded nested/once driver must finish")
}

#[test]
fn declared_row_nested_wrappers_discharge_privately_and_run_predicate_once() {
    nested_once_case(3, 3);
}

#[test]
fn declared_row_nested_wrappers_refuse_before_first_escape() {
    nested_once_case(300, 3);
}

#[test]
fn declared_row_nested_wrappers_preserve_original_alias_guard() {
    nested_once_case(3, 300);
}

fn assert_saturated_receiver_adapter(
    program: &fipe::mir::Program,
    start: usize,
) {
    use fipe::ir::Implementation;
    {
        let tables = program.tables.borrow();
        let found: Vec<_> = tables
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(index, definition)| {
                (tables.namespaces[definition.namespace] == "methods"
                    && definition.span.start == start
                    && definition.span.end == start + 2)
                    .then_some(index)
            })
            .collect();
        assert_eq!(found.len(), 1, "exact source-owned written op");
        let definition = found[0];
        assert_eq!(
            program.bodies[program.definitions[definition].unwrap()].arity,
            1
        );
        let adapters: Vec<_> = program.receiver_adapters.iter().filter(|adapter| {
            matches!(tables.dispatch[adapter.member].rows[adapter.row].target,
                Implementation::Definition { index, .. } if index == definition)
        }).collect();
        assert!(!adapters.is_empty(), "actual generic member row");
        for adapter in adapters {
            assert_eq!(
                adapter.receiver_spot, 0,
                "written self is its only raw head"
            );
            assert_eq!(program.bodies[adapter.body].arity, 1);
        }
    }
}

#[test]
fn a_saturated_written_receiver_runs_before_its_result_argument() {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
    };
    const HOST: &str = "pub raw: () -> Integer\npub arg: () -> Integer\n";
    const METHODS: &str = "use host\npub type M { pub n: Integer }\nimpl M { op: Self -> Integer -> Integer = self => { ignored = host.raw ()\n delta => self.n + delta } }\n";
    const MAIN: &str = "use host\nuse methods\ncall = m => m.op (host.arg ())\ncall (methods.M 3)\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    declarations.module("methods", METHODS).unwrap();
    let module = parse(lex(MAIN)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let start = METHODS.find("op:").unwrap();
    assert_saturated_receiver_adapter(&program, start);
    let mut machine = Machine::new(None);
    let mut outcome = machine.run(&program);
    let mut trace = Vec::new();
    for expected in ["raw", "arg"] {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            environment,
        }) = outcome
        else {
            panic!("{expected} callback: {outcome:?}")
        };
        assert!(environment.is_empty());
        assert_eq!(arguments, [Value::Unit]);
        let label = {
            let tables = machine.program().unwrap().tables.borrow();
            let declaration = &tables.declarations[declaration];
            assert_eq!(tables.namespaces[declaration.namespace], "host");
            declaration.label.clone()
        };
        trace.push(label.clone());
        assert_eq!(
            label, expected,
            "actual receiver body precedes result operand: {trace:?}"
        );
        if label == "raw" {
            let joined = machine.program().unwrap();
            let tables = joined.tables.borrow();
            let found: Vec<_> = tables
                .definitions
                .iter()
                .enumerate()
                .filter_map(|(index, declaration)| {
                    (tables.namespaces[declaration.namespace] == "methods"
                        && declaration.span.start == start
                        && declaration.span.end == start + 2)
                        .then_some(index)
                })
                .collect();
            assert_eq!(found.len(), 1, "joined source-owned op");
            let body = joined.definitions[found[0]].unwrap();
            assert!(machine.frames().iter().any(|frame| matches!(frame,
                fipe::vm::Frame::Body(frame) if frame.body == body)));
        }
        outcome = machine.reply(
            &program,
            Reply::Value(Ok(Value::Int(if label == "raw" { 7 } else { 9 }))),
        );
    }
    assert!(
        matches!(outcome, Outcome::Done(Ok(Value::Int(12)))),
        "{outcome:?}"
    );
}

const NESTED_ROW_METHODS: &str = "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\npub type Sorted<T: Ord>: Vec<T> where is_sorted self\npub type M<T: Ord> { pub token: T }\nimpl M { op: (Sorted<T> -> Integer) -> (self: Self) -> Integer }\n";
const NESTED_ROW_HOST_INTEGER: &str = "pub after: () -> Integer\npub arg: () -> Vec<Integer>\npub recall: () -> a\n";
const NESTED_ROW_HOST_STRING: &str = "pub after: () -> Integer\npub arg: () -> Vec<String>\npub recall: () -> a\n";
const NESTED_ROW_MAIN_INTEGER: &str = "use host\nuse methods\nget = m => m.op\nrelay = get (methods.M 7)\nop_result = relay (xs => 7)\nafter_result = host.after ()\nwrapped: Vec<Integer> -> Integer = host.recall ()\nwrapped (host.arg ())\n";
const NESTED_ROW_MAIN_STRING: &str = "use host\nuse methods\nget = m => m.op\nrelay = get (methods.M \"tag\")\nop_result = relay (xs => 7)\nafter_result = host.after ()\nwrapped: Vec<String> -> Integer = host.recall ()\nwrapped (host.arg ())\n";

fn nested_row_metadata(
    typed: &fipe::typed::Typed,
    program: &fipe::mir::Program,
) {
    println!("NESTED_ROW_CALLER_CHECKS {:?}", typed.checks);
    println!(
        "NESTED_ROW_TYPED_OP {:?}",
        typed
            .dispatch
            .iter()
            .filter(|dispatch| dispatch.name == "op")
            .collect::<Vec<_>>()
    );
    for test in typed.tests.iter().filter(|test| {
        test.origin == fipe::resolve::Origin::Host("methods".into())
    }) {
        println!(
            "NESTED_ROW_TYPED_TEST owner={:?} value={:?} recorded={:?}",
            test.owner,
            test.value,
            test.expression
                .as_ref()
                .map(|expression| (&expression.origin, expression.value))
        );
    }
    let tables = program.tables.borrow();
    for (member, dispatch) in tables
        .dispatch
        .iter()
        .enumerate()
        .filter(|(_, dispatch)| dispatch.label == "op")
    {
        for (row_index, row) in dispatch.rows.iter().enumerate() {
            let fipe::ir::Implementation::Declared(declaration) = &row.target
            else {
                continue;
            };
            let declaration = &tables.declarations[*declaration];
            if tables.namespaces[declaration.namespace] != "methods" {
                continue;
            }
            println!("NESTED_ROW_IR member={member} row={row_index} declaration={declaration:?} receiver={:?} ty={:?} rawarity={:?} selfspot={:?} input_contract={:?}", row.receiver, row.ty, row.declared_arity, row.receiver_spot, row.input_contract);
            assert_eq!(row.declared_arity, Some(2));
            assert_eq!(row.receiver_spot, Some(1));
            if let Some(entry) = row.input_contract {
                println!(
                    "NESTED_ROW_INPUT_ENTRY entry={entry} predicate={:?}",
                    tables.entries[entry].predicate
                );
            } else {
                println!("NESTED_ROW_INPUT_ENTRY ABSENT");
            }
        }
    }
    for (index, test) in tables.tests.iter().enumerate().filter(|(_, test)| {
        tables.namespaces[test.namespace] == "methods"
            && NESTED_ROW_METHODS.get(test.span.start..test.span.end)
                == Some("is_sorted self")
    }) {
        println!("NESTED_ROW_IR_SORTED_TEST index={index} parameters={:?} body={:?} span={:?}", test.parameters, program.tests[index], test.span);
    }
}

fn nested_row_selected(machine: &Machine, meaning: &str, requested: usize) {
    use fipe::{ir::Type, value::Value, vm::Frame};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    for (index, value) in machine.globals() {
        let definition = &tables.definitions[index];
        println!("NESTED_ROW_GLOBAL index={index} namespace={} label={} value={value:?}", tables.namespaces[definition.namespace], definition.label);
    }
    let globals: Vec<_> = machine
        .globals()
        .filter(|(index, _)| {
            let definition = &tables.definitions[*index];
            tables.namespaces[definition.namespace] == "main"
                && definition.label == "relay"
        })
        .collect();
    let mut observed = Vec::new();
    if let [(_, Value::Function(function))] = globals.as_slice() {
        let adapters: Vec<_> = joined
            .receiver_adapters
            .iter()
            .filter(|adapter| adapter.body == function.body)
            .collect();
        let [adapter] = adapters.as_slice() else {
            panic!("actual retained relay receiver adapter")
        };
        observed.push((*adapter, function.meanings.as_slice()));
    } else {
        println!(
            "NESTED_ROW_RELAY_NOT_RETAINED_AS_FUNCTION observed={globals:?}"
        );
    }
    for frame in machine.frames() {
        let Frame::Body(frame) = frame else { continue };
        let root = &tables.nodes[joined.bodies[frame.body].node];
        println!("NESTED_ROW_FRAME body={} namespace={} span={:?} hidden={:?} meanings={:?}", frame.body, tables.namespaces[root.namespace], root.span, joined.bodies[frame.body].hidden, frame.meanings);
        for adapter in joined
            .receiver_adapters
            .iter()
            .filter(|adapter| adapter.body == frame.body)
        {
            let row = &tables.dispatch[adapter.member].rows[adapter.row];
            if matches!(&row.target, fipe::ir::Implementation::Declared(declaration) if *declaration == requested)
            {
                observed.push((adapter, frame.meanings.as_slice()));
            }
        }
    }
    if observed.is_empty() {
        println!("NESTED_ROW_SELECTED_CONTEXT_NOT_RETAINED");
    }
    for (adapter, meanings) in observed {
        let row = &tables.dispatch[adapter.member].rows[adapter.row];
        let fipe::ir::Implementation::Declared(declaration) = &row.target
        else {
            panic!("actual declared row")
        };
        assert_eq!(*declaration, requested);
        assert_eq!(
            tables.namespaces[tables.declarations[*declaration].namespace],
            "methods"
        );
        assert_eq!(tables.declarations[*declaration].label, "op");
        assert_eq!(row.receiver_spot, Some(1));
        assert_eq!(joined.bodies[adapter.body].arity, 2);
        let context = joined.receiver_contexts[adapter.body].as_ref().unwrap();
        let Type::Meaning {
            index: row_meaning,
            arguments: row_arguments,
        } = &row.receiver
        else {
            panic!("actual M<T> row receiver")
        };
        let [Type::Parameter(parameter)] = row_arguments.as_slice() else {
            panic!("one exact row receiver binder")
        };
        let Type::Meaning {
            index: actual_meaning,
            arguments: actual_arguments,
        } = &meanings[context.receiver_meaning]
        else {
            panic!("actual captured receiver meaning")
        };
        assert_eq!(actual_meaning, row_meaning);
        let [Type::Meaning {
            index: actual,
            arguments,
        }] = actual_arguments.as_slice()
        else {
            panic!("actual selected T")
        };
        assert!(arguments.is_empty());
        assert_eq!(
            tables.namespaces[tables.meanings[*actual].namespace],
            "prelude"
        );
        assert_eq!(tables.meanings[*actual].label, meaning);
        println!("NESTED_ROW_SELECTED member={} row={} adapter={} row_parameter={parameter} actual_T={:?} input_contract={:?} input_parameters={:?} hidden={:?} meanings={meanings:?}", adapter.member, adapter.row, adapter.body, actual_arguments[0], context.input_contract, context.input_parameters, joined.bodies[adapter.body].hidden);
    }
}

fn observe_nested_row_test(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    test: Option<usize>,
    meaning: &str,
) {
    let test = test.expect("observed Test body has an actual source Test");
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    println!(
        "NESTED_ROW_ACTUAL_TEST body={} hidden={:?} meanings={:?}",
        frame.body, joined.bodies[frame.body].hidden, frame.meanings
    );
    for parameter in &tables.tests[test].parameters {
        let position = joined.bodies[frame.body]
            .hidden
            .iter()
            .position(|known| known == parameter)
            .expect("actual Test binder in body hidden");
        let fipe::ir::Type::Meaning { index, arguments } =
            &frame.meanings[position]
        else {
            panic!("concrete Test witness")
        };
        assert!(arguments.is_empty());
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert_eq!(tables.meanings[*index].label, meaning);
    }
    assert!(!tables.tests[test].parameters.is_empty());
}

fn nested_row_received_function(
    machine: &Machine,
    meaning: &str,
    declaration: usize,
    arguments: &[super::Value],
    marker: &super::Value,
) -> std::rc::Rc<fipe::value::Closure> {
    use fipe::value::Value;
    nested_row_selected(machine, meaning, declaration);
    let [Value::Function(function), Value::Record(receiver)] = arguments else {
        panic!("actual Function and self-last receiver")
    };
    assert_eq!(receiver.fields.as_slice(), core::slice::from_ref(marker));
    let tables = machine.program().unwrap().tables.borrow();
    assert_eq!(
        tables.namespaces[tables.meanings[receiver.meaning].namespace],
        "methods"
    );
    assert_eq!(tables.meanings[receiver.meaning].label, "M");
    println!(
        "NESTED_ROW_RECEIVED_FUNCTION body={} guards={:?} filled={:?}",
        function.body, function.guards, function.filled
    );
    function.clone()
}

fn assert_nested_row_result(
    machine: &Machine,
    healthy: bool,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
    test: Option<usize>,
    tested: bool,
    lambda_entered: bool,
) {
    use fipe::value::Value;
    assert_eq!(
        calls,
        ["methods.op", "host.after", "host.recall", "host.arg"]
    );
    if healthy {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert!(
            tested,
            "real Sorted predicate must execute on admitted function input"
        );
        assert!(lambda_entered);
    } else {
        let fault = result
            .expect_err("reversed input must be refused by the function guard");
        let Reason::Failed { entry, .. } = &fault.reason else {
            panic!("actual Sorted semantic failure: {fault:?}")
        };
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        println!("NESTED_ROW_FAULT namespace={} span={:?} entry={entry} predicate={:?}", tables.namespaces[fault.namespace], fault.span, tables.entries[*entry].predicate);
        assert!(tested, "actual source-owned Sorted Test frame executes");
        let test = test
            .expect("terminal refusal must identify the actual methods Test");
        assert!(bound_order_test_ids(&tables.entries[*entry].predicate)
            .contains(&test));
        assert!(!lambda_entered, "refusal precedes xs=>7 body entry");
    }
}

fn nested_row_fixture(
    strings: bool,
    healthy: bool,
) -> (
    &'static str,
    &'static str,
    &'static str,
    Vec<super::Value>,
    super::Value,
) {
    use fipe::value::Value;
    if strings {
        (
            NESTED_ROW_MAIN_STRING,
            NESTED_ROW_HOST_STRING,
            "String",
            if healthy {
                vec![Value::text("a"), Value::text("b")]
            } else {
                vec![Value::text("b"), Value::text("a")]
            },
            Value::text("tag"),
        )
    } else {
        (
            NESTED_ROW_MAIN_INTEGER,
            NESTED_ROW_HOST_INTEGER,
            "Integer",
            if healthy {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            },
            Value::Int(7),
        )
    }
}

fn nested_row_program(source: &str, host: &str) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("methods", NESTED_ROW_METHODS).unwrap();
    declarations.module("host", host).unwrap();
    let module = parse(lex(source)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    nested_row_metadata(&typed, &program);
    program
}

fn nested_row_test(machine: &Machine) -> (Option<usize>, Option<usize>) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        let tests: Vec<_> = tables
            .tests
            .iter()
            .enumerate()
            .filter_map(|(index, test)| {
                (tables.namespaces[test.namespace] == "methods"
                    && NESTED_ROW_METHODS.get(test.span.start..test.span.end)
                        == Some("is_sorted self"))
                .then_some(index)
            })
            .collect();
        match tests.as_slice() {
            [] => (None, None),
            [test] => (Some(*test), joined.tests[*test]),
            _ => panic!("ambiguous methods Sorted source tests: {tests:?}"),
        }
    }
}

fn nested_row_host_reply(
    label: &str,
    stored_function: Option<&super::Value>,
    values: &[super::Value],
) -> super::Value {
    use fipe::value::Value;
    match label {
        "after" => Value::Int(7),
        "recall" => {
            let value =
                stored_function.expect("actual op input stored before recall");
            println!("NESTED_ROW_RECALL exact_stored={value:?}");
            value.clone()
        }
        "arg" => Value::seq(values.to_vec()),
        _ => panic!("unexpected callback"),
    }
}

fn nested_row_function_input_case(strings: bool, healthy: bool) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::{Frame, Stop},
    };
    let (source, host, meaning, values, marker) =
        nested_row_fixture(strings, healthy);
    println!(
        "NESTED_ROW_CASE meaning={meaning} healthy={healthy} source={source:?}"
    );
    let program = nested_row_program(source, host);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let (test, test_body) = nested_row_test(&machine);
    println!("NESTED_ROW_JOINED_TEST test={test:?} body={test_body:?}");
    let mut tested = false;
    let mut lambda_body = None;
    let mut stored_function = None;
    let mut lambda_entered = false;
    let mut calls = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if let Some(Frame::Body(frame)) = machine.frames().last() {
            if Some(frame.body) == lambda_body {
                lambda_entered = true;
            }
            if Some(frame.body) == test_body {
                observe_nested_row_test(&machine, frame, test, meaning);
                tested = true;
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let joined = machine.program().unwrap();
                let (namespace, label) = {
                    let tables = joined.tables.borrow();
                    let declaration = &tables.declarations[declaration];
                    (
                        tables.namespaces[declaration.namespace].clone(),
                        declaration.label.clone(),
                    )
                };
                println!("NESTED_ROW_CALLBACK namespace={namespace} label={label} arguments={arguments:?}");
                let reply = if namespace == "methods" {
                    assert_eq!(label, "op");
                    let function = nested_row_received_function(
                        &machine,
                        meaning,
                        declaration,
                        &arguments,
                        &marker,
                    );
                    lambda_body = Some(function.body);
                    stored_function = Some(Value::Function(function));
                    Value::Int(7)
                } else {
                    assert_eq!(namespace, "host");
                    assert_eq!(arguments, [Value::Unit]);
                    nested_row_host_reply(
                        &label,
                        stored_function.as_ref(),
                        &values,
                    )
                };
                calls.push(format!("{namespace}.{label}"));
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                println!("NESTED_ROW_DONE meaning={meaning} healthy={healthy} result={result:?} calls={calls:?} tested={tested} lambda_entered={lambda_entered} charged={}", machine.charged());
                assert!(
                    lambda_body.is_some(),
                    "Host recalled the exact actual request Function"
                );
                assert_nested_row_result(
                    &machine,
                    healthy,
                    result,
                    &calls,
                    test,
                    tested,
                    lambda_entered,
                );
                return;
            }
            Stop::Request(other) => panic!("typed callbacks only: {other:?}"),
        }
    }
    panic!("bounded nested function-input observer must finish");
}

#[test]
fn declared_nested_function_retrieval_integer_healthy_diagnostic() {
    nested_row_function_input_case(false, true);
}
#[test]
fn declared_nested_function_retrieval_integer_reversed_diagnostic() {
    nested_row_function_input_case(false, false);
}
#[test]
fn declared_nested_function_retrieval_string_healthy_diagnostic() {
    nested_row_function_input_case(true, true);
}
#[test]
fn declared_nested_function_retrieval_string_reversed_diagnostic() {
    nested_row_function_input_case(true, false);
}

const OWNED_GUARD_HOST: &str =
    "pub items: a -> Vec<a>\npub after: () -> Integer\n";
const OWNED_GUARD_INTEGER: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\napply = f x => f x\ninvoke: Ord a => a -> (Sorted<a> -> Integer) -> Integer = seed fn => apply fn (host.items seed)\npass = value consumer => consumer value\ngo = seed => {\n callback = invoke seed\n forward = fn => fn (xs => host.after ())\n pass callback forward\n}\ngo 7\n";
const OWNED_GUARD_STRING: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\napply = f x => f x\ninvoke: Ord a => a -> (Sorted<a> -> Integer) -> Integer = seed fn => apply fn (host.items seed)\npass = value consumer => consumer value\ngo = seed => {\n callback = invoke seed\n forward = fn => fn (xs => host.after ())\n pass callback forward\n}\ngo \"tag\"\n";

#[derive(Default, Debug)]
struct OwnedGuardAttachments {
    outer: bool,
    inner: bool,
    original_callback_alias: bool,
}

#[derive(Default, Debug)]
struct OwnedGuardExecution {
    tested: bool,
    lambda_entered: bool,
}

#[derive(Default, Debug)]
struct OwnedGuardObservation {
    attachments: OwnedGuardAttachments,
    execution: OwnedGuardExecution,
    errors: Vec<String>,
    creator_ids: Vec<(usize, usize)>,
    origin: Option<usize>,
    outer_guard: Option<fipe::value::Guard>,
    original_inner: Option<std::rc::Rc<fipe::value::Closure>>,
}

fn owned_guard_operand(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    operand: fipe::mir::Operand,
) -> Option<fipe::value::Value> {
    use fipe::mir::Operand;
    match operand {
        Operand::Local(slot) => frame.locals.get(slot).cloned(),
        Operand::Capture(slot) => frame.captures.get(slot).cloned(),
        Operand::Global(definition) => {
            machine.globals().find_map(|(index, value)| {
                (index == definition).then(|| value.clone())
            })
        }
        Operand::Const(_) | Operand::Hidden(_) => None,
    }
}

fn owned_guard_creator_associations(
    tables: &fipe::ir::Tables,
    guard: &fipe::value::Guard,
    demanded: &[usize],
    meaning: &str,
    observed: &mut OwnedGuardObservation,
) -> bool {
    use fipe::ir::Type;
    let mut valid = true;
    for &id in demanded {
        let matches: Vec<_> = guard
            .witnesses
            .iter()
            .filter(|(captured, _)| *captured == id)
            .collect();
        if matches.len() != 1 {
            observed.errors.push(format!(
                "creator ID{id} requires one association, got {}",
                matches.len()
            ));
            valid = false;
            continue;
        }
        let Type::Meaning { index, arguments } = matches[0].1.as_type() else {
            observed.errors.push(format!(
                "actual association ID{id} is not nominal: {:?}",
                matches[0].1.as_type()
            ));
            valid = false;
            continue;
        };
        if !arguments.is_empty()
            || tables.namespaces[tables.meanings[*index].namespace] != "prelude"
            || tables.meanings[*index].label != meaning
        {
            observed.errors.push(format!(
                "association ID{id} expected {meaning}, got {:?}",
                matches[0].1.as_type()
            ));
            valid = false;
        }
        let creator: Vec<_> = observed
            .creator_ids
            .iter()
            .filter_map(|(known, body)| (*known == id).then_some(*body))
            .collect();
        if creator.is_empty() {
            observed.errors.push(format!("association ID{id} has no earlier actual creator hidden/meaning snapshot"));
            valid = false;
        }
        println!("OWNED_GUARD_CREATOR id={id} creator_bodies={creator:?} meaning={:?}", matches[0].1.as_type());
    }
    valid
}

fn owned_guard_association(
    machine: &Machine,
    function: &fipe::value::Closure,
    test: usize,
    meaning: &str,
    leading_function: bool,
    observed: &mut OwnedGuardObservation,
) -> bool {
    use fipe::ir::{Predicate, Type};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let guards: Vec<_> = function
        .guards
        .iter()
        .filter(|guard| {
            Some(guard.entry) == observed.origin
                && bound_order_test_ids(&guard.predicate).contains(&test)
        })
        .collect();
    println!("OWNED_GUARD_ASSOC body={} expected_leading_function={leading_function} guards={:?}", function.body, function.guards);
    if guards.is_empty() {
        observed.errors.push(format!("missing semantic guard body={} leading_function={leading_function}", function.body));
        return false;
    }
    let mut valid = true;
    for guard in guards {
        if matches!(guard.predicate, Predicate::Function { .. })
            != leading_function
        {
            observed.errors.push(format!("guard leading Function shape must match expected binding level: {:?}", guard.predicate));
            valid = false;
        }
        let mut pending = vec![&guard.predicate];
        let mut demanded = Vec::new();
        while let Some(predicate) = pending.pop() {
            match predicate {
                Predicate::All(parts) | Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                Predicate::Not(inner) => pending.push(inner),
                Predicate::Function { parameter, result } => {
                    pending.push(result);
                    pending.push(parameter);
                }
                Predicate::Test { index, meanings } if *index == test => {
                    for actual in meanings {
                        if let Type::Parameter(id) = actual {
                            demanded.push(*id);
                        } else {
                            observed.errors.push(format!("creator template must retain actual Parameter identity, got {actual:?}"));
                            valid = false;
                        }
                    }
                }
                Predicate::Test { .. } | Predicate::Nonzero { .. } => {}
            }
        }
        if demanded.is_empty() {
            observed.errors.push(
                "generic semantic guard has no demanded parameter IDs".into(),
            );
            valid = false;
        }
        valid &= owned_guard_creator_associations(
            &tables, guard, &demanded, meaning, observed,
        );
    }
    valid
}

fn observe_owned_test(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    test: Option<usize>,
    test_body: Option<usize>,
    meaning: &str,
    observed: &mut OwnedGuardObservation,
) -> bool {
    let joined = machine.program().unwrap();
    let body = &joined.bodies[frame.body];
    if Some(frame.body) == test_body {
        let Some(test) = test else { return false };
        let tables = joined.tables.borrow();
        let ids = &tables.tests[test].parameters;
        if ids.is_empty() {
            observed
                .errors
                .push("generic source Test must declare a parameter".into());
        }
        for id in ids {
            let actual = body
                .hidden
                .iter()
                .position(|parameter| parameter == id)
                .and_then(|position| frame.meanings.get(position));
            match actual {
                Some(fipe::ir::Type::Meaning { index, arguments }) if arguments.is_empty() && tables.namespaces[tables.meanings[*index].namespace] == "prelude" && tables.meanings[*index].label == meaning => (),
                _ => observed.errors.push(format!("actual Test parameter ID{id} expected {meaning}, got {actual:?}")),
            }
        }
        observed.execution.tested = true;
    }
    true
}

fn observe_owned_lambda(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    source: &str,
    observed: &mut OwnedGuardObservation,
) {
    use fipe::mir::{Callee, Terminator};
    let joined = machine.program().unwrap();
    let body = &joined.bodies[frame.body];
    {
        let tables = joined.tables.borrow();
        if body.blocks.iter().any(|block| {
            let Terminator::Call {
                callee: Callee::Host(declaration),
                ..
            } = &block.terminator
            else {
                return false;
            };
            let declaration = &tables.declarations[*declaration];
            let Some(node) =
                block.nodes.last().map(|node| &tables.nodes[*node])
            else {
                return false;
            };
            tables.namespaces[declaration.namespace] == "host"
                && declaration.label == "after"
                && tables.namespaces[node.namespace] == "main"
                && source.get(node.span.start..node.span.end)
                    == Some("host.after ()")
        }) {
            observed.execution.lambda_entered = true;
        }
    }
}

fn observe_owned_outer(
    machine: &Machine,
    frame: &fipe::vm::BodyFrame,
    function: &std::rc::Rc<fipe::value::Closure>,
    args: &[fipe::mir::Operand],
    test: usize,
    valid: bool,
    observed: &mut OwnedGuardObservation,
) {
    use fipe::{value::Value, vm::Frame};
    observed.attachments.outer |= valid;
    observed.outer_guard = function
        .guards
        .iter()
        .find(|guard| {
            Some(guard.entry) == observed.origin
                && bound_order_test_ids(&guard.predicate).contains(&test)
        })
        .cloned();
    if args.len() == 1 {
        if let Some(Value::Function(original)) =
            owned_guard_operand(machine, frame, args[0])
        {
            println!(
                "OWNED_GUARD_ORIGINAL_INNER body={} guards={:?}",
                original.body, original.guards
            );
            if !original.guards.is_empty() {
                observed.errors.push(
                    "unsigned inner lambda must start without guards".into(),
                );
            }
            observed.original_inner = Some(original);
        }
    }
    for prior in machine.frames() {
        if let Frame::Body(prior) = prior {
            for value in &prior.locals {
                if let Value::Function(alias) = value {
                    if alias.body == function.body
                        && alias.guards.is_empty()
                        && !std::rc::Rc::ptr_eq(alias, function)
                    {
                        observed.attachments.original_callback_alias = true;
                    }
                }
            }
        }
    }
}

fn observe_owned_inner(
    function: &std::rc::Rc<fipe::value::Closure>,
    test: usize,
    valid: bool,
    observed: &mut OwnedGuardObservation,
) {
    let same_entry = function.guards.iter().find(|guard| {
        Some(guard.entry) == observed.origin
            && bound_order_test_ids(&guard.predicate).contains(&test)
    });
    let same_witnesses = same_entry
        .zip(observed.outer_guard.as_ref())
        .is_some_and(|(inner, outer)| {
            std::rc::Rc::ptr_eq(&inner.witnesses, &outer.witnesses)
        });
    let private_alias =
        observed.original_inner.as_ref().is_some_and(|original| {
            original.body == function.body
                && original.guards.is_empty()
                && !std::rc::Rc::ptr_eq(original, function)
        });
    observed.attachments.inner |= valid && same_witnesses && private_alias;
    if !same_witnesses {
        observed.errors.push(
            "derived Sorted guard must retain exact outer-entry witness Rc"
                .into(),
        );
    }
    if !private_alias {
        observed.errors.push("derived inner function must be private samebody copy of original unsigned lambda".into());
    }
}

fn observe_owned_guard(
    machine: &Machine,
    source: &str,
    test: Option<usize>,
    test_body: Option<usize>,
    meaning: &str,
    observed: &mut OwnedGuardObservation,
) {
    use fipe::{
        mir::{Callee, Terminator},
        value::Value,
        vm::Frame,
    };
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return;
    };
    let joined = machine.program().unwrap();
    let body = &joined.bodies[frame.body];
    if !observe_owned_test(machine, frame, test, test_body, meaning, observed) {
        return;
    }
    observe_owned_lambda(machine, frame, source, observed);
    let block = &body.blocks[frame.block];
    if frame.statement != block.statements.len() {
        return;
    }
    let Terminator::Call {
        callee: Callee::Value(operand),
        args,
        ..
    } = &block.terminator
    else {
        return;
    };
    let Some(node) = block.nodes.last() else {
        return;
    };
    let expression_source = {
        let tables = joined.tables.borrow();
        let node = &tables.nodes[*node];
        if tables.namespaces[node.namespace] != "main" {
            return;
        }
        source.get(node.span.start..node.span.end).unwrap_or("")
    };
    let outer = match expression_source {
        "fn (xs => host.after ())" => true,
        "f x" => false,
        _ => return,
    };
    println!("OWNED_GUARD_VALUE_CALL text={expression_source:?} frame={} operand={operand:?} args={args:?}", frame.body);
    let Some(Value::Function(function)) =
        owned_guard_operand(machine, frame, *operand)
    else {
        observed.errors.push(format!(
            "actual {expression_source} operand is not a Function"
        ));
        return;
    };
    let Some(test) = test else {
        observed
            .errors
            .push("actual source Test catalog missing at attachment".into());
        return;
    };
    let valid = args.len() == 1
        && owned_guard_association(
            machine, &function, test, meaning, outer, observed,
        );
    if outer {
        observe_owned_outer(
            machine, frame, &function, args, test, valid, observed,
        );
    } else {
        observe_owned_inner(&function, test, valid, observed);
    }
}

fn snapshot_owned_creators(
    machine: &Machine,
    meaning: &str,
    observed: &mut OwnedGuardObservation,
) {
    use fipe::{ir::Type, vm::Frame};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    for frame in machine.frames() {
        let Frame::Body(frame) = frame else { continue };
        for (id, actual) in
            joined.bodies[frame.body].hidden.iter().zip(&frame.meanings)
        {
            let Type::Meaning { index, arguments } = actual else {
                continue;
            };
            if arguments.is_empty()
                && tables.namespaces[tables.meanings[*index].namespace]
                    == "prelude"
                && tables.meanings[*index].label == meaning
                && !observed.creator_ids.contains(&(*id, frame.body))
            {
                println!("OWNED_GUARD_CREATOR_SNAPSHOT id={id} body={} meaning={actual:?}", frame.body);
                observed.creator_ids.push((*id, frame.body));
            }
        }
    }
}

fn owned_binder_diagnostic(
    module: &fipe::ast::Module,
    typed: &fipe::typed::Typed,
    source: &str,
    argument: Option<fipe::ast::ExprId>,
    argument_span: fipe::token::Span,
) -> fipe::token::Span {
    for entry in &module.entries {
        let fipe::ast::Entry::Line(id) = entry else {
            continue;
        };
        let Some(expression) = module.expr(*id) else {
            continue;
        };
        let fipe::ast::ExprKind::Bind { pattern, value, .. } = &expression.kind
        else {
            continue;
        };
        let pattern_node = module.pattern(*pattern).unwrap();
        let fipe::ast::PatternKind::Name(name) = &pattern_node.kind else {
            continue;
        };
        if name.segments.last().map(String::as_str) != Some("invoke") {
            continue;
        }
        println!("OWNED_DIAG_INVOKE binding={id:?} site={:?} scheme={:?} body_parameters={:?} body_type={:?}", pattern_node.span, typed.binding(*id), typed.body_parameters.get(&(pattern_node.span.start, pattern_node.span.end)), typed.expr(*value));
        if let Some(fipe::ast::Expr {
            kind: fipe::ast::ExprKind::Arms(arms),
            ..
        }) = module.expr(*value)
        {
            for arm in arms {
                for parameter in &arm.patterns {
                    let pattern = module.pattern(*parameter).unwrap();
                    println!("OWNED_DIAG_PARAM source={:?} pattern={parameter:?} site={:?} body_type={:?}", source.get(pattern.span.start..pattern.span.end), pattern.span, typed.pattern(*parameter));
                }
            }
        }
    }
    let fn_start =
        source.find("apply fn (host.items seed)").unwrap() + "apply ".len();
    let fn_span = fipe::token::Span {
        start: fn_start,
        end: fn_start + "fn".len(),
    };
    let fn_expression = match module.at(fn_start) {
        Some(fipe::ast::Node::Expr(id)) => Some(id),
        _ => None,
    };
    println!("OWNED_DIAG_FN_REF expression={fn_expression:?} span={fn_span:?} known={:?}", fn_expression.and_then(|id| typed.known(id)));
    for check in &typed.checks {
        if check.place == fn_span
            || check.place == argument_span
            || check.value.is_some_and(|id| {
                Some(id) == argument || Some(id) == fn_expression
            })
        {
            println!("OWNED_DIAG_RELEVANT_CHECK value={:?} place={:?} receiving_call={:?} target={:?} full={check:?}", check.value, check.place, check.receiving_call, check.target);
        }
    }
    fn_span
}

fn owned_ir_binder_diagnostic(
    program: &fipe::mir::Program,
    fn_span: fipe::token::Span,
    argument_span: fipe::token::Span,
) {
    let tables = program.tables.borrow();
    for (index, node) in tables.nodes.iter().enumerate() {
        if tables.namespaces[node.namespace] == "main"
            && (node.span == fn_span || node.span == argument_span)
        {
            println!(
                "OWNED_DIAG_IR_NODE index={index} span={:?} ty={:?} kind={:?}",
                node.span, node.ty, node.kind
            );
        }
    }
    for (index, entry) in tables.entries.iter().enumerate() {
        if tables.namespaces[entry.namespace] == "main"
            && (entry.span == fn_span || entry.span == argument_span)
        {
            println!(
                "OWNED_DIAG_IR_ENTRY index={index} span={:?} predicate={:?}",
                entry.span, entry.predicate
            );
        }
    }
    for (index, definition) in tables.definitions.iter().enumerate() {
        if tables.namespaces[definition.namespace] == "main"
            && definition.label == "invoke"
        {
            println!("OWNED_DIAG_INVOKE_IR definition={index} parameters={:?} hidden={:?} ty={:?} actual_body={:?}", definition.parameters, definition.hidden, definition.ty, program.definitions[index]);
            if let Some(body) = program.definitions[index] {
                println!("OWNED_DIAG_INVOKE_BODY index={body} hidden={:?} locals={:?}", program.bodies[body].hidden, program.bodies[body].locals);
            }
        }
    }
}

fn owned_guard_producer(
    source: &str,
) -> (fipe::mir::Program, bool, fipe::token::Span) {
    let mut declarations = Declarations::new();
    declarations.module("host", OWNED_GUARD_HOST).unwrap();
    let module = parse(lex(source)).expect("signed public source syntax");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("signed public producer resolution");
    let typed = fipe::infer::infer(&module, &resolved, &declarations, &[])
        .expect("signed public producer inference");
    let call_start = source.find("pass callback forward").unwrap();
    let call_span = fipe::token::Span {
        start: call_start,
        end: call_start + "pass callback forward".len(),
    };
    let argument_span = fipe::token::Span {
        start: call_start + "pass ".len(),
        end: call_start + "pass callback".len(),
    };
    let argument = match module.at(argument_span.start) {
        Some(fipe::ast::Node::Expr(id))
            if module
                .expr(id)
                .is_some_and(|expr| expr.span == argument_span) =>
        {
            Some(id)
        }
        _ => None,
    };
    let matching_checks: Vec<_> = typed
        .checks
        .iter()
        .filter(|check| {
            check.value == argument
                && argument.is_some()
                && check.place == argument_span
        })
        .collect();
    println!("OWNED_GUARD_TYPED_ORIGIN argument={argument:?} argument_span={argument_span:?} call={call_span:?} checks={matching_checks:?}");
    let typed_origin = matching_checks.iter().any(|check| matches!(&check.target, Some(fipe::types::Type::Function { parameter, .. }) if matches!(parameter.as_ref(), fipe::types::Type::Function { .. })));
    let fn_span = owned_binder_diagnostic(
        &module,
        &typed,
        source,
        argument,
        argument_span,
    );
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("signed public producer admission");
    let program = compile::compile(&unit);
    owned_ir_binder_diagnostic(&program, fn_span, argument_span);
    (program, typed_origin, argument_span)
}

fn owned_guard_catalog(
    machine: &Machine,
    source: &str,
    observed: &mut OwnedGuardObservation,
) -> (Option<usize>, Option<usize>) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
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
        let test = (tests.len() == 1).then(|| tests[0]);
        let body =
            test.and_then(|test| joined.tests.get(test).copied().flatten());
        println!("OWNED_GUARD_CATALOG tests={tests:?} selected={test:?} compiled_body={body:?}");
        if test.is_none() {
            observed.errors.push(format!(
                "require one semantic source Test, got {tests:?}"
            ));
        }
        if body.is_none() {
            observed.errors.push(
                "actual semantic Test body absent before execution".into(),
            );
        }
        (test, body)
    }
}

fn owned_guard_origin(
    machine: &Machine,
    argument_span: fipe::token::Span,
    test: Option<usize>,
    typed_origin: bool,
    observed: &mut OwnedGuardObservation,
) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        let entries: std::collections::BTreeSet<_> = tables.nodes.iter().filter_map(|node| {
            let fipe::ir::Kind::Enter { entry, value } = &node.kind else { return None };
            let value = &tables.nodes[*value];
            let nested = matches!(&tables.entries[*entry].predicate, fipe::ir::Predicate::Function { parameter, .. } if matches!(parameter.as_ref(), fipe::ir::Predicate::Function { .. }));
            (tables.namespaces[value.namespace] == "main" && value.span == argument_span && nested
                && test.is_some_and(|test| bound_order_test_ids(&tables.entries[*entry].predicate).contains(&test))).then_some(*entry)
        }).collect::<std::collections::BTreeSet<_>>();
        println!("OWNED_GUARD_IR_ORIGIN typed_origin={typed_origin} entries={entries:?}");
        if entries.len() == 1 {
            observed.origin = entries.iter().copied().next();
        } else {
            observed.errors.push(format!("require one actual nested Function entry at pass callback argument, got {entries:?}"));
        }
        if !typed_origin {
            observed.errors.push("missing typed Check target/receiving-call at pass callback argument".into());
        }
    }
}

fn assert_owned_guard_result(
    machine: &Machine,
    sorted: bool,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
    test: Option<usize>,
    typed_origin: bool,
    observed: &OwnedGuardObservation,
) {
    use fipe::value::Value;
    if sorted {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert_eq!(calls, ["items", "after"]);
        assert!(
            observed.execution.lambda_entered,
            "healthy actual lambda must execute"
        );
    } else {
        let fault = result.unwrap_err();
        let Reason::Failed { entry, .. } = fault.reason else {
            panic!("actual Sorted refusal: {fault:?}")
        };
        assert_eq!(Some(entry), observed.origin, "negative must fail at original pass-bound Function entry, not independent apply check");
        assert!(
            test.is_some_and(|test| bound_order_test_ids(
                &machine.program().unwrap().tables.borrow().entries[entry]
                    .predicate
            )
            .contains(&test)),
            "same source Test owns negative entry"
        );
        assert_eq!(calls, ["items"]);
        assert!(
            !observed.execution.lambda_entered,
            "negative check precedes original lambda body"
        );
    }
    assert!(
        typed_origin && observed.origin.is_some(),
        "source-owned first entry must be real: {observed:?}"
    );
    assert!(
        observed.attachments.outer && observed.attachments.inner,
        "both actual attachment levels required: {observed:?}"
    );
    assert!(
        observed.execution.tested,
        "actual semantic Test must execute: {observed:?}"
    );
    assert!(
        observed.errors.is_empty(),
        "actual producer/capture identity errors: {observed:?}"
    );
    assert!(
        observed
            .original_inner
            .as_ref()
            .is_some_and(|original| original.guards.is_empty()),
        "actual retained unsigned lambda alias must remain unchanged"
    );
    // The outer callback original alias remains separately reported.
}

fn owned_guard_case(meaning: &str, sorted: bool) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::Stop,
    };
    let source = if meaning == "Integer" {
        OWNED_GUARD_INTEGER
    } else {
        OWNED_GUARD_STRING
    };
    let (program, typed_origin, argument_span) = owned_guard_producer(source);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let mut observed = OwnedGuardObservation::default();
    let (test, test_body) =
        owned_guard_catalog(&machine, source, &mut observed);
    owned_guard_origin(
        &machine,
        argument_span,
        test,
        typed_origin,
        &mut observed,
    );
    let mut calls = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(stop, Stop::Before(_)) {
            snapshot_owned_creators(&machine, meaning, &mut observed);
            observe_owned_guard(
                &machine,
                source,
                test,
                test_body,
                meaning,
                &mut observed,
            );
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => (),
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let joined = machine.program().unwrap();
                let tables = joined.tables.borrow();
                let declaration = &tables.declarations[declaration];
                assert_eq!(tables.namespaces[declaration.namespace], "host");
                let label = declaration.label.clone();
                let reply = match label.as_str() {
                    "items" => {
                        let mut items = if meaning == "Integer" {
                            assert_eq!(arguments, [Value::Int(7)]);
                            vec![Value::Int(1), Value::Int(2)]
                        } else {
                            assert_eq!(arguments, [Value::text("tag")]);
                            vec![Value::text("a"), Value::text("b")]
                        };
                        if !sorted {
                            items.reverse();
                        }
                        Value::seq(items)
                    }
                    "after" => {
                        assert_eq!(arguments, [Value::Unit]);
                        Value::Int(7)
                    }
                    _ => panic!("unexpected recursive request {label}"),
                };
                calls.push(label);
                drop(tables);
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Request(other) => {
                panic!("unexpected recursive request {other:?}")
            }
            Stop::Done(result) => {
                println!("OWNED_GUARD_TERMINAL meaning={meaning} sorted={sorted} result={result:?} calls={calls:?} observed={observed:?}");
                assert_owned_guard_result(
                    &machine,
                    sorted,
                    result,
                    &calls,
                    test,
                    typed_origin,
                    &observed,
                );
                return;
            }
        }
    }
    panic!("bounded recursive producer must finish")
}

#[test]
fn owned_signed_function_guards_keep_integer_sorted_input() {
    owned_guard_case("Integer", true);
}
#[test]
fn owned_signed_function_guards_refuse_integer_reversed_input() {
    owned_guard_case("Integer", false);
}
#[test]
fn owned_signed_function_guards_keep_string_sorted_input() {
    owned_guard_case("String", true);
}
#[test]
fn owned_signed_function_guards_refuse_string_reversed_input() {
    owned_guard_case("String", false);
}

const DEPTH2_ROW_METHODS: &str = "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\npub type Sorted<T: Ord>: Vec<T> where is_sorted self\npub type M<T: Ord> { pub token: T }\nimpl M { op: ((Sorted<T> -> Integer) -> Integer) -> (self: Self) -> Integer }\n";
const DEPTH2_ROW_HOST_INTEGER: &str = "pub after: () -> Integer\npub arg: () -> Vec<Integer>\npub recall: () -> a\n";
const DEPTH2_ROW_HOST_STRING: &str = "pub after: () -> Integer\npub arg: () -> Vec<String>\npub recall: () -> a\n";
const DEPTH2_ROW_MAIN_INTEGER: &str = "use host\nuse methods\nget = m => m.op\nrelay = get (methods.M 7)\nop_result = relay (fn => fn (host.arg ()))\nwrapped: (Vec<Integer> -> Integer) -> Integer = host.recall ()\nwrapped (xs => host.after ())\n";
const DEPTH2_ROW_MAIN_STRING: &str = "use host\nuse methods\nget = m => m.op\nrelay = get (methods.M \"tag\")\nop_result = relay (fn => fn (host.arg ()))\nwrapped: (Vec<String> -> Integer) -> Integer = host.recall ()\nwrapped (xs => host.after ())\n";

type GuardWitnesses = std::rc::Rc<[(usize, std::rc::Rc<fipe::value::Meaning>)]>;

#[derive(Default, Debug)]
struct DeclaredDepth2Attachments {
    outer: bool,
    inner: bool,
    same_creator: bool,
}

#[derive(Default, Debug)]
struct DeclaredDepth2Execution {
    tested: bool,
    inner_entered: bool,
}

#[derive(Default, Debug)]
struct DeclaredDepth2Observation {
    entry: Option<usize>,
    witnesses: Option<GuardWitnesses>,
    original_inner: Option<super::Value>,
    attachments: DeclaredDepth2Attachments,
    execution: DeclaredDepth2Execution,
    errors: Vec<String>,
}

fn declared_depth2_witness(
    guard: &fipe::value::Guard,
    tables: &fipe::ir::Tables,
    test: usize,
    meaning: &str,
) -> Result<Vec<usize>, String> {
    use fipe::ir::{Predicate, Type};
    let mut pending = vec![&guard.predicate];
    let mut parameters = Vec::new();
    let mut matched = false;
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                pending.extend(parts);
            }
            Predicate::Not(inner) => pending.push(inner),
            Predicate::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            Predicate::Test { index, meanings } if *index == test => {
                matched = true;
                for ty in meanings {
                    let Type::Parameter(parameter) = ty else {
                        return Err(format!(
                            "unexpected source Test prototype {ty:?}"
                        ));
                    };
                    let witness = guard
                        .witnesses
                        .iter()
                        .find(|(known, _)| known == parameter)
                        .ok_or_else(|| {
                            format!("missing creator parameter {parameter}")
                        })?;
                    let Type::Meaning { index, arguments } =
                        witness.1.as_type()
                    else {
                        return Err("creator witness is not concrete".into());
                    };
                    if !arguments.is_empty()
                        || tables.namespaces[tables.meanings[*index].namespace]
                            != "prelude"
                        || tables.meanings[*index].label != meaning
                    {
                        return Err(format!(
                            "wrong creator witness {:?}",
                            witness.1.as_type()
                        ));
                    }
                    if !parameters.contains(parameter) {
                        parameters.push(*parameter);
                    }
                }
            }
            _ => {}
        }
    }
    if !matched || parameters.is_empty() {
        return Err("source Sorted Test has no captured binder".into());
    }
    Ok(parameters)
}

fn declared_depth2_outer(
    machine: &Machine,
    declaration: usize,
    arguments: &[super::Value],
    test: Option<usize>,
    meaning: &str,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::{ir, value::Value};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let [Value::Function(function), Value::Record(receiver)] = arguments else {
        observed
            .errors
            .push("actual op must receive Function/self".into());
        return;
    };
    let candidates: Vec<_> = tables.dispatch.iter().enumerate().flat_map(|(member, dispatch)| dispatch.rows.iter().enumerate().map(move |(row, value)| (member, row, value)))
        .filter(|(_, _, row)| matches!(&row.target, ir::Implementation::Declared(index) if *index == declaration)
            && matches!(&row.receiver, ir::Type::Meaning { index, .. } if *index == receiver.meaning)).collect();
    println!(
        "DEPTH2_ROW_CANDIDATES {:?} received_body={} guards={:?}",
        candidates, function.body, function.guards
    );
    let Some(test) = test else {
        observed
            .errors
            .push("source Test absent while observing outer guard".into());
        return;
    };
    let found = function.guards.iter().find_map(|guard| {
        if guard.spot != function.filled.len()
            || !matches!(&guard.predicate, ir::Predicate::Function { .. })
            || !bound_order_test_ids(&guard.predicate).contains(&test)
        {
            return None;
        }
        candidates
            .iter()
            .find(|(_, _, row)| row.input_contract == Some(guard.entry))
            .map(|row| (guard, row))
    });
    let Some((guard, (member, row_index, row))) = found else {
        observed.errors.push(
            "missing row-owned leading Function guard on actual op input"
                .into(),
        );
        return;
    };
    println!("DEPTH2_ROW_OUTER member={member} row={row_index} entry={} receiver={:?} witness={:?}", guard.entry, row.receiver, guard.witnesses);
    match declared_depth2_witness(guard, &tables, test, meaning) {
        Ok(parameters) => {
            let ir::Type::Meaning { arguments, .. } = &row.receiver else {
                unreachable!()
            };
            let receiver_parameters: Vec<_> = arguments
                .iter()
                .filter_map(|ty| {
                    if let ir::Type::Parameter(id) = ty {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            if parameters
                .iter()
                .any(|id| !receiver_parameters.contains(id))
            {
                observed.errors.push(
                    "Test binder not the actual row receiver binder".into(),
                );
            }
        }
        Err(error) => observed.errors.push(error),
    }
    observed.entry = Some(guard.entry);
    observed.witnesses = Some(guard.witnesses.clone());
    observed.attachments.outer = true;
}

fn observe_depth2_original_inner(
    value: Option<&super::Value>,
    observed: &mut DeclaredDepth2Observation,
) -> bool {
    use fipe::value::Value;
    let Some(Value::Function(function)) = value else {
        observed
            .errors
            .push("original unsigned inner operand absent".into());
        return false;
    };
    println!(
        "DEPTH2_ROW_ORIGINAL_INNER body={} guards={:?}",
        function.body, function.guards
    );
    if !function.guards.is_empty() {
        observed.errors.push(
            "unsigned inner already guarded before recursive binding".into(),
        );
    }
    observed.original_inner = Some(Value::Function(function.clone()));
    true
}

fn observe_depth2_inner_guard(
    tables: &fipe::ir::Tables,
    function: &std::rc::Rc<fipe::value::Closure>,
    test: Option<usize>,
    meaning: &str,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::value::Value;
    println!(
        "DEPTH2_ROW_INNER body={} guards={:?}",
        function.body, function.guards
    );
    let Some(Value::Function(original)) = &observed.original_inner else {
        observed
            .errors
            .push("original inner body association absent".into());
        return;
    };
    if function.body != original.body {
        observed
            .errors
            .push("recursive binding changed inner body".into());
    }
    if std::rc::Rc::ptr_eq(function, original) {
        observed.errors.push(
            "recursive binding did not produce a distinct Function Rc".into(),
        );
    }
    if !original.guards.is_empty() {
        observed.errors.push("original inner alias mutated".into());
    }
    let (Some(entry), Some(test)) = (observed.entry, test) else {
        observed
            .errors
            .push("outer row entry/source Test absent for inner guard".into());
        return;
    };
    let found = function.guards.iter().find(|guard| {
        guard.entry == entry
            && guard.spot == function.filled.len()
            && !matches!(&guard.predicate, fipe::ir::Predicate::Function { .. })
            && bound_order_test_ids(&guard.predicate).contains(&test)
    });
    let Some(guard) = found else {
        observed
            .errors
            .push("missing recursively derived same-row Sorted guard".into());
        return;
    };
    if let Err(error) = declared_depth2_witness(guard, tables, test, meaning) {
        observed.errors.push(error);
    }
    observed.attachments.same_creator =
        observed.witnesses.as_ref().is_some_and(|witnesses| {
            std::rc::Rc::ptr_eq(witnesses, &guard.witnesses)
        });
    if !observed.attachments.same_creator {
        observed.errors.push(
            "inner guard did not retain exact outer creator witnesses".into(),
        );
    }
    observed.attachments.inner = true;
}

fn declared_depth2_before_call(
    machine: &Machine,
    source: &str,
    test: Option<usize>,
    meaning: &str,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::{
        mir::{Callee, Operand, Terminator},
        value::Value,
        vm::Frame,
    };
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return;
    };
    let joined = machine.program().unwrap();
    let block = &joined.bodies[frame.body].blocks[frame.block];
    if frame.statement != block.statements.len() {
        return;
    }
    let Terminator::Call {
        callee: Callee::Value(callee),
        args,
        ..
    } = &block.terminator
    else {
        return;
    };
    let Some(node) = block.nodes.last() else {
        return;
    };
    let tables = joined.tables.borrow();
    let node = &tables.nodes[*node];
    if tables.namespaces[node.namespace] != "main" {
        return;
    }
    let value = |operand: &Operand| match operand {
        Operand::Local(index) => frame.locals.get(*index),
        Operand::Capture(index) => frame.captures.get(*index),
        Operand::Const(index) => joined.constants.get(*index),
        Operand::Global(index) => machine
            .globals()
            .find_map(|(known, value)| (known == *index).then_some(value)),
        // Hidden operands are meanings, not callable Values; report their use
        // through the required Function operand observations below.
        Operand::Hidden(_) => None,
    };
    let expression_source = source.get(node.span.start..node.span.end);
    if expression_source == Some("wrapped (xs => host.after ())")
        && observed.original_inner.is_none()
        && !observe_depth2_original_inner(
            args.first().and_then(&value),
            observed,
        )
    {
        return;
    }
    if expression_source != Some("fn (host.arg ())")
        || observed.attachments.inner
    {
        return;
    }
    let Some(Value::Function(function)) = value(callee) else {
        observed
            .errors
            .push("callback's actual fn operand absent".into());
        return;
    };
    observe_depth2_inner_guard(&tables, function, test, meaning, observed);
}

fn check_depth2_selected_adapter(
    joined: &fipe::mir::Program,
    tables: &fipe::ir::Tables,
    adapter: &fipe::mir::ReceiverAdapter,
    meanings: &[fipe::ir::Type],
    requested: usize,
    meaning: &str,
) -> Result<(), String> {
    use fipe::ir::Type;
    let row = &tables.dispatch[adapter.member].rows[adapter.row];
    if !matches!(&row.target, fipe::ir::Implementation::Declared(index) if *index == requested)
        || row.receiver_spot != Some(1)
        || joined.bodies[adapter.body].arity != 2
    {
        return Err("selected declared row/receiver/arity mismatch".into());
    }
    let context = joined.receiver_contexts[adapter.body]
        .as_ref()
        .ok_or("selected receiver context absent")?;
    let Type::Meaning {
        index: row_meaning,
        arguments: row_arguments,
    } = &row.receiver
    else {
        return Err("selected receiver is not M<T>".into());
    };
    let [Type::Parameter(parameter)] = row_arguments.as_slice() else {
        return Err("selected receiver binder absent".into());
    };
    let Some(Type::Meaning {
        index: actual_meaning,
        arguments: actual_arguments,
    }) = meanings.get(context.receiver_meaning)
    else {
        return Err("selected receiver captured meaning absent".into());
    };
    let [Type::Meaning {
        index: actual,
        arguments,
    }] = actual_arguments.as_slice()
    else {
        return Err("selected receiver T absent".into());
    };
    if actual_meaning != row_meaning
        || !arguments.is_empty()
        || tables.namespaces[tables.meanings[*actual].namespace] != "prelude"
        || tables.meanings[*actual].label != meaning
    {
        return Err(
            "selected receiver T differs from actual creator meaning".into()
        );
    }
    println!("DEPTH2_ROW_SELECTED member={} row={} adapter={} parameter={parameter} input={:?} meanings={meanings:?}", adapter.member, adapter.row, adapter.body, context.input_contract);
    Ok(())
}

fn declared_depth2_selected(
    machine: &Machine,
    meaning: &str,
    requested: usize,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::{value::Value, vm::Frame};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let mut selected = Vec::new();
    for (index, value) in machine.globals() {
        let definition = &tables.definitions[index];
        if tables.namespaces[definition.namespace] != "main"
            || definition.label != "relay"
        {
            continue;
        }
        let Value::Function(function) = value else {
            observed
                .errors
                .push("retained relay is not a Function".into());
            continue;
        };
        let adapters: Vec<_> = joined
            .receiver_adapters
            .iter()
            .filter(|adapter| adapter.body == function.body)
            .collect();
        if let [adapter] = adapters.as_slice() {
            selected.push((*adapter, function.meanings.as_slice()));
        } else {
            observed.errors.push(format!(
                "retained relay adapter association count {}",
                adapters.len()
            ));
        }
    }
    for frame in machine.frames() {
        let Frame::Body(frame) = frame else { continue };
        for adapter in joined
            .receiver_adapters
            .iter()
            .filter(|adapter| adapter.body == frame.body)
        {
            let row = &tables.dispatch[adapter.member].rows[adapter.row];
            if matches!(&row.target, fipe::ir::Implementation::Declared(index) if *index == requested)
            {
                selected.push((adapter, frame.meanings.as_slice()));
            }
        }
    }
    for (adapter, meanings) in selected {
        if let Err(error) = check_depth2_selected_adapter(
            joined, &tables, adapter, meanings, requested, meaning,
        ) {
            observed.errors.push(error);
        }
    }
}

fn depth2_fixture(
    strings: bool,
    healthy: bool,
) -> (
    &'static str,
    &'static str,
    &'static str,
    super::Value,
    Vec<super::Value>,
) {
    use fipe::value::Value;
    if strings {
        (
            DEPTH2_ROW_MAIN_STRING,
            DEPTH2_ROW_HOST_STRING,
            "String",
            Value::text("tag"),
            if healthy {
                vec![Value::text("a"), Value::text("b")]
            } else {
                vec![Value::text("b"), Value::text("a")]
            },
        )
    } else {
        (
            DEPTH2_ROW_MAIN_INTEGER,
            DEPTH2_ROW_HOST_INTEGER,
            "Integer",
            Value::Int(7),
            if healthy {
                vec![Value::Int(1), Value::Int(2)]
            } else {
                vec![Value::Int(2), Value::Int(1)]
            },
        )
    }
}

fn depth2_program(source: &str, host: &str) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("methods", DEPTH2_ROW_METHODS).unwrap();
    declarations.module("host", host).unwrap();
    let module = parse(lex(source)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    println!(
        "DEPTH2_ROW_TYPED op={:?} caller_checks={:?}",
        typed
            .dispatch
            .iter()
            .filter(|dispatch| dispatch.name == "op")
            .collect::<Vec<_>>(),
        typed.checks
    );
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    compile::compile(&unit)
}

fn depth2_catalog(
    machine: &Machine,
    observed: &mut DeclaredDepth2Observation,
) -> (Option<usize>, Option<usize>) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        let tests: Vec<_> = tables
            .tests
            .iter()
            .enumerate()
            .filter_map(|(index, test)| {
                (tables.namespaces[test.namespace] == "methods"
                    && DEPTH2_ROW_METHODS.get(test.span.start..test.span.end)
                        == Some("is_sorted self"))
                .then_some(index)
            })
            .collect();
        println!("DEPTH2_ROW_CATALOG tests={tests:?}");
        match tests.as_slice() {
            [] => (None, None),
            [test] => (Some(*test), joined.tests[*test]),
            _ => {
                observed
                    .errors
                    .push(format!("ambiguous source Sorted tests {tests:?}"));
                (None, None)
            }
        }
    }
}

fn observe_depth2_execution(
    machine: &Machine,
    stop: &fipe::vm::Stop,
    test: Option<usize>,
    test_body: Option<usize>,
    meaning: &str,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::{
        value::Value,
        vm::{Frame, Stop},
    };
    if matches!(stop, Stop::Before(_)) {
        if let Some(Frame::Body(frame)) = machine.frames().last() {
            let joined = machine.program().unwrap();
            let body = &joined.bodies[frame.body];
            let first = body.blocks[0]
                .statements
                .iter()
                .position(|statement| {
                    !matches!(statement, fipe::mir::Statement::Charge(_))
                })
                .unwrap_or(body.blocks[0].statements.len());
            let entered = frame.block == 0 && frame.statement == first;
            if let Some(Value::Function(original)) = &observed.original_inner {
                if frame.body == original.body && entered {
                    observed.execution.inner_entered = true;
                }
            }
            if Some(frame.body) == test_body && entered {
                let tables = joined.tables.borrow();
                println!(
                    "DEPTH2_ROW_ACTUAL_TEST body={} hidden={:?} meanings={:?}",
                    frame.body, body.hidden, frame.meanings
                );
                if let Some(test) =
                    test.and_then(|index| tables.tests.get(index))
                {
                    if test.parameters.is_empty() {
                        observed
                            .errors
                            .push("actual generic Test binder absent".into());
                    }
                    for parameter in &test.parameters {
                        let actual = body
                            .hidden
                            .iter()
                            .position(|id| id == parameter)
                            .and_then(|position| frame.meanings.get(position));
                        match actual {
                            Some(fipe::ir::Type::Meaning { index, arguments })
                                if arguments.is_empty() && tables.namespaces[tables.meanings[*index].namespace] == "prelude"
                                    && tables.meanings[*index].label == meaning => {}
                            _ => observed.errors.push(format!("actual Test binder {parameter} has wrong/missing witness {actual:?}")),
                        }
                    }
                } else {
                    observed
                        .errors
                        .push("actual Test catalog association absent".into());
                }
                observed.execution.tested = true;
            }
        }
    }
}

fn depth2_operation(
    machine: &Machine,
    declaration: usize,
    arguments: &[super::Value],
    marker: &super::Value,
    test: Option<usize>,
    meaning: &str,
    observed: &mut DeclaredDepth2Observation,
) {
    use fipe::value::Value;
    let [Value::Function(_), Value::Record(receiver)] = arguments else {
        panic!("actual Function/self spots")
    };
    assert_eq!(receiver.fields.as_slice(), core::slice::from_ref(marker));
    declared_depth2_selected(machine, meaning, declaration, observed);
    declared_depth2_outer(
        machine,
        declaration,
        arguments,
        test,
        meaning,
        observed,
    );
}

fn assert_depth2_result(
    machine: &Machine,
    healthy: bool,
    result: Result<super::Value, fipe::vm::Fault>,
    calls: &[String],
    test: Option<usize>,
    observed: &DeclaredDepth2Observation,
) {
    use fipe::value::Value;
    assert!(
        observed.errors.is_empty(),
        "observer nonvacuity: {:?}",
        observed.errors
    );
    assert!(
        observed.attachments.outer
            && observed.attachments.inner
            && observed.attachments.same_creator
            && observed.execution.tested,
        "actual two-level row guard/witness/Test association"
    );
    if healthy {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert!(observed.execution.inner_entered);
        assert_eq!(
            calls,
            ["methods.op", "host.recall", "host.arg", "host.after"]
        );
    } else {
        let fault =
            result.expect_err("reversed input must fail before inner lambda");
        let Reason::Failed { entry, .. } = fault.reason else {
            panic!("actual semantic Sorted failure")
        };
        assert_eq!(Some(entry), observed.entry);
        assert!(bound_order_test_ids(
            &machine.program().unwrap().tables.borrow().entries[entry]
                .predicate
        )
        .contains(&test.unwrap()));
        assert!(!observed.execution.inner_entered);
        assert_eq!(calls, ["methods.op", "host.recall", "host.arg"]);
    }
}

fn declared_depth2_case(strings: bool, healthy: bool) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::Stop,
    };
    let (source, host, meaning, marker, items) =
        depth2_fixture(strings, healthy);
    let program = depth2_program(source, host);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let mut observed = DeclaredDepth2Observation::default();
    let (test, test_body) = depth2_catalog(&machine, &mut observed);
    println!("DEPTH2_ROW_TEST test={test:?} body={test_body:?}");
    let mut stored = None;
    let mut calls = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(&stop, Stop::Before(_)) {
            declared_depth2_before_call(
                &machine,
                source,
                test,
                meaning,
                &mut observed,
            );
        }
        observe_depth2_execution(
            &machine,
            &stop,
            test,
            test_body,
            meaning,
            &mut observed,
        );
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                let (namespace, label) = {
                    let tables = machine.program().unwrap().tables.borrow();
                    let d = &tables.declarations[declaration];
                    (tables.namespaces[d.namespace].clone(), d.label.clone())
                };
                println!("DEPTH2_ROW_CALLBACK namespace={namespace} label={label} arguments={arguments:?} env={environment:?}");
                assert!(environment.is_empty());
                let reply = if namespace == "methods" {
                    assert_eq!(label, "op");
                    depth2_operation(
                        &machine,
                        declaration,
                        &arguments,
                        &marker,
                        test,
                        meaning,
                        &mut observed,
                    );
                    stored = Some(arguments[0].clone());
                    Value::Int(7)
                } else {
                    assert_eq!(namespace, "host");
                    assert_eq!(arguments, [Value::Unit]);
                    match label.as_str() {
                        "recall" => stored
                            .as_ref()
                            .expect("actual received callback")
                            .clone(),
                        "arg" => Value::seq(items.clone()),
                        "after" => Value::Int(7),
                        _ => panic!("unexpected typed callback"),
                    }
                };
                calls.push(format!("{namespace}.{label}"));
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                println!("DEPTH2_ROW_TERMINAL meaning={meaning} healthy={healthy} result={result:?} calls={calls:?} observed={observed:?} charged={}", machine.charged());
                assert_depth2_result(
                    &machine, healthy, result, &calls, test, &observed,
                );
                return;
            }
            Stop::Request(other) => panic!("typed callbacks only: {other:?}"),
        }
    }
    panic!("bounded declared depth2 observer must finish");
}

#[test]
fn declared_row_depth2_integer_healthy_diagnostic() {
    declared_depth2_case(false, true);
}
#[test]
fn declared_row_depth2_integer_reversed_diagnostic() {
    declared_depth2_case(false, false);
}
#[test]
fn declared_row_depth2_string_healthy_diagnostic() {
    declared_depth2_case(true, true);
}
#[test]
fn declared_row_depth2_string_reversed_diagnostic() {
    declared_depth2_case(true, false);
}

const GUARD_GROWTH_METHODS: &str = "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\npub type Sorted<T: Ord>: Vec<T> where is_sorted self\npub type M<T: Ord> { pub token: T }\nimpl M { op: (Sorted<T> -> Integer) -> (self: Self) -> Integer }\n";
const GUARD_GROWTH_HOST: &str = "pub recall: () -> a\n";
const GUARD_GROWTH_MAIN: &str = "use host\nuse methods\nget = m => m.op\nrelay = get (methods.M 7)\nmake: Integer -> Vec<Integer> -> Integer = seed => {\n c0 = seed + 0\n c1 = seed + 1\n c2 = seed + 2\n c3 = seed + 3\n c4 = seed + 4\n c5 = seed + 5\n c6 = seed + 6\n c7 = seed + 7\n c8 = seed + 8\n c9 = seed + 9\n c10 = seed + 10\n c11 = seed + 11\n c12 = seed + 12\n c13 = seed + 13\n c14 = seed + 14\n c15 = seed + 15\n c16 = seed + 16\n c17 = seed + 17\n c18 = seed + 18\n c19 = seed + 19\n c20 = seed + 20\n c21 = seed + 21\n c22 = seed + 22\n c23 = seed + 23\n c24 = seed + 24\n c25 = seed + 25\n c26 = seed + 26\n c27 = seed + 27\n c28 = seed + 28\n c29 = seed + 29\n c30 = seed + 30\n c31 = seed + 31\n c32 = seed + 32\n c33 = seed + 33\n c34 = seed + 34\n c35 = seed + 35\n c36 = seed + 36\n c37 = seed + 37\n c38 = seed + 38\n c39 = seed + 39\n c40 = seed + 40\n c41 = seed + 41\n c42 = seed + 42\n c43 = seed + 43\n c44 = seed + 44\n c45 = seed + 45\n c46 = seed + 46\n c47 = seed + 47\n c48 = seed + 48\n c49 = seed + 49\n c50 = seed + 50\n c51 = seed + 51\n c52 = seed + 52\n c53 = seed + 53\n c54 = seed + 54\n c55 = seed + 55\n c56 = seed + 56\n c57 = seed + 57\n c58 = seed + 58\n c59 = seed + 59\n c60 = seed + 60\n c61 = seed + 61\n c62 = seed + 62\n c63 = seed + 63\n xs => c0 + c1 + c2 + c3 + c4 + c5 + c6 + c7 + c8 + c9 + c10 + c11 + c12 + c13 + c14 + c15 + c16 + c17 + c18 + c19 + c20 + c21 + c22 + c23 + c24 + c25 + c26 + c27 + c28 + c29 + c30 + c31 + c32 + c33 + c34 + c35 + c36 + c37 + c38 + c39 + c40 + c41 + c42 + c43 + c44 + c45 + c46 + c47 + c48 + c49 + c50 + c51 + c52 + c53 + c54 + c55 + c56 + c57 + c58 + c59 + c60 + c61 + c62 + c63\n}\nplain: Vec<Integer> -> Integer = make 7\nround1 = relay plain\nfn1: Vec<Integer> -> Integer = host.recall ()\nround2 = relay fn1\nfn2: Vec<Integer> -> Integer = host.recall ()\nround3 = relay fn2\nfn3: Vec<Integer> -> Integer = host.recall ()\nround4 = relay fn3\nfn4: Vec<Integer> -> Integer = host.recall ()\nround5 = relay fn4\nfn5: Vec<Integer> -> Integer = host.recall ()\nround6 = relay fn5\nfn6: Vec<Integer> -> Integer = host.recall ()\nround7 = relay fn6\nfn7: Vec<Integer> -> Integer = host.recall ()\nround8 = relay fn7\nfn8: Vec<Integer> -> Integer = host.recall ()\nround9 = relay fn8\nfn9: Vec<Integer> -> Integer = host.recall ()\nround10 = relay fn9\nfn10: Vec<Integer> -> Integer = host.recall ()\nround11 = relay fn10\nfn11: Vec<Integer> -> Integer = host.recall ()\nround12 = relay fn11\nfn12: Vec<Integer> -> Integer = host.recall ()\nround13 = relay fn12\nfn13: Vec<Integer> -> Integer = host.recall ()\nround14 = relay fn13\nfn14: Vec<Integer> -> Integer = host.recall ()\nround15 = relay fn14\nfn15: Vec<Integer> -> Integer = host.recall ()\nround16 = relay fn15\nfn16: Vec<Integer> -> Integer = host.recall ()\nround17 = relay fn16\nfn17: Vec<Integer> -> Integer = host.recall ()\nround18 = relay fn17\nfn18: Vec<Integer> -> Integer = host.recall ()\nround19 = relay fn18\nfn19: Vec<Integer> -> Integer = host.recall ()\nround20 = relay fn19\nfn20: Vec<Integer> -> Integer = host.recall ()\nround21 = relay fn20\nfn21: Vec<Integer> -> Integer = host.recall ()\nround22 = relay fn21\nfn22: Vec<Integer> -> Integer = host.recall ()\nround23 = relay fn22\nfn23: Vec<Integer> -> Integer = host.recall ()\nround24 = relay fn23\nfn24: Vec<Integer> -> Integer = host.recall ()\nround25 = relay fn24\nfn25: Vec<Integer> -> Integer = host.recall ()\nround26 = relay fn25\nfn26: Vec<Integer> -> Integer = host.recall ()\nround27 = relay fn26\nfn27: Vec<Integer> -> Integer = host.recall ()\nround28 = relay fn27\nfn28: Vec<Integer> -> Integer = host.recall ()\nround29 = relay fn28\nfn29: Vec<Integer> -> Integer = host.recall ()\nround30 = relay fn29\nfn30: Vec<Integer> -> Integer = host.recall ()\nround31 = relay fn30\nfn31: Vec<Integer> -> Integer = host.recall ()\nround32 = relay fn31\nfn32: Vec<Integer> -> Integer = host.recall ()\nround33 = relay fn32\nfn33: Vec<Integer> -> Integer = host.recall ()\nround34 = relay fn33\nfn34: Vec<Integer> -> Integer = host.recall ()\nround35 = relay fn34\nfn35: Vec<Integer> -> Integer = host.recall ()\nround36 = relay fn35\nfn36: Vec<Integer> -> Integer = host.recall ()\nround37 = relay fn36\nfn37: Vec<Integer> -> Integer = host.recall ()\nround38 = relay fn37\nfn38: Vec<Integer> -> Integer = host.recall ()\nround39 = relay fn38\nfn39: Vec<Integer> -> Integer = host.recall ()\nround40 = relay fn39\nfn40: Vec<Integer> -> Integer = host.recall ()\nround41 = relay fn40\nfn41: Vec<Integer> -> Integer = host.recall ()\nround42 = relay fn41\nfn42: Vec<Integer> -> Integer = host.recall ()\nround43 = relay fn42\nfn43: Vec<Integer> -> Integer = host.recall ()\nround44 = relay fn43\nfn44: Vec<Integer> -> Integer = host.recall ()\nround45 = relay fn44\nfn45: Vec<Integer> -> Integer = host.recall ()\nround46 = relay fn45\nfn46: Vec<Integer> -> Integer = host.recall ()\nround47 = relay fn46\nfn47: Vec<Integer> -> Integer = host.recall ()\nround48 = relay fn47\nfn48: Vec<Integer> -> Integer = host.recall ()\nround49 = relay fn48\nfn49: Vec<Integer> -> Integer = host.recall ()\nround50 = relay fn49\nfn50: Vec<Integer> -> Integer = host.recall ()\nround51 = relay fn50\nfn51: Vec<Integer> -> Integer = host.recall ()\nround52 = relay fn51\nfn52: Vec<Integer> -> Integer = host.recall ()\nround53 = relay fn52\nfn53: Vec<Integer> -> Integer = host.recall ()\nround54 = relay fn53\nfn54: Vec<Integer> -> Integer = host.recall ()\nround55 = relay fn54\nfn55: Vec<Integer> -> Integer = host.recall ()\nround56 = relay fn55\nfn56: Vec<Integer> -> Integer = host.recall ()\nround57 = relay fn56\nfn57: Vec<Integer> -> Integer = host.recall ()\nround58 = relay fn57\nfn58: Vec<Integer> -> Integer = host.recall ()\nround59 = relay fn58\nfn59: Vec<Integer> -> Integer = host.recall ()\nround60 = relay fn59\nfn60: Vec<Integer> -> Integer = host.recall ()\nround61 = relay fn60\nfn61: Vec<Integer> -> Integer = host.recall ()\nround62 = relay fn61\nfn62: Vec<Integer> -> Integer = host.recall ()\nround63 = relay fn62\nfn63: Vec<Integer> -> Integer = host.recall ()\nround64 = relay fn63\n0\n";

struct GuardGrowthPending {
    round: usize,
    body: usize,
    block: usize,
    statement: usize,
    dest: usize,
    node: usize,
    before: u64,
    argument: std::rc::Rc<fipe::value::Closure>,
}

fn guard_growth_spans(module: &fipe::ast::Module) -> Vec<fipe::token::Span> {
    use fipe::ast::{Entry, ExprKind, PatternKind};
    let mut found = std::collections::BTreeMap::new();
    for entry in &module.entries {
        let Entry::Line(id) = entry else {
            continue;
        };
        let ExprKind::Bind { pattern, value, .. } =
            module.expr(*id).unwrap().kind
        else {
            continue;
        };
        let PatternKind::Name(name) = &module.pattern(pattern).unwrap().kind
        else {
            continue;
        };
        let Some(round) = name
            .segments
            .last()
            .and_then(|name| name.strip_prefix("round"))
            .and_then(|n| n.parse::<usize>().ok())
        else {
            continue;
        };
        let call = module.expr(value).unwrap();
        assert!(matches!(call.kind, ExprKind::Call { .. }));
        assert!(found.insert(round, call.span).is_none());
    }
    assert_eq!(
        found.keys().copied().collect::<Vec<_>>(),
        (1..=64).collect::<Vec<_>>()
    );
    found.into_values().collect()
}

fn guard_growth_test(program: &fipe::mir::Program) -> usize {
    let tables = program.tables.borrow();
    let found: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "methods"
                && GUARD_GROWTH_METHODS.get(test.span.start..test.span.end)
                    == Some("is_sorted self"))
            .then_some(index)
        })
        .collect();
    assert_eq!(found.len(), 1, "actual source-owned Sorted Test");
    assert!(program.tests[found[0]].is_some());
    found[0]
}

fn guard_growth_guard(
    guard: &fipe::value::Guard,
    test: usize,
    program: &fipe::mir::Program,
) -> (usize, usize) {
    use fipe::ir::{Predicate, Type};
    assert_eq!(guard.spot, 0);
    let tables = program.tables.borrow();
    assert!(guard.entry < tables.entries.len());
    let mut predicates = vec![&guard.predicate];
    let mut types = Vec::new();
    let (mut predicate_nodes, mut type_nodes, mut matched) = (0, 0, 0);
    while let Some(predicate) = predicates.pop() {
        predicate_nodes += 1;
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                predicates.extend(parts);
            }
            Predicate::Not(inner) => predicates.push(inner),
            Predicate::Function { parameter, result } => {
                predicates.extend([parameter.as_ref(), result.as_ref()]);
            }
            Predicate::Test { index, meanings } => {
                types.extend(meanings);
                if *index == test {
                    matched += 1;
                    let [Type::Parameter(id)] = meanings.as_slice() else {
                        panic!("actual row-owned generic Sorted witness")
                    };
                    let witnesses: Vec<_> = guard
                        .witnesses
                        .iter()
                        .filter(|(bound, _)| bound == id)
                        .collect();
                    assert_eq!(witnesses.len(), 1);
                    let Type::Meaning { index, arguments } =
                        witnesses[0].1.as_type()
                    else {
                        panic!("actual Integer witness")
                    };
                    assert!(arguments.is_empty());
                    assert_eq!(
                        tables.namespaces[tables.meanings[*index].namespace],
                        "prelude"
                    );
                    assert_eq!(tables.meanings[*index].label, "Integer");
                }
            }
            Predicate::Nonzero { .. } => (),
        }
    }
    assert_eq!(matched, 1);
    while let Some(ty) = types.pop() {
        type_nodes += 1;
        match ty {
            Type::Meaning { arguments, .. } => types.extend(arguments),
            Type::Tuple(parts)
            | Type::Packed(parts)
            | Type::All(parts)
            | Type::Sum(parts) => types.extend(parts),
            Type::Array { element, length } => {
                types.extend([element.as_ref(), length.as_ref()]);
            }
            Type::Function { parameter, result } => {
                types.extend([parameter.as_ref(), result.as_ref()]);
            }
            Type::Not(inner) => types.push(inner),
            _ => (),
        }
    }
    (predicate_nodes, type_nodes)
}

fn guard_growth_before(
    machine: &Machine,
    span: fipe::token::Span,
    round: usize,
) -> Option<GuardGrowthPending> {
    use fipe::{
        mir::{Operand, Rvalue, Statement},
        value::Value,
        vm::Frame,
    };
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    let program = machine.program().unwrap();
    let block = &program.bodies[frame.body].blocks[frame.block];
    let Some(Statement::Assign(dest, Rvalue::Admit { values, .. })) =
        block.statements.get(frame.statement)
    else {
        return None;
    };
    let node = block.nodes[frame.statement];
    let tables = program.tables.borrow();
    if tables.namespaces[tables.nodes[node].namespace] != "main"
        || tables.nodes[node].span != span
    {
        return None;
    }
    let Operand::Local(argument) = values[1] else {
        panic!("actual source supply in its local")
    };
    let Value::Function(function) = &frame.locals[argument] else {
        panic!("real checked function argument")
    };
    assert!(function.filled.is_empty());
    assert!(
        function.meanings.is_empty(),
        "no ordinary hidden type-copy fee may mask guard-vector cost"
    );
    assert_eq!(
        function.captures.len(),
        64,
        "actual factory captures, not guessed source depth"
    );
    assert!(function
        .captures
        .iter()
        .all(|value| matches!(value, Value::Int(_))));
    let lambda = &tables.nodes[program.bodies[function.body].node];
    assert_eq!(tables.namespaces[lambda.namespace], "main");
    assert!(GUARD_GROWTH_MAIN
        .get(lambda.span.start..lambda.span.end)
        .unwrap()
        .starts_with("xs => c0 + c1"));
    assert_eq!(program.bodies[function.body].arity, 1);
    Some(GuardGrowthPending {
        round,
        body: frame.body,
        block: frame.block,
        statement: frame.statement,
        dest: *dest,
        node,
        before: machine.charged(),
        argument: function.clone(),
    })
}

fn guard_growth_after(
    machine: &Machine,
    pending: &GuardGrowthPending,
    value: &fipe::value::Value,
    aliases: &[std::rc::Rc<fipe::value::Closure>],
    test: usize,
) -> std::rc::Rc<fipe::value::Closure> {
    use fipe::{value::Value, vm::Frame};
    use std::rc::Rc;
    let Value::Tuple(parts) = value else {
        panic!("actual admission result tuple")
    };
    let [Value::Function(_), Value::Function(function)] = parts.as_slice()
    else {
        panic!("private callee and guarded argument")
    };
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        panic!("assignment completion in caller")
    };
    assert_eq!((frame.body, frame.block), (pending.body, pending.block));
    assert_eq!(frame.statement, pending.statement + 1);
    let Value::Tuple(stored) = &frame.locals[pending.dest] else {
        panic!("same destination tuple")
    };
    assert!(Rc::ptr_eq(parts, stored));
    assert!(!Rc::ptr_eq(function, &pending.argument));
    assert_eq!(function.body, pending.argument.body);
    assert_eq!(function.captures, pending.argument.captures);
    assert_eq!(function.filled, pending.argument.filled);
    assert_eq!(function.guards.len(), pending.round);
    for (old, copied) in pending.argument.guards.iter().zip(&function.guards) {
        assert!(!std::ptr::eq(old, copied), "distinct actual guard objects");
        assert_eq!(
            (old.spot, old.entry, &old.predicate),
            (copied.spot, copied.entry, &copied.predicate)
        );
        assert!(Rc::ptr_eq(&old.witnesses, &copied.witnesses));
    }
    let program = machine.program().unwrap();
    let mut nodes = (0, 0);
    for guard in &function.guards {
        let shape = guard_growth_guard(guard, test, program);
        nodes.0 += shape.0;
        nodes.1 += shape.1;
    }
    for (count, alias) in aliases.iter().enumerate() {
        assert_eq!(alias.guards.len(), count, "old aliases never gain guards");
        for guard in &alias.guards {
            guard_growth_guard(guard, test, program);
        }
    }
    println!("GUARD_GROWTH round={} before={} after={} delta={} captures={} guards={} predicate_nodes={} type_nodes={}",
        pending.round, pending.before, machine.charged(), machine.charged().checked_sub(pending.before).unwrap(),
        function.captures.len(), function.guards.len(), nodes.0, nodes.1);
    function.clone()
}

fn guard_growth_operation(
    tables: &fipe::ir::Tables,
    declaration: &fipe::ir::Declaration,
    arguments: &[super::Value],
    published: &mut Option<std::rc::Rc<fipe::value::Closure>>,
    operations: &mut usize,
) -> super::Value {
    use fipe::value::Value;
    use std::rc::Rc;
    assert_eq!(declaration.label, "op");
    let [Value::Function(function), Value::Record(receiver)] = arguments else {
        panic!("actual self-last op arguments")
    };
    assert_eq!(receiver.fields, [Value::Int(7)]);
    assert_eq!(
        tables.namespaces[tables.meanings[receiver.meaning].namespace],
        "methods"
    );
    assert_eq!(tables.meanings[receiver.meaning].label, "M");
    assert!(
        Rc::ptr_eq(
            function,
            &published.take().expect("published admission precedes op")
        ),
        "Host freezing must preserve actual published Rc or expose first loss"
    );
    *operations += 1;
    assert_eq!(function.guards.len(), *operations);
    Value::Int(7)
}

#[test]
fn public_retrieval_guard_growth_preserves_old_aliases_and_observes_copy_work()
{
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::Stop,
    };
    use std::rc::Rc;
    let mut declarations = Declarations::new();
    declarations.module("host", GUARD_GROWTH_HOST).unwrap();
    declarations
        .module("methods", GUARD_GROWTH_METHODS)
        .unwrap();
    let module = parse(lex(GUARD_GROWTH_MAIN)).unwrap();
    let spans = guard_growth_spans(&module);
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("public bounded round-trip producer");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(None);
    machine.begin(&program);
    let test = guard_growth_test(machine.program().unwrap());
    let mut aliases = Vec::new();
    let mut pending: Option<GuardGrowthPending> = None;
    let mut published = None;
    let (mut operations, mut recalls) = (0, 0);
    for _ in 0..100_000 {
        match machine.next_stop() {
            Stop::Before(_) => {
                if operations < 64 && pending.is_none() {
                    if let Some(observed) = guard_growth_before(
                        &machine,
                        spans[operations],
                        operations + 1,
                    ) {
                        assert_eq!(observed.argument.guards.len(), operations);
                        if operations == 0 {
                            aliases.push(observed.argument.clone());
                        } else {
                            assert!(Rc::ptr_eq(
                                &observed.argument,
                                &aliases[operations]
                            ));
                        }
                        pending = Some(observed);
                    }
                }
            }
            Stop::After(node, value) => {
                if pending.as_ref().is_some_and(|pending| pending.node == node)
                    && matches!(value, Value::Tuple(_))
                {
                    let completed = pending.take().unwrap();
                    let guarded = guard_growth_after(
                        &machine, &completed, &value, &aliases, test,
                    );
                    aliases.push(guarded.clone());
                    assert!(published.replace(guarded).is_none());
                }
            }
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let tables = machine.program().unwrap().tables.borrow();
                let declaration = &tables.declarations[declaration];
                let namespace = &tables.namespaces[declaration.namespace];
                let reply = if namespace == "methods" {
                    guard_growth_operation(
                        &tables,
                        declaration,
                        &arguments,
                        &mut published,
                        &mut operations,
                    )
                } else {
                    assert_eq!(namespace, "host");
                    assert_eq!(declaration.label, "recall");
                    assert_eq!(arguments, [Value::Unit]);
                    recalls += 1;
                    assert_eq!(recalls, operations);
                    Value::Function(aliases[operations].clone())
                };
                drop(tables);
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Request(other) => panic!("typed callback only: {other:?}"),
            Stop::Done(result) => {
                assert_eq!(result.unwrap(), Value::Int(0));
                assert_eq!((operations, recalls, aliases.len()), (64, 63, 65));
                assert!(pending.is_none() && published.is_none());
                println!(
                    "GUARD_GROWTH_END rounds=64 aliases=65 charged={}",
                    machine.charged()
                );
                return;
            }
        }
    }
    panic!("bounded64-round producer must finish");
}

const SAME_SPOT_MAIN: &str = "use host\nuse methods\nget_first = m => m.first\nget_second = m => m.second\nfirst = get_first (methods.M 7)\nsecond = get_second (methods.M 7)\noriginal = fn => fn (host.arg ())\nfirst_result = first original\nmiddle: (Integer -> Integer) -> Integer = host.recall ()\nsecond_result = second middle\nwrapped: (Integer -> Integer) -> Integer = host.recall ()\nwrapped (x => host.after ())\n";
const SAME_SPOT_METHODS: &str = "pub type Positive: Integer where self > 0\npub type Small: Integer where self < 10\npub type M { pub token: Integer }\nimpl M {\n first: ((Positive -> Integer) -> Integer) -> (self: Self) -> Integer,\n second: ((Small -> Integer) -> Integer) -> (self: Self) -> Integer\n}\n";
const SAME_SPOT_HOST: &str =
    "pub arg: () -> Integer\npub after: () -> Integer\npub recall: () -> a\n";

#[derive(Default)]
struct SameSpotObservation {
    callback_entered: bool,
    test_order: Vec<usize>,
    calls: Vec<String>,
    stored: Option<super::Value>,
    inner: Option<std::rc::Rc<fipe::value::Closure>>,
    callback: Option<std::rc::Rc<fipe::value::Closure>>,
    original: Option<std::rc::Rc<fipe::value::Closure>>,
    outer: Vec<std::rc::Rc<fipe::value::Closure>>,
    entries: [Option<usize>; 2],
    errors: Vec<String>,
}

fn same_spot_tests(
    machine: &Machine,
    observed: &mut SameSpotObservation,
) -> ([Option<usize>; 2], [Option<usize>; 2]) {
    let mut tests = [None; 2];
    let mut test_bodies = [None; 2];
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        for (which, text) in ["self > 0", "self < 10"].iter().enumerate() {
            let found: Vec<_> = tables
                .tests
                .iter()
                .enumerate()
                .filter(|(_, test)| {
                    tables.namespaces[test.namespace] == "methods"
                        && SAME_SPOT_METHODS.get(test.span.start..test.span.end)
                            == Some(*text)
                })
                .map(|(index, _)| index)
                .collect();
            if let [index] = found.as_slice() {
                tests[which] = Some(*index);
                test_bodies[which] = joined.tests[*index];
            } else {
                observed
                    .errors
                    .push(format!("source Test {text}: {found:?}"));
            }
            if test_bodies[which].is_none() {
                observed
                    .errors
                    .push(format!("source Test {text} body absent"));
            }
        }
    }
    (tests, test_bodies)
}

fn observe_same_spot_entry(
    machine: &Machine,
    test_bodies: &[Option<usize>; 2],
    observed: &mut SameSpotObservation,
) {
    use fipe::{value::Value, vm::Frame};
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let body = &joined.bodies[frame.body];
        let first = body.blocks[0]
            .statements
            .iter()
            .position(|s| !matches!(s, fipe::mir::Statement::Charge(_)))
            .unwrap_or(body.blocks[0].statements.len());
        if frame.block == 0 && frame.statement == first {
            for (which, test_body) in test_bodies.iter().enumerate() {
                if *test_body == Some(frame.body) {
                    observed.test_order.push(which);
                }
            }
            if observed.callback.as_ref().is_some_and(
                |function: &std::rc::Rc<fipe::value::Closure>| {
                    function.body == frame.body
                },
            ) {
                observed.callback_entered = true;
            }
            if observed.original.as_ref().is_some_and(
                |function: &std::rc::Rc<fipe::value::Closure>| {
                    function.body == frame.body
                },
            ) && observed.inner.is_none()
            {
                let tables = joined.tables.borrow();
                let parameter = body
                    .locals
                    .iter()
                    .enumerate()
                    .skip(1)
                    .take(body.arity)
                    .filter_map(|(index, local)| {
                        let (namespace, span) =
                            tables.slots.get(local.slot?)?.name?;
                        (tables.namespaces[namespace] == "main"
                            && SAME_SPOT_MAIN.get(span.start..span.end)
                                == Some("fn"))
                        .then_some(index)
                    })
                    .collect::<Vec<_>>();
                match parameter.as_slice() {
                    [slot] if body.arity == 1 => match frame.locals.get(*slot) {
                        Some(Value::Function(function)) => observed.inner = Some(function.clone()),
                        other => observed.errors.push(format!("transformed callback argument absent: {other:?}")),
                    },
                    _ => observed.errors.push(format!("original parameter metadata: arity={} slots={parameter:?}", body.arity)),
                }
            }
        }
    }
}

fn observe_same_spot_admit(
    machine: &Machine,
    observed: &mut SameSpotObservation,
) {
    use fipe::{
        mir::{Rvalue, Statement},
        value::Value,
        vm::Frame,
    };
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let body = &joined.bodies[frame.body];
        let block = &body.blocks[frame.block];
        if let Some(Statement::Assign(_, Rvalue::Admit { values, .. })) =
            block.statements.get(frame.statement)
        {
            if let Some(node) = block.nodes.get(frame.statement) {
                let tables = joined.tables.borrow();
                let node = &tables.nodes[*node];
                let text = (tables.namespaces[node.namespace] == "main")
                    .then(|| SAME_SPOT_MAIN.get(node.span.start..node.span.end))
                    .flatten();
                let target = if text == Some("first original") {
                    Some(&mut observed.original)
                } else if text == Some("wrapped (x => host.after ())") {
                    Some(&mut observed.callback)
                } else {
                    None
                };
                if let Some(target) = target {
                    if target.is_none() {
                        match owned_guard_operand(machine, frame, values[1]) {
                            Some(Value::Function(function)) => *target = Some(function),
                            other => observed.errors.push(format!("pre-admission operand {text:?} absent: {other:?}")),
                        }
                    }
                }
            }
        }
    }
}

fn same_spot_received(
    machine: &Machine,
    declaration: usize,
    label: &str,
    arguments: &[super::Value],
    observed: &mut SameSpotObservation,
) -> super::Value {
    use fipe::{ir::Implementation, value::Value};
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let which = match label {
        "first" => 0,
        "second" => 1,
        _ => panic!("unexpected method {label}"),
    };
    let [Value::Function(function), Value::Record(receiver)] = arguments else {
        panic!("Function/self protocol")
    };
    assert_eq!(receiver.fields, [Value::Int(7)]);
    let candidates: Vec<_> = tables.dispatch.iter().flat_map(|dispatch| &dispatch.rows).filter(|row| matches!(row.target, Implementation::Declared(index) if index == declaration) && matches!(&row.receiver, fipe::ir::Type::Meaning { index, .. } if *index == receiver.meaning)).collect();
    if let [row] = candidates.as_slice() {
        observed.entries[which] = row.input_contract;
    } else {
        observed
            .errors
            .push(format!("ambiguous selected {label} rows: {candidates:?}"));
    }
    if observed.entries[which].is_none() {
        observed
            .errors
            .push(format!("{label} input contract absent"));
    }
    println!(
        "SAME_SPOT_RECEIVED {label} entry={:?} guards={:?}",
        observed.entries[which], function.guards
    );
    observed.outer.push(function.clone());
    observed.stored = Some(Value::Function(function.clone()));
    Value::Int(7)
}

fn assert_same_spot_sources(
    observed: &SameSpotObservation,
    tests: &[Option<usize>; 2],
) {
    assert!(observed.entries.iter().all(Option::is_some));
    assert_ne!(observed.entries[0], observed.entries[1]);
    assert!(tests.iter().all(Option::is_some));
    assert_ne!(tests[0], tests[1]);
    assert_eq!(observed.outer.len(), 2);
}

fn assert_same_spot_aliases(
    machine: &Machine,
    observed: &SameSpotObservation,
    tests: &[Option<usize>; 2],
    original: &std::rc::Rc<fipe::value::Closure>,
    callback: &std::rc::Rc<fipe::value::Closure>,
    inner: &std::rc::Rc<fipe::value::Closure>,
) {
    use fipe::ir::Predicate;
    assert!(original.guards.is_empty());
    assert!(callback.guards.is_empty());
    for (function, source, count, leading_function) in [
        (&observed.outer[0], original, 1, true),
        (&observed.outer[1], original, 2, true),
        (inner, callback, 2, false),
    ] {
        assert!(!std::rc::Rc::ptr_eq(function, source));
        assert_eq!(function.body, source.body);
        assert_eq!(function.captures, source.captures);
        assert_eq!(function.filled, source.filled);
        assert_eq!(function.guards.len(), count);
        for (which, guard) in function.guards.iter().enumerate() {
            assert_eq!(guard.spot, 0);
            assert_eq!(Some(guard.entry), observed.entries[which]);
            assert_eq!(
                matches!(&guard.predicate, Predicate::Function { .. }),
                leading_function
            );
            assert_eq!(
                bound_order_test_ids(&guard.predicate),
                [tests[which].unwrap()]
            );
            let tables = machine.program().unwrap().tables.borrow();
            assert!(bound_order_test_ids(
                &tables.entries[guard.entry].predicate
            )
            .contains(&tests[which].unwrap()));
        }
    }
    assert!(std::rc::Rc::ptr_eq(
        &observed.outer[0].guards[0].witnesses,
        &observed.outer[1].guards[0].witnesses
    ));
    for which in 0..2 {
        assert!(std::rc::Rc::ptr_eq(
            &inner.guards[which].witnesses,
            &observed.outer[1].guards[which].witnesses
        ));
    }
}

fn assert_same_spot_result(
    argument: i64,
    result: Result<super::Value, fipe::vm::Fault>,
    observed: &SameSpotObservation,
) {
    use fipe::value::Value;
    let mut expected_calls = vec![
        "methods.first",
        "host.recall",
        "methods.second",
        "host.recall",
        "host.arg",
    ];
    if argument == 5 {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert!(observed.callback_entered);
        expected_calls.push("host.after");
    } else {
        let fault = result.unwrap_err();
        let Reason::Failed { entry, .. } = fault.reason else {
            panic!("source Test refusal: {fault:?}")
        };
        assert_eq!(Some(entry), observed.entries[usize::from(argument != 0)]);
        assert!(!observed.callback_entered);
    }
    assert_eq!(observed.calls, expected_calls);
    assert_eq!(
        observed.test_order,
        if argument == 0 { vec![0] } else { vec![0, 1] }
    );
    assert!(
        observed.errors.is_empty(),
        "metadata observations: {:?}",
        observed.errors
    );
}

fn same_spot_guard_case(argument: i64) {
    use fipe::{
        embed::{Reply, Request},
        value::Value,
        vm::Stop,
    };
    let mut declarations = Declarations::new();
    declarations.module("methods", SAME_SPOT_METHODS).unwrap();
    declarations.module("host", SAME_SPOT_HOST).unwrap();
    let module = parse(lex(SAME_SPOT_MAIN)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(1 << 20));
    machine.begin(&program);
    let mut observed = SameSpotObservation::default();
    let (tests, test_bodies) = same_spot_tests(&machine, &mut observed);
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(&stop, Stop::Before(_)) {
            observe_same_spot_entry(&machine, &test_bodies, &mut observed);
            observe_same_spot_admit(&machine, &mut observed);
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let joined = machine.program().unwrap();
                let tables = joined.tables.borrow();
                let d = &tables.declarations[declaration];
                let namespace = tables.namespaces[d.namespace].as_str();
                let label = d.label.as_str();
                let reply = if namespace == "methods" {
                    same_spot_received(
                        &machine,
                        declaration,
                        label,
                        &arguments,
                        &mut observed,
                    )
                } else {
                    assert_eq!(namespace, "host");
                    assert_eq!(arguments, [Value::Unit]);
                    match label {
                        "recall" => observed
                            .stored
                            .as_ref()
                            .expect("actual received Function")
                            .clone(),
                        "arg" => Value::Int(argument),
                        "after" => Value::Int(7),
                        _ => panic!("unexpected host {label}"),
                    }
                };
                observed.calls.push(format!("{namespace}.{label}"));
                drop(tables);
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Request(other) => panic!("unexpected request {other:?}"),
            Stop::Done(result) => {
                let (entries, test_order, calls, errors, inner) = (
                    &observed.entries,
                    &observed.test_order,
                    &observed.calls,
                    &observed.errors,
                    &observed.inner,
                );
                println!("SAME_SPOT_TERMINAL arg={argument} entries={entries:?} tests={tests:?} order={test_order:?} calls={calls:?} result={result:?} errors={errors:?} inner={inner:?}");
                assert_same_spot_sources(&observed, &tests);
                let original =
                    observed.original.take().expect("observed original alias");
                let callback =
                    observed.callback.take().expect("observed callback alias");
                let inner = observed
                    .inner
                    .take()
                    .expect("observed transformed callback");
                assert_same_spot_aliases(
                    &machine, &observed, &tests, &original, &callback, &inner,
                );
                assert_same_spot_result(argument, result, &observed);
                return;
            }
        }
    }
    panic!("bounded same-slot producer must finish");
}

#[test]
fn same_spot_declared_contracts_keep_ordered_transformed_argument() {
    same_spot_guard_case(5);
}
#[test]
fn same_spot_declared_contracts_first_failure_stops_second() {
    same_spot_guard_case(0);
}
#[test]
fn same_spot_declared_contracts_second_failure_keeps_first() {
    same_spot_guard_case(20);
}

const FOLD_COPY_METHODS: &str = "pub type Positive: Integer where self > 0\npub type Carrier { pub token: Integer }\nimpl Carrier { hold: (Positive -> (Positive -> Integer) -> Integer) -> (self: Self) -> Integer }\npub type M<T> { pub token: T }\nimpl M { op: Integer -> (self: Self) -> Integer }\n";
const FOLD_COPY_HOST: &str = "pub arg: () -> Integer\npub recall: () -> a\npub saved: (Integer -> Integer) -> Integer\n";
const FOLD_COPY_SHALLOW: &str = "use host\nuse methods\nget_hold = m => m.hold\nhold = get_hold (methods.Carrier 7)\noriginal = acc fn => fn acc\nheld = hold original\nwrapped: Integer -> (Integer -> Integer) -> Integer = host.recall ()\nget = m => m.op\nv0 = 7\nv1 = [v0]\nfn = get (methods.M v1)\nsaved = host.saved fn\nfold (host.arg ()) wrapped [fn]\n";
const FOLD_COPY_DEEP: &str = "use host\nuse methods\nget_hold = m => m.hold\nhold = get_hold (methods.Carrier 7)\noriginal = acc fn => fn acc\nheld = hold original\nwrapped: Integer -> (Integer -> Integer) -> Integer = host.recall ()\nget = m => m.op\nv0 = 7\nv1 = [v0]\nv2 = [v1]\nv3 = [v2]\nv4 = [v3]\nv5 = [v4]\nv6 = [v5]\nv7 = [v6]\nv8 = [v7]\nv9 = [v8]\nv10 = [v9]\nv11 = [v10]\nv12 = [v11]\nv13 = [v12]\nv14 = [v13]\nv15 = [v14]\nv16 = [v15]\nv17 = [v16]\nv18 = [v17]\nv19 = [v18]\nv20 = [v19]\nv21 = [v20]\nv22 = [v21]\nv23 = [v22]\nv24 = [v23]\nv25 = [v24]\nv26 = [v25]\nv27 = [v26]\nv28 = [v27]\nv29 = [v28]\nv30 = [v29]\nv31 = [v30]\nv32 = [v31]\nv33 = [v32]\nv34 = [v33]\nv35 = [v34]\nv36 = [v35]\nv37 = [v36]\nv38 = [v37]\nv39 = [v38]\nv40 = [v39]\nv41 = [v40]\nv42 = [v41]\nv43 = [v42]\nv44 = [v43]\nv45 = [v44]\nv46 = [v45]\nv47 = [v46]\nv48 = [v47]\nv49 = [v48]\nv50 = [v49]\nv51 = [v50]\nv52 = [v51]\nv53 = [v52]\nv54 = [v53]\nv55 = [v54]\nv56 = [v55]\nv57 = [v56]\nv58 = [v57]\nv59 = [v58]\nv60 = [v59]\nfn = get (methods.M v60)\nsaved = host.saved fn\nfold (host.arg ()) wrapped [fn]\n";

#[derive(Debug)]
struct FoldCopyReceipt {
    before: u64,
    final_charge: u64,
    entry: usize,
}

#[derive(Default)]
struct FoldCopyObservation {
    calls: Vec<String>,
    reducer_entered: bool,
    test_entries: usize,
    protocol_seen: bool,
    fold_seen: bool,
    before: Option<u64>,
    stored: Option<super::Value>,
    guarded_item: Option<std::rc::Rc<fipe::value::Closure>>,
    item: Option<std::rc::Rc<fipe::value::Closure>>,
    reducer: Option<std::rc::Rc<fipe::value::Closure>>,
    original: Option<std::rc::Rc<fipe::value::Closure>>,
    entry: Option<usize>,
    errors: Vec<String>,
}

fn fold_copy_test(
    machine: &Machine,
    observed: &mut FoldCopyObservation,
) -> (Option<usize>, Option<usize>) {
    {
        let joined = machine.program().unwrap();
        let tables = joined.tables.borrow();
        let found: Vec<_> = tables
            .tests
            .iter()
            .enumerate()
            .filter(|(_, test)| {
                tables.namespaces[test.namespace] == "methods"
                    && FOLD_COPY_METHODS.get(test.span.start..test.span.end)
                        == Some("self > 0")
            })
            .map(|(index, _)| index)
            .collect();
        let test = if let [index] = found.as_slice() {
            Some(*index)
        } else {
            observed
                .errors
                .push(format!("Positive Test catalog: {found:?}"));
            None
        };
        let body = test.and_then(|index| joined.tests[index]);
        if body.is_none() {
            observed.errors.push("Positive Test body absent".into());
        }
        (test, body)
    }
}

fn observe_fold_copy_entry(
    machine: &Machine,
    source: &str,
    test_body: Option<usize>,
    observed: &mut FoldCopyObservation,
) {
    use fipe::{mir::Statement, value::Value, vm::Frame};
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let body = &joined.bodies[frame.body];
        let first = body.blocks[0]
            .statements
            .iter()
            .position(|s| !matches!(s, Statement::Charge(_)))
            .unwrap_or(body.blocks[0].statements.len());
        if frame.block == 0 && frame.statement == first {
            if test_body == Some(frame.body) {
                observed.test_entries += 1;
            }
            if observed.original.as_ref().is_some_and(
                |function: &std::rc::Rc<fipe::value::Closure>| {
                    function.body == frame.body
                },
            ) {
                observed.reducer_entered = true;
                let tables = joined.tables.borrow();
                let slots: Vec<_> = body
                    .locals
                    .iter()
                    .enumerate()
                    .skip(1)
                    .take(body.arity)
                    .filter_map(|(index, local)| {
                        let (namespace, span) =
                            tables.slots.get(local.slot?)?.name?;
                        (tables.namespaces[namespace] == "main"
                            && source.get(span.start..span.end) == Some("fn"))
                        .then_some(index)
                    })
                    .collect();
                match slots.as_slice() {
                    [slot] if body.arity == 2 => {
                        match frame.locals.get(*slot) {
                            Some(Value::Function(function)) => {
                                observed.guarded_item = Some(function.clone());
                            }
                            other => observed.errors.push(format!(
                                "current reducer fn absent: {other:?}"
                            )),
                        }
                    }
                    _ => observed.errors.push(format!(
                        "reducer arity/slots: {} {slots:?}",
                        body.arity
                    )),
                }
            }
        }
    }
}

fn observe_fold_copy_original(
    machine: &Machine,
    source: &str,
    observed: &mut FoldCopyObservation,
) {
    use fipe::{
        mir::{Rvalue, Statement},
        value::Value,
        vm::Frame,
    };
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let body = &joined.bodies[frame.body];
        let block = &body.blocks[frame.block];
        if let Some(Statement::Assign(_, Rvalue::Admit { values, .. })) =
            block.statements.get(frame.statement)
        {
            let tables = joined.tables.borrow();
            let node = &tables.nodes[block.nodes[frame.statement]];
            if tables.namespaces[node.namespace] == "main"
                && source.get(node.span.start..node.span.end)
                    == Some("hold original")
                && observed.original.is_none()
            {
                match owned_guard_operand(machine, frame, values[1]) {
                    Some(Value::Function(function)) => {
                        observed.original = Some(function);
                    }
                    other => observed.errors.push(format!(
                        "original before hold admission: {other:?}"
                    )),
                }
            }
        }
    }
}

fn observe_fold_copy_boundary(
    machine: &Machine,
    source: &str,
    healthy: bool,
    depth: usize,
    budget: Option<u64>,
    observed: &mut FoldCopyObservation,
) {
    use fipe::{
        mir::{Callee, Terminator},
        value::Value,
        vm::Frame,
    };
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        let body = &joined.bodies[frame.body];
        let block = &body.blocks[frame.block];
        if frame.statement == block.statements.len() {
            if let Terminator::Call { callee, args, .. } = &block.terminator {
                let tables = joined.tables.borrow();
                let node = &tables.nodes[*block.nodes.last().unwrap()];
                if tables.namespaces[node.namespace] == "main"
                    && source.get(node.span.start..node.span.end)
                        == Some("fold (host.arg ()) wrapped [fn]")
                    && observed.before.is_none()
                {
                    observed.before = Some(machine.charged());
                    let native_fold = matches!(callee, Callee::Native(index) if tables.namespaces[tables.declarations[*index].namespace] == "prelude" && tables.declarations[*index].label == "fold");
                    observed.fold_seen = native_fold;
                    if !native_fold {
                        observed.errors.push(format!("final fold callee is not native prelude fold: {callee:?}"));
                    }
                    let values: Vec<_> = args
                        .iter()
                        .map(|operand| {
                            owned_guard_operand(machine, frame, *operand)
                        })
                        .collect();
                    match values.as_slice() {
                        [Some(Value::Int(acc)), Some(Value::Function(function)), Some(Value::Seq(items))] =>
                        {
                            if *acc != if healthy { 5 } else { 0 } {
                                observed
                                    .errors
                                    .push(format!("fold accumulator {acc}"));
                            }
                            if !observed.reducer.as_ref().is_some_and(|held| {
                                std::rc::Rc::ptr_eq(held, function)
                            }) {
                                observed.errors.push("fold callback differs from actual held reducer".into());
                            }
                            if !matches!(items.as_slice(), [Value::Function(function)] if observed.item.as_ref().is_some_and(|saved| std::rc::Rc::ptr_eq(saved, function)))
                            {
                                observed.errors.push(
                                    "fold item differs from saved adapter"
                                        .into(),
                                );
                            }
                        }
                        _ => observed
                            .errors
                            .push(format!("actual fold operands: {values:?}")),
                    }
                    let before = &observed.before;
                    println!("FOLD_COPY_BOUNDARY depth={depth} healthy={healthy} budget={budget:?} before={before:?} callee={callee:?}");
                }
            }
        }
    }
}

fn fold_copy_hold(
    tables: &fipe::ir::Tables,
    declaration: usize,
    arguments: &[super::Value],
    observed: &mut FoldCopyObservation,
) -> super::Value {
    use fipe::{
        ir::{Implementation, Type},
        value::Value,
    };
    let [Value::Function(function), Value::Record(receiver)] = arguments else {
        panic!("hold Function/self protocol")
    };
    assert_eq!(receiver.fields, [Value::Int(7)]);
    let rows: Vec<_> = tables.dispatch.iter().flat_map(|dispatch| &dispatch.rows).filter(|row| matches!(&row.target, Implementation::Declared(index) if *index == declaration) && matches!(&row.receiver, Type::Meaning { index, .. } if *index == receiver.meaning)).collect();
    if let [row] = rows.as_slice() {
        observed.entry = row.input_contract;
    } else {
        observed
            .errors
            .push(format!("hold row candidates: {rows:?}"));
    }
    let entry = &observed.entry;
    println!(
        "FOLD_COPY_HELD entry={entry:?} guards={:?}",
        function.guards
    );
    observed.reducer = Some(function.clone());
    observed.stored = Some(Value::Function(function.clone()));
    Value::Int(7)
}

fn fold_copy_receiver_depths(
    callable: &fipe::ir::Type,
    tables: &fipe::ir::Tables,
    errors: &mut Vec<String>,
) -> Vec<usize> {
    use fipe::ir::Type;
    let mut pending = vec![callable];
    let mut receiver_depths = Vec::new();
    while let Some(ty) = pending.pop() {
        match ty {
            Type::Function { parameter, result } => {
                pending.push(parameter);
                pending.push(result);
            }
            Type::Meaning { index, arguments }
                if tables.namespaces[tables.meanings[*index].namespace]
                    == "methods"
                    && tables.meanings[*index].label == "M"
                    && arguments.len() == 1 =>
            {
                let mut ty = &arguments[0];
                let mut actual_depth = 0;
                while let Type::Array { element, length } = ty {
                    if !matches!(length.as_ref(), Type::Number(_)) {
                        errors.push(format!(
                            "array length not concrete: {length:?}"
                        ));
                    }
                    actual_depth += 1;
                    ty = element;
                }
                if matches!(ty, Type::Meaning { index, arguments } if arguments.is_empty() && tables.namespaces[tables.meanings[*index].namespace] == "prelude" && tables.meanings[*index].label == "Integer")
                {
                    receiver_depths.push(actual_depth);
                }
            }
            _ => {}
        }
    }
    receiver_depths
}

fn observe_fold_copy_protocol(
    joined: &fipe::mir::Program,
    tables: &fipe::ir::Tables,
    function: &fipe::value::Closure,
    depth: usize,
    observed: &mut FoldCopyObservation,
) {
    use fipe::ir::Implementation;
    if let Some(context) = joined
        .receiver_contexts
        .get(function.body)
        .and_then(Option::as_ref)
    {
        if let Some(result) = &context.protocol_result {
            if let Some(callable) = function.meanings.get(result.meaning) {
                let receiver_depths = fold_copy_receiver_depths(
                    callable,
                    tables,
                    &mut observed.errors,
                );
                observed.protocol_seen = receiver_depths == [depth] && joined.receiver_adapters.iter().any(|adapter| adapter.body == function.body && matches!(&tables.dispatch[adapter.member].rows[adapter.row].target, Implementation::Declared(index) if tables.declarations[*index].label == "op" && tables.namespaces[tables.declarations[*index].namespace] == "methods"));
                println!("FOLD_COPY_PROTOCOL depth={depth} body={} context={context:?} receiver_depths={receiver_depths:?} C={callable:?}", function.body);
            }
        }
    }
    if !observed.protocol_seen {
        observed.errors.push("saved item lacks direct declared adapter/full C/nested receiver association".into());
    }
}

fn assert_fold_copy_reducer(
    machine: &Machine,
    entry: usize,
    test: usize,
    original: &std::rc::Rc<fipe::value::Closure>,
    reducer: &std::rc::Rc<fipe::value::Closure>,
    item: &std::rc::Rc<fipe::value::Closure>,
) {
    use fipe::ir::Predicate;
    assert!(original.guards.is_empty());
    assert!(item.guards.is_empty());
    assert_eq!(reducer.body, original.body);
    assert_eq!(reducer.captures, original.captures);
    assert_eq!(reducer.filled, original.filled);
    assert!(reducer.filled.is_empty());
    assert_eq!(machine.program().unwrap().bodies[reducer.body].arity, 2);
    assert!(!std::rc::Rc::ptr_eq(reducer, original));
    assert_eq!(reducer.guards.len(), 2);
    for (spot, guard) in reducer.guards.iter().enumerate() {
        assert_eq!((guard.spot, guard.entry), (spot, entry));
        assert_eq!(
            matches!(&guard.predicate, Predicate::Function { .. }),
            spot == 1
        );
        assert_eq!(bound_order_test_ids(&guard.predicate), [test]);
    }
    assert!(bound_order_test_ids(
        &machine.program().unwrap().tables.borrow().entries[entry].predicate
    )
    .contains(&test));
}

fn assert_fold_copy_result(
    entry: usize,
    test: usize,
    healthy: bool,
    result: Result<super::Value, fipe::vm::Fault>,
    observed: &mut FoldCopyObservation,
    item: &std::rc::Rc<fipe::value::Closure>,
    reducer: &std::rc::Rc<fipe::value::Closure>,
) {
    use fipe::{ir::Predicate, value::Value};
    let mut expected_calls =
        vec!["methods.hold", "host.recall", "host.saved", "host.arg"];
    if healthy {
        assert_eq!(result.unwrap(), Value::Int(7));
        assert!(observed.reducer_entered);
        assert_eq!(observed.test_entries, 2);
        expected_calls.push("methods.op");
        let guarded = observed
            .guarded_item
            .take()
            .expect("current transformed fn argument");
        assert!(!std::rc::Rc::ptr_eq(&guarded, item));
        assert_eq!(guarded.body, item.body);
        assert_eq!(guarded.captures, item.captures);
        assert_eq!(guarded.filled, item.filled);
        assert_eq!(guarded.guards.len(), 1);
        let guard = &guarded.guards[0];
        assert_eq!((guard.spot, guard.entry), (item.filled.len(), entry));
        assert_eq!(bound_order_test_ids(&guard.predicate), [test]);
        assert!(!matches!(&guard.predicate, Predicate::Function { .. }));
        assert!(std::rc::Rc::ptr_eq(
            &guard.witnesses,
            &reducer.guards[1].witnesses
        ));
    } else {
        let fault = result.unwrap_err();
        assert!(
            matches!(fault.reason, Reason::Failed { entry: actual, .. } if actual == entry),
            "earlier scalar Failed, not later copy Budget: {fault:?}"
        );
        assert_eq!(observed.test_entries, 1);
        assert!(!observed.reducer_entered);
        assert!(observed.guarded_item.is_none());
    }
    assert_eq!(observed.calls, expected_calls);
    assert!(observed.fold_seen && observed.protocol_seen);
    assert!(
        observed.errors.is_empty(),
        "metadata observations: {:?}",
        observed.errors
    );
}

fn fold_copy_program(source: &str) -> fipe::mir::Program {
    let mut declarations = Declarations::new();
    declarations.module("methods", FOLD_COPY_METHODS).unwrap();
    declarations.module("host", FOLD_COPY_HOST).unwrap();
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    compile::compile(&unit)
}

fn fold_copy_reply(
    machine: &Machine,
    declaration: usize,
    arguments: &[super::Value],
    depth: usize,
    healthy: bool,
    observed: &mut FoldCopyObservation,
) -> super::Value {
    use fipe::value::Value;
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let d = &tables.declarations[declaration];
    let namespace = tables.namespaces[d.namespace].as_str();
    let label = d.label.as_str();
    let reply = match (namespace, label) {
        ("methods", "hold") => {
            fold_copy_hold(&tables, declaration, arguments, observed)
        }
        ("host", "recall") => {
            assert_eq!(arguments, [Value::Unit]);
            observed.stored.as_ref().expect("actual hold input").clone()
        }
        ("host", "saved") => {
            let [Value::Function(function)] = arguments else {
                panic!("saved Function protocol")
            };
            observed.item = Some(function.clone());
            observe_fold_copy_protocol(
                joined, &tables, function, depth, observed,
            );
            Value::Int(7)
        }
        ("host", "arg") => {
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(if healthy { 5 } else { 0 })
        }
        ("methods", "op") => {
            let [Value::Int(5), Value::Record(receiver)] = arguments else {
                panic!("op Integer/self protocol")
            };
            let mut value = receiver.fields.first().expect("M token");
            for _ in 0..depth {
                let Value::Seq(items) = value else {
                    panic!("nested token sequence")
                };
                assert_eq!(items.len(), 1);
                value = &items[0];
            }
            assert_eq!(*value, Value::Int(7));
            Value::Int(7)
        }
        _ => panic!("unexpected callback {namespace}.{label}"),
    };
    observed.calls.push(format!("{namespace}.{label}"));
    drop(tables);
    reply
}

fn fold_copy_receipt(
    machine: &Machine,
    budget: Option<u64>,
    before: Option<u64>,
    entry: usize,
) -> FoldCopyReceipt {
    if let Some(budget) = budget {
        assert_eq!(machine.charged(), budget);
    }
    FoldCopyReceipt {
        before: before.expect("source-linked fold boundary"),
        final_charge: machine.charged(),
        entry,
    }
}

fn fold_copy_case(
    depth: usize,
    healthy: bool,
    budget: Option<u64>,
) -> FoldCopyReceipt {
    use fipe::{
        embed::{Reply, Request},
        vm::Stop,
    };
    let source = if depth == 1 {
        FOLD_COPY_SHALLOW
    } else {
        FOLD_COPY_DEEP
    };
    let program = fold_copy_program(source);
    let mut machine = Machine::new(budget);
    machine.begin(&program);
    let mut observed = FoldCopyObservation::default();
    let (test, test_body) = fold_copy_test(&machine, &mut observed);
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(&stop, Stop::Before(_)) {
            observe_fold_copy_entry(&machine, source, test_body, &mut observed);
            observe_fold_copy_original(&machine, source, &mut observed);
            observe_fold_copy_boundary(
                &machine,
                source,
                healthy,
                depth,
                budget,
                &mut observed,
            );
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let reply = fold_copy_reply(
                    &machine,
                    declaration,
                    &arguments,
                    depth,
                    healthy,
                    &mut observed,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Request(other) => panic!("unexpected request {other:?}"),
            Stop::Done(result) => {
                let (
                    before,
                    entry,
                    test_entries,
                    calls,
                    reducer_entered,
                    errors,
                ) = (
                    &observed.before,
                    &observed.entry,
                    observed.test_entries,
                    &observed.calls,
                    observed.reducer_entered,
                    &observed.errors,
                );
                println!("FOLD_COPY_TERMINAL depth={depth} healthy={healthy} budget={budget:?} before={before:?} charged={} entry={entry:?} test={test:?} tests={test_entries} calls={calls:?} reducer_entered={reducer_entered} result={result:?} errors={errors:?}", machine.charged());
                let entry = observed
                    .entry
                    .expect("actual selected hold input contract");
                let test = test.expect("actual Positive Test");
                let original = observed
                    .original
                    .take()
                    .expect("original pre-admission alias");
                let reducer =
                    observed.reducer.take().expect("actual held reducer");
                let item = observed.item.take().expect("actual saved adapter");
                assert_fold_copy_reducer(
                    &machine, entry, test, &original, &reducer, &item,
                );
                assert_fold_copy_result(
                    entry,
                    test,
                    healthy,
                    result,
                    &mut observed,
                    &item,
                    &reducer,
                );
                return fold_copy_receipt(
                    &machine,
                    budget,
                    observed.before,
                    entry,
                );
            }
        }
    }
    panic!("bounded Fold producer must finish");
}

#[test]
fn scalar_refusal_precedes_later_function_copy_in_fold_queue() {
    let shallow = fold_copy_case(1, false, None);
    let deep = fold_copy_case(60, false, None);
    let healthy = fold_copy_case(60, true, None);
    let scalar = shallow.final_charge.checked_sub(shallow.before).unwrap();
    assert_eq!(deep.final_charge.checked_sub(deep.before), Some(scalar));
    let allowance = deep.before.checked_add(scalar).unwrap();
    let cut = fold_copy_case(60, false, Some(allowance));
    assert_eq!(cut.entry, deep.entry);
    assert_eq!(healthy.entry, deep.entry);
    assert_eq!(cut.before, deep.before);
    assert_eq!(cut.final_charge, allowance);
}
