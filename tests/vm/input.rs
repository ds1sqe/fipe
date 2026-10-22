//! Host input enters the spot of the function the last top-level line
//! makes; the run's value is that function's result.
use super::host::{self, observe_input, Script};
use fipe::{
    ir,
    value::{Reason, Value},
    vm::{input_spot, InputError},
};

#[test]
fn input_enters_the_one_open_spot_of_the_last_line() {
    let program = host::program("add = a b => a + b\nadd 2");
    let observation =
        observe_input(&program, Script::default(), Value::Int(40)).unwrap();
    assert_eq!(observation.result, Ok(Value::Int(42)));
}

#[test]
fn the_function_s_result_is_the_last_line_s_value() {
    let program = host::program("add = a b => a + b\n1 + 1\nadd 2");
    let mut machine = fipe::vm::Machine::new(None);
    let outcome = machine.run_with_input(&program, Value::Int(5)).unwrap();
    assert_eq!(outcome, fipe::embed::Outcome::Done(Ok(Value::Int(7))));
    let values: Vec<_> = machine
        .lines()
        .iter()
        .map(|(_, value)| value.clone())
        .collect();
    assert_eq!(values, [Value::Int(2), Value::Int(7)]);
}

#[test]
fn input_reaches_a_host_function_and_its_calls_are_traced() {
    let program = host::program("text.trim");
    let script =
        Script::default().reply("text.trim", Ok(Value::text("trimmed")));
    let observation =
        observe_input(&program, script, Value::text("  trimmed ")).unwrap();
    assert_eq!(observation.result, Ok(Value::text("trimmed")));
    let [(fipe::embed::Request::Call { arguments, .. }, _)] =
        observation.trace.as_slice()
    else {
        panic!("one host call: {:?}", observation.trace)
    };
    assert_eq!(arguments, &[Value::text("  trimmed ")]);
}

#[test]
fn input_of_another_shape_faults_before_any_effect() {
    let program = host::program("cli.port\nadd = a b => a + b\nadd 2");
    let observation =
        observe_input(&program, Script::default(), Value::text("forty"))
            .unwrap();
    assert!(observation.trace.is_empty(), "cli.port was not asked");
    let fault = observation.result.unwrap_err();
    assert_eq!(
        fault.reason,
        Reason::Host(String::from("the input is not an Integer"))
    );
}

#[test]
fn a_last_line_of_another_open_arity_is_refused_unstarted() {
    for (source, spots) in [
        (
            "cli.port\nadd: Integer -> Integer -> Integer = a b => a + b\nadd",
            2,
        ),
        ("cli.port\n1 + 1", 0),
        ("cli.port\nf = a => a + 1", 0),
    ] {
        let program = host::program(source);
        let refused = observe_input(&program, Script::default(), Value::Int(1));
        assert_eq!(refused, Err(InputError { spots }), "{source}");
    }
}

#[test]
fn the_spot_s_type_tells_a_host_how_to_decode_its_bytes() {
    let program = host::program("text.trim");
    let spot = input_spot(&program).unwrap();
    let tables = program.tables.borrow();
    let ir::Type::Meaning { index, .. } = spot else {
        panic!("a meaning, not {spot:?}")
    };
    assert_eq!(tables.meanings[index].label, "String");
}

#[test]
fn the_refusal_reads_in_words() {
    assert_eq!(
        InputError { spots: 2 }.to_string(),
        "host input enters a last line that is a function of one open \
         spot; this one has 2"
    );
    assert_eq!(
        InputError { spots: 0 }.to_string(),
        "host input enters a last line that is a function of one open \
         spot; this one is not a function"
    );
}

#[test]
fn a_stepped_run_applies_the_input_at_its_end() {
    use fipe::vm::{Machine, Stop};
    let program = host::program("add = a b => a + b\n1 + 1\nadd 2");
    let mut machine = Machine::new(None);
    machine.begin_with_input(&program, Value::Int(5)).unwrap();
    let result = loop {
        if let Stop::Done(result) = machine.next_stop() {
            break result;
        }
    };
    assert_eq!(result, Ok(Value::Int(7)));
    let values: Vec<_> = machine
        .lines()
        .iter()
        .map(|(_, value)| value.clone())
        .collect();
    assert_eq!(values, [Value::Int(2), Value::Int(7)]);
    // Input of another shape Faults at the last line at the first stop.
    let mut machine = Machine::new(None);
    machine
        .begin_with_input(&program, Value::text("five"))
        .unwrap();
    let Stop::Done(Err(fault)) = machine.next_stop() else {
        panic!("a Fault before any effect")
    };
    assert!(matches!(fault.reason, Reason::Host(_)), "{fault:?}");
    assert!(matches!(
        Machine::new(None).begin_with_input(&host::program("1"), Value::Int(1)),
        Err(InputError { spots: 0 })
    ));
}
