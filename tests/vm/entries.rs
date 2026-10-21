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
