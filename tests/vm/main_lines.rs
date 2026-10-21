//! A host reads each main expression line's value, in line order, from
//! a plain run or a stepped one; a binding line gives none.
use super::{check, lex, parse, run::BUDGET, Declarations, Machine, Outcome};
use fipe::{compile, mir::Program, value::Value, vm::Stop};

fn compiled(source: &str) -> (Program, String) {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    (compile::compile(&unit), String::from(source))
}

/// Each kept line as its source text and value.
fn kept(
    machine: &Machine,
    program: &Program,
    source: &str,
) -> Vec<(String, Value)> {
    let tables = program.tables.borrow();
    machine
        .lines()
        .iter()
        .map(|(node, value)| {
            let span = tables.nodes[*node].span;
            (String::from(&source[span.start..span.end]), value.clone())
        })
        .collect()
}

#[test]
fn each_expression_line_of_a_run_is_kept_in_order() {
    let (program, source) = compiled("1 + 1\nx = 3\nx * 2\n\"a\"");
    let mut machine = Machine::new(Some(BUDGET));
    assert!(machine.lines().is_empty(), "nothing ran yet");
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("the run ends with a value")
    };
    assert_eq!(value, Value::Text("a".to_string().into()));
    assert_eq!(
        kept(&machine, &program, &source),
        [
            (String::from("1 + 1"), Value::Int(2)),
            (String::from("x * 2"), Value::Int(6)),
            (String::from("\"a\""), Value::Text("a".to_string().into())),
        ],
        "the binding line gives no value"
    );
    assert_eq!(machine.lines().len(), 3, "reading changes nothing");
}

#[test]
fn a_line_after_a_fault_is_not_kept_and_a_new_run_starts_empty() {
    let (program, source) = compiled("2\n1.0 / 0.0\n3");
    let mut machine = Machine::new(Some(BUDGET));
    let Outcome::Done(Err(_)) = machine.run(&program) else {
        panic!("the second line Faults")
    };
    assert_eq!(
        kept(&machine, &program, &source),
        [(String::from("2"), Value::Int(2))]
    );
    let (next, _) = compiled("x = 1");
    let _ = machine.run(&next);
    assert!(machine.lines().is_empty(), "a binding-only run keeps none");
}

#[test]
fn a_stepped_run_keeps_the_same_lines_and_charges() {
    let (program, _) = compiled("f = n => n + 1\nf 1\ny = f 2\n[f 3, 4]");
    let mut plain = Machine::new(Some(BUDGET));
    let _ = plain.run(&program);
    let mut stepped = Machine::new(Some(BUDGET));
    stepped.begin(&program);
    while !matches!(stepped.next_stop(), Stop::Done(_)) {}
    assert_eq!(plain.lines(), stepped.lines());
    assert_eq!(plain.charged(), stepped.charged());
    assert_eq!(plain.lines().len(), 2);
}

#[test]
fn the_lines_a_main_lift_holds_are_kept_when_they_run() {
    let source = "1 + 1\nm = Just 5\n[:e] = m\ne + 1\ne + 2";
    let (program, source) = compiled(source);
    let mut machine = Machine::new(Some(BUDGET));
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("the run ends with a value")
    };
    let shown = |value: &Value| {
        fipe::value::structural::show(value, &program.tables.borrow())
    };
    assert_eq!(shown(&value), "Just 7", "the lift rebuilds the last value");
    let kept: Vec<(String, String)> = kept(&machine, &program, &source)
        .iter()
        .map(|(text, value)| (text.clone(), shown(value)))
        .collect();
    assert_eq!(
        kept,
        [
            (String::from("1 + 1"), String::from("2")),
            (String::from("e + 1"), String::from("6")),
            (String::from("e + 2"), String::from("7")),
        ],
        "the lift is a binding line and gives no value"
    );
}

#[test]
fn a_main_lift_that_misses_keeps_none_of_its_lines() {
    let (program, source) =
        compiled("1 + 1\nm: Maybe<Integer> = Nothing\n[:e] = m\ne + 1");
    let mut machine = Machine::new(Some(BUDGET));
    let Outcome::Done(Ok(_)) = machine.run(&program) else {
        panic!("the run ends with a value")
    };
    assert_eq!(
        kept(&machine, &program, &source),
        [(String::from("1 + 1"), Value::Int(2))]
    );
}
