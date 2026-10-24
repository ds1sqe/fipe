use super::run::{run, session, shown};
use fipe::value::Reason;

#[test]
fn a_known_body_is_entered_with_its_spots() {
    assert_eq!(shown("add2 = x => x + 2\nadd2 4"), "6");
    assert_eq!(shown("f = (w = 80) (c = (w / 8)) => c\nf 16"), "2");
    assert_eq!(shown("f = (w = 80) (c = (w / 8)) => c\nf 16 3"), "3");
}

#[test]
fn a_session_keeps_each_line_value_and_a_closure_keeps_what_it_captured() {
    assert_eq!(
        session(&["x = 1", "f = () => x", "x = 2", "f ()"]),
        ["()", "()", "()", "1"]
    );
    assert_eq!(session(&["add2 = x => x + 2", "add2 4"]), ["()", "6"]);
}

#[test]
fn eval_apply_fills_enters_and_applies_the_rest() {
    assert_eq!(shown("app = f => (f 1) 2\napp (a b => a + b)"), "3");
    assert_eq!(
        shown("app = f => f 1 2\napp (a => (b => (a, b)))"),
        "(1, 2)"
    );
    assert_eq!(shown("app = f => f 1\n(app (a b => a + b)) 4"), "5");
    assert_eq!(
        shown("pair = a b => (x => (a, b, x))\npair 1 2 3"),
        "(1, 2, 3)"
    );
    assert_eq!(shown("k = (f => f 1) (x => x + 1)\nk"), "2");
    assert_eq!(
        shown("twice = f => (x => f (f x))\ntwice (n => n * 2) 3"),
        "12"
    );
}

#[test]
fn an_open_spot_makes_a_function_of_the_rest() {
    assert_eq!(shown("pair = a b => (a, b)\n(pair --b 2) 1"), "(1, 2)");
}

#[test]
fn conditionals_blocks_and_binds_run_in_order() {
    assert_eq!(shown("if 1 == 1 { 2 } else { 3 }"), "2");
    assert_eq!(shown("if 1 == 2 { 2 } else { 3 }"), "3");
    assert_eq!(shown("{\n    x = 1\n    y = x + 1\n    y * 2\n}"), "4");
    assert_eq!(shown("with cwd /tmp { 1 }"), "1");
}

#[test]
fn a_recursive_definition_grows_the_frames_not_the_stack() {
    let source =
        "count = n => if n == 0 { 0 } else { 1 + (count (n - 1)) }\ncount 2000";
    assert_eq!(shown(source), "2000");
}

/// A tail recursion `100 000` calls deep runs on a thread whose Rust
/// stack holds a few kilobytes per call at most: the frames are the
/// machine's, on the heap.
#[test]
fn a_recursion_a_hundred_thousand_deep_runs_on_a_small_stack() {
    let deep = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let source = "f = n => if n == 0 { 0 } else { f (n - 1) }\n\
                          f 100000";
            let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
            let unit = fipe::check::check(
                &module,
                &fipe::decl::Declarations::new(),
                &[],
                fipe::check::Options::default(),
            )
            .unwrap();
            let program = fipe::compile::compile(&unit);
            let mut machine = fipe::vm::Machine::new(Some(1 << 24));
            let fipe::embed::Outcome::Done(Ok(value)) = machine.run(&program)
            else {
                panic!("the run ends with a value")
            };
            let shown =
                fipe::value::structural::show(&value, &program.tables.borrow());
            (shown, machine.charged())
        })
        .unwrap();
    let (value, charged) = deep.join().unwrap();
    assert_eq!(value, "0");
    assert!(charged < 1 << 24, "{charged}");
}

#[test]
fn a_host_call_is_a_request_and_a_failed_reply_faults() {
    let module = fipe::parse::parse(fipe::lex::lex("\"a\" | put ./f")).unwrap();
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    let unit = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let program = fipe::compile::compile(&unit);
    let mut machine = fipe::vm::Machine::new(Some(super::run::BUDGET));
    let fipe::embed::Outcome::Request(fipe::embed::Request::Call { .. }) =
        machine.run(&program)
    else {
        panic!("the sink literal's reader is a host call")
    };
    let reply = fipe::embed::Reply::Value(Err("no host".into()));
    match machine.reply(&program, reply) {
        fipe::embed::Outcome::Done(Err(fault)) => {
            assert_eq!(fault.reason, Reason::Host("no host".into()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_run_charges_the_machine() {
    let module = fipe::parse::parse(fipe::lex::lex("1 + 2")).unwrap();
    let unit = fipe::check::check(
        &module,
        &fipe::decl::Declarations::new(),
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let program = fipe::compile::compile(&unit);
    let mut machine = fipe::vm::Machine::new(Some(super::run::BUDGET));
    let _ = machine.run(&program);
    assert!(machine.charged() > 0);
    assert_eq!(machine.budget(), Some(super::run::BUDGET));
    assert!(machine.frames().is_empty());
    let _ = run("1");
}

const OWNER_BODY: &str = "use csv\ntype W<T> { value: T }\nimpl W<a> { probe: () -> Integer = _ => { x: a = csv.get ()\n1 }, call: Self -> Integer = self => Self.probe () }\nx: W<Integer> = W 1\nx.call";

fn owner_program(source: &str) -> fipe::mir::Program {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("csv", "pub get: () -> a").unwrap();
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let unit = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    fipe::compile::compile(&unit)
}

#[test]
fn owner_static_body_only_parameter_rejects_wrong_host_reply() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let program = owner_program(OWNER_BODY);
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("get request")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, vec![Value::Unit]);
        let reply = if wrong {
            Value::text("wrong")
        } else {
            Value::Int(7)
        };
        let result = machine.reply(&program, Reply::Value(Ok(reply)));
        if wrong {
            assert!(
                matches!(
                    result,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Outcome::Done(Ok(Value::Int(1)))),
                "{result:?}"
            );
        }
    }
}

#[test]
fn owner_static_other_result_keeps_its_own_nominal_type() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let source="use csv\ntype W<T> { value: T }\ntype Other<T> { value: T }\nimpl W<a> { make: () -> Other<a> = _ => Other (csv.get ()), call: Self -> Other<a> = self => Self.make () }\nx: W<Integer> = W 1\nx.call";
    let program = owner_program(source);
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        assert!(matches!(
            machine.run(&program),
            Outcome::Request(Request::Call { .. })
        ));
        let result = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(
                matches!(
                    result,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{result:?}"
            );
        } else {
            let Outcome::Done(Ok(Value::Record(record))) = result else {
                panic!("other result")
            };
            assert_eq!(record.fields, [Value::Int(7)]);
            assert_eq!(
                program.tables.borrow().meanings[record.meaning].label,
                "Other"
            );
        }
    }
}

#[test]
fn owner_unused_phantom_has_no_runtime_witness_or_effect() {
    assert_eq!(shown("type Phantom<T> { value: Integer }\nimpl Phantom<a> { probe: () -> Integer = _ => 1, call: Self -> Integer = self => Self.probe () }\nx: Phantom<String> = Phantom 1\nx.call"),"1");
}

#[test]
fn owner_static_default_keeps_the_same_owner_argument() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let program=owner_program("use csv\ntype W<T> { value: T }\nimpl W<a> { probe: a -> Integer = (x = (csv.get ())) => 1, call: Self -> Integer = self => Self.probe }\nx: W<Integer> = W 1\nx.call");
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        assert!(matches!(
            machine.run(&program),
            Outcome::Request(Request::Call { .. })
        ));
        let result = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(
                matches!(
                    result,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Outcome::Done(Ok(Value::Int(1)))),
                "{result:?}"
            );
        }
    }
}

#[test]
fn owner_deferred_static_member_checks_the_completed_owner_at_runtime() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let source="use csv\ntype W<T> { value: T }\nearly: W<Integer> -> Integer = self => self.call\nlate=1\nimpl W<a> { probe: () -> Integer = _ => { x: a = csv.get ()\nlate }, call: Self -> Integer = self => Self.probe () }\nx: W<Integer> = W 1\nearly x";
    let program = owner_program(source);
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("deferred get")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Ok(Value::Int(1)))),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn owner_deferred_repeated_accesses_keep_distinct_owner_arguments() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let source = "use csv\ntype W<T> { value: T }\nearly: W<a> -> Integer = self => self.next.next.call\nlate=1\nimpl W<a> { next: Self -> W<String> = self => { x: a = csv.get ()\nW \"next\" }, call: Self -> Integer = self => late }\nx: W<Integer> = W 1\nearly x";
    let program = owner_program(source);
    for wrong_at in [None, Some(0), Some(1)] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let mut outcome = machine.run(&program);
        let mut calls = 0;
        while let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = outcome
        {
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "get"
            );
            assert_eq!(arguments, [Value::Unit]);
            assert!(calls < 2);
            let value = match (calls, wrong_at == Some(calls)) {
                (0, false) | (1, true) => Value::Int(7),
                _ => Value::text("next"),
            };
            calls += 1;
            outcome = machine.reply(&program, Reply::Value(Ok(value)));
        }
        if let Some(at) = wrong_at {
            assert_eq!(calls, at + 1);
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        } else {
            assert_eq!(calls, 2);
            assert!(
                matches!(outcome, Outcome::Done(Ok(Value::Int(1)))),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn owner_deferred_static_root_checks_the_captured_owner_at_runtime() {
    use fipe::{
        embed::{Outcome, Reply, Request},
        value::Value,
        vm::Machine,
    };
    let source="use csv\ntype W<T> { value:T }\ntype Other<T> { value:T }\nearly: W<Integer> -> Integer = self => self.call\nlate=1\nimpl W<a> {\nmake: Other<a> = { ignored=late\nOther (csv.get ()) },\ncall: Self -> a = self => Self.make.value\n}\nx: W<Integer> = W 1\nearly x\n";
    let program = owner_program(source);
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("deferred get")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, [Value::Unit]);
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            })),
        );
        if wrong {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: Reason::Host(_),
                        ..
                    }))
                ),
                "{outcome:?}"
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Ok(Value::Int(7)))),
                "{outcome:?}"
            );
        }
    }
}
