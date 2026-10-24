//! A tool reads the values a run stored for value definitions, by
//! definition, and reading changes nothing.
use super::{
    check, host, lex, parse, run::BUDGET, Declarations, Machine, Outcome,
};
use fipe::{
    compile,
    embed::{Reply, Request},
    mir::Program,
    session::Session,
    value::{Reason, Value},
    vm::Stop,
};
use std::rc::Rc;

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

#[test]
fn unrelated_tables_refresh_equal_length_indexed_caches() {
    let first = host::program("1 + 2");
    let second = host::program("fault \"stop\"");
    assert!(!Rc::ptr_eq(&first.tables, &second.tables));
    // Equal cache lengths must not substitute for the Tables identity.
    {
        let first = first.tables.borrow();
        let second = second.tables.borrow();
        assert_eq!(first.declarations.len(), second.declarations.len());
        assert_eq!(first.meanings.len(), second.meanings.len());
    }
    let mut machine = Machine::new(None);
    assert_eq!(machine.run(&first), Outcome::Done(Ok(Value::Int(3))));
    let Outcome::Done(Err(fault)) = machine.run(&second) else {
        panic!("the second program faults normally")
    };
    assert_eq!(fault.reason, Reason::Message(String::from("stop")));
    assert_eq!(
        (fault.span.start, fault.span.end),
        (0, "fault \"stop\"".len())
    );
}

#[test]
fn beginning_a_smaller_context_drops_old_stored_globals() {
    let first = host::program("secret = 99\nsecret");
    let second = host::program("1");
    let mut machine = Machine::new(None);
    assert_eq!(machine.run(&first), Outcome::Done(Ok(Value::Int(99))));
    assert!(machine.globals().any(|(_, value)| *value == Value::Int(99)));
    machine.begin(&second);
    assert_eq!(machine.globals().count(), 0);
    loop {
        match machine.next_stop() {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Done(value) => {
                assert_eq!(value, Ok(Value::Int(1)));
                break;
            }
            other @ Stop::Request(_) => panic!("pure context: {other:?}"),
        }
    }
    assert_eq!(machine.globals().count(), 0);
}

fn module_context() -> Program {
    let module = parse(lex("use item\nitem.value")).unwrap();
    let file = parse(lex("use cli\npub value = cli.port")).unwrap();
    let unit = check::check(
        &module,
        &host::declarations(),
        &[(String::from("item"), file)],
        check::Options::default(),
    )
    .unwrap();
    compile::compile(&unit)
}

#[test]
fn independent_module_initialization_never_reuses_or_resurrects_old_values() {
    let first = module_context();
    let second = module_context();
    assert!(!Rc::ptr_eq(&first.tables, &second.tables));
    assert_eq!(
        first.tables.borrow().modules.len(),
        second.tables.borrow().modules.len()
    );
    let mut machine = Machine::new(None);
    let mut calls = 0;
    for (program, expected) in [(&first, 1), (&second, 2), (&first, 3)] {
        let Outcome::Request(request @ Request::Call { .. }) =
            machine.run(program)
        else {
            panic!("each fresh context must execute its module initializer")
        };
        assert_eq!(host::key(program, &request), "cli.port");
        calls += 1;
        assert_eq!(
            machine.reply(program, Reply::Value(Ok(Value::Int(expected)))),
            Outcome::Done(Ok(Value::Int(expected)))
        );
    }
    assert_eq!(calls, 3);
}

fn session_program(session: &mut Session, source: &str) -> Program {
    let Outcome::Done(report) = session.line(source) else {
        panic!("no source request")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    compile::compile(report.unit.as_ref().unwrap())
}

#[test]
fn additional_shared_session_programs_keep_captured_values() {
    let mut session = Session::new(Declarations::new());
    let first = session_program(&mut session, "base = 40\nf = n => base + n");
    let second = session_program(&mut session, "f 2");
    assert!(Rc::ptr_eq(&first.tables, &second.tables));
    let mut machine = Machine::new(None);
    assert_eq!(machine.run(&first), Outcome::Done(Ok(Value::Unit)));
    assert_eq!(machine.run(&second), Outcome::Done(Ok(Value::Int(42))));
    assert!(machine.globals().any(|(_, value)| *value == Value::Int(40)));
}
