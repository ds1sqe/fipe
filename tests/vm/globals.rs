//! A tool reads the values a run stored for value definitions, by
//! definition, and reading changes nothing.
use super::{check, lex, parse, run::BUDGET, Declarations, Machine, Outcome};
use fipe::{compile, value::Value};

#[test]
fn the_values_a_run_stored_are_read_by_definition() {
    let source = "k = 6\nm = k * 7\nf = n => n + 1\nm";
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(BUDGET));
    assert!(machine.globals().next().is_none(), "nothing ran yet");
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("the run ends with a value")
    };
    assert_eq!(value, Value::Int(42));
    let tables = unit.tables();
    let stored: Vec<_> = machine
        .globals()
        .map(|(definition, value)| {
            (tables.definitions[definition].label.clone(), value.clone())
        })
        .collect();
    assert_eq!(
        stored,
        [
            (String::from("k"), Value::Int(6)),
            (String::from("m"), Value::Int(42)),
        ],
        "a function definition stores no value"
    );
    assert_eq!(machine.globals().count(), 2, "reading changes nothing");
}
