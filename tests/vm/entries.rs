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
    let mut machine = Machine::new(1 << 20);
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
    let source =
        "f: () -> Vec<Integer> = _ => [1, 300, 400]\nx: Vec<U8> = f ()\nx";
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
