//! Genuine references enter through checked Host result types, never data values.
use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
use fipe::{
    compile,
    embed::Reply,
    value::resource::{ResourceDomain, ResourceError, ResourceType},
};
use std::rc::Rc;

fn program(declarations: &str, source: &str) -> fipe::mir::Program {
    let mut host = Declarations::new();
    host.module("csv", declarations).unwrap();
    let module = parse(lex(source)).unwrap();
    compile::compile(
        &check::check(&module, &host, &[], check::Options::default()).unwrap(),
    )
}
#[test]
fn resource_registered_and_pending_share_exact_checked_identity() {
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut machine = Machine::new(None);
    assert_eq!(
        machine.pending_resource_type(),
        Err(ResourceError::NoPendingCall)
    );
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let pending = machine.pending_resource_type().unwrap();
    let registered =
        ResourceType::registered(&program.tables.borrow(), "csv", "Table")
            .unwrap();
    assert_eq!(pending, registered);
    let domain = ResourceDomain::new();
    let value = domain.issue(&pending, Rc::from(&b"private-entry"[..]));
    assert_eq!(format!("{value:?}"), "Resource(<resource>)");
    let Outcome::Done(Ok(returned)) =
        machine.reply(&program, Reply::Value(Ok(value.clone())))
    else {
        panic!("resource reply")
    };
    assert_eq!(returned, value);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
        Outcome::Done(Err(_))
    ));
}
#[test]
fn resource_fresh_tables_reject_same_name_and_slot() {
    let first = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let second = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&first), Outcome::Request(_)));
    let value = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    assert!(machine.abandon());
    assert!(matches!(machine.run(&second), Outcome::Request(_)));
    assert!(matches!(
        machine.reply(&second, Reply::Value(Ok(value))),
        Outcome::Done(Err(_))
    ));
}

#[test]
fn resource_pending_generic_arguments_and_typed_containers() {
    let program=program("pub type resource Rd<T>\npub unit: Rd<()>\npub integer: Rd<Integer>\npub sequence: Vec<Rd<()>>","use csv\nx=csv.unit\ny=csv.integer\ncsv.sequence");
    let mut machine = Machine::new(None);
    let domain = ResourceDomain::new();
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let unit = machine.pending_resource_type().unwrap();
    let alias = domain.issue(&unit, Rc::from(&b"u"[..]));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Request(_)
    ));
    let integer = machine.pending_resource_type().unwrap();
    assert_ne!(unit, integer);
    let integer = domain.issue(&integer, Rc::from(&b"i"[..]));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(integer))),
        Outcome::Request(_)
    ));
    assert_eq!(
        machine.pending_resource_type(),
        Err(ResourceError::NotResourceResult)
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::seq(vec![alias])))),
        Outcome::Done(Ok(_))
    ));
}
#[test]
fn resource_cannot_hide_in_opaque_data() {
    let program = program(
        "pub type resource Table\npub open: Table\npub data: Packet",
        "use csv\nx=csv.open\ncsv.data",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Request(_)
    ));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::seq(vec![alias])))),
        Outcome::Done(Err(_))
    ));
}
#[test]
fn resource_input_uses_checked_identity_and_preserves_bad_arity_run() {
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    assert!(machine.run_with_input(&program, Value::Unit).is_err());
    let ty = machine.pending_resource_type().unwrap();
    let alias = ResourceDomain::new().issue(&ty, Rc::from(&b"entry"[..]));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(alias))),
        Outcome::Done(Ok(_))
    ));
}

fn session_program(
    session: &mut fipe::session::Session,
    source: &str,
) -> fipe::mir::Program {
    let Outcome::Done(report) = session.line(source) else {
        panic!("checked line")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    compile::compile(report.unit.as_ref().expect("checked unit"))
}
#[test]
fn resource_session_forks_are_fresh_before_and_after_first_stamp() {
    for populated in [false, true] {
        let mut host = Declarations::new();
        host.module("csv", "pub type resource Table\npub open: Table")
            .unwrap();
        let mut original = fipe::session::Session::new(host);
        let program = session_program(&mut original, "use csv\ncsv.open");
        let mut fork = (!populated).then(|| original.clone());
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let ty = machine.pending_resource_type().unwrap();
        if populated {
            fork = Some(original.clone());
        }
        let value = ResourceDomain::new().issue(&ty, Rc::from(&b"entry"[..]));
        let fork_program = session_program(fork.as_mut().unwrap(), "csv.open");
        assert!(machine.abandon());
        assert!(matches!(machine.run(&fork_program), Outcome::Request(_)));
        assert_ne!(ty, machine.pending_resource_type().unwrap());
        assert!(matches!(
            machine.reply(&fork_program, Reply::Value(Ok(value))),
            Outcome::Done(Err(_))
        ));
    }
}
#[test]
fn resource_withdrawal_rejects_reused_ordinary_argument_slot() {
    let mut host = Declarations::new();
    host.module("csv", "pub type resource Rd<T>\npub open: () -> Rd<a>")
        .unwrap();
    let mut session = fipe::session::Session::new(host);
    let _ = session_program(
        &mut session,
        "use csv\nf: csv.Rd<()> -> csv.Rd<()> = x => x",
    );
    let mut prior = None;
    let mut prior_index = None;
    let domain = ResourceDomain::new();
    for _ in 0..2 {
        assert!(matches!(
            session.line("use tags\nx: csv.Rd<tags.Mark> = csv.open ()\nx"),
            Outcome::Request(_)
        ));
        let Outcome::Done(report) =
            session.reply(Reply::File(Some("pub type Mark {}".into())))
        else {
            panic!("checked file")
        };
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let program = compile::compile(report.unit.as_ref().unwrap());
        let index = program
            .tables
            .borrow()
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Mark")
            .unwrap();
        if let Some(old) = prior_index {
            assert_eq!(old, index);
        }
        prior_index = Some(index);
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let ty = machine.pending_resource_type().unwrap();
        let value = domain.issue(&ty, Rc::from(&b"same-live-entry"[..]));
        if let Some((old_ty, old_value)) = prior.take() {
            assert_ne!(old_ty, ty);
            assert!(matches!(
                machine.reply(&program, Reply::Value(Ok(old_value))),
                Outcome::Done(Err(_))
            ));
        } else {
            assert!(machine.abandon());
        }
        prior = Some((ty, value));
        assert!(session.withdraw(report.accepted.unwrap()).is_some());
    }
}

#[test]
fn resource_full_module_marker_and_unit_are_distinct_arguments() {
    let mut host = Declarations::new();
    host.commands("pub type resource Rd<T>").unwrap();
    host.module("full.api", "pub open: Rd<full.api>\npub unit: Rd<()>")
        .unwrap();
    let module =
        parse(lex("use full.api\nx=full.api.open\nfull.api.unit")).unwrap();
    let program = compile::compile(
        &check::check(&module, &host, &[], check::Options::default()).unwrap(),
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let module = machine.pending_resource_type().unwrap();
    let value = ResourceDomain::new().issue(&module, Rc::from(&b"entry"[..]));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(value.clone()))),
        Outcome::Request(_)
    ));
    assert_ne!(module, machine.pending_resource_type().unwrap());
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(value))),
        Outcome::Done(Err(_))
    ));
}
#[test]
fn resource_typed_functions_may_capture_aliases_but_data_cannot() {
    for (declaration, valid) in [
        ("pub result: () -> ((() -> Integer) * ())", true),
        ("pub result: () -> Packet", false),
    ] {
        let program = program(
            &format!("pub type resource Table\npub open: Table\n{declaration}"),
            "use csv\nx=csv.open\ncsv.result ()",
        );
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let alias = ResourceDomain::new().issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"entry"[..]),
        );
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(alias.clone()))),
            Outcome::Request(_)
        ));
        let closure = Value::Function(Rc::new(fipe::value::Closure {
            body: 0,
            captures: vec![alias],
            meanings: vec![],
            filled: vec![],
            guards: vec![],
        }));
        let returned = if valid {
            Value::Tuple(Rc::new(vec![closure, Value::Unit]))
        } else {
            closure
        };
        let Outcome::Done(result) =
            machine.reply(&program, Reply::Value(Ok(returned)))
        else {
            panic!("terminal")
        };
        assert_eq!(result.is_ok(), valid);
    }
}
#[test]
fn resource_work_budget_refuses_before_call_and_after_reply() {
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut unbounded = Machine::new(None);
    assert!(matches!(unbounded.run(&program), Outcome::Request(_)));
    let before = unbounded.charged();
    assert!(before > 1);
    let mut limited = Machine::new(Some(before - 1));
    assert!(matches!(
        limited.run(&program),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Budget,
            ..
        }))
    ));
    assert_eq!(limited.charged(), before - 1);
    let mut limited = Machine::new(Some(before + 1));
    assert!(matches!(limited.run(&program), Outcome::Request(_)));
    // The Host has already performed its effect by the time it returns a value.
    let alias = ResourceDomain::new().issue(
        &limited.pending_resource_type().unwrap(),
        Rc::from(&b"effect-already-done"[..]),
    );
    assert!(matches!(
        limited.reply(&program, Reply::Value(Ok(alias))),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Budget,
            ..
        }))
    ));
    assert_eq!(limited.charged(), before + 1);
}

#[test]
fn resource_alias_survives_a_generic_closure_and_open_function_guard() {
    let program=program("pub type resource Rd<T>\npub open: Rd<Integer>\npub fresh: () -> Rd<a>\npub choose: () -> Bool","use csv\nmake: csv.Rd<a> -> (U8 -> Integer) = x=>(n=>{ ignored=if csv.choose () { csv.fresh () } else { x }\n n })\nf = make csv.open\ng = n => map f [n]\ng 3");
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(alias.clone())))
    else {
        panic!("guarded body choice")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "choose"
    );
    assert_eq!(arguments, [Value::Unit]);
    let no = fipe::value::Prelude::new(&program.tables.borrow()).bool(false);
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(no))),
        Outcome::Done(Ok(Value::Seq(values))) if *values == vec![Value::Int(3)]
    ));
    let (_, Value::Function(guarded)) = machine
        .globals()
        .find(|(index, _)| {
            program.tables.borrow().definitions[*index].label == "f"
        })
        .expect("stored guarded function")
    else {
        panic!("function")
    };
    assert!(
        !guarded.meanings.is_empty(),
        "must exercise actual materialized generic witnesses"
    );
    assert!(
        guarded
            .captures
            .iter()
            .chain(&guarded.filled)
            .any(|value| *value == alias),
        "must retain the issued alias"
    );
    let invalid = self::program("pub type resource Rd<T>\npub open: Rd<Integer>\npub fresh: () -> Rd<a>\npub choose: () -> Bool", "use csv\nmake: csv.Rd<a> -> (U8 -> Integer) = x=>(n=>{ ignored=if csv.choose () { csv.fresh () } else { x }\n n })\nf = make csv.open\ng = n => map f [n]\ng 300");
    let mut rejected = Machine::new(None);
    assert!(matches!(rejected.run(&invalid), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &rejected.pending_resource_type().unwrap(),
        Rc::from(&b"guard-negative"[..]),
    );
    let Outcome::Done(Err(fipe::vm::Fault {
        reason: fipe::value::Reason::Failed { entry, value },
        ..
    })) = rejected.reply(&invalid, Reply::Value(Ok(alias)))
    else {
        panic!("open guard must reject before choose callback")
    };
    assert_eq!(value, "300");
    assert!(invalid.tables.borrow().entries[entry]
        .label
        .starts_with("U8 ->"));
}
#[test]
fn resource_reply_uses_frozen_fields_and_frozen_error_site() {
    let declarations="pub type resource Table\npub open: Table\npub type Box { pub value: Table }\npub boxed: Box";
    let program = program(declarations, "use csv\nx=csv.open\ncsv.boxed");
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Request(_)
    ));
    let boxed = {
        let tables = program.tables.borrow();
        tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Box")
            .unwrap()
    };
    program.tables.borrow_mut().meanings[boxed].constructors[0].fields[0].ty =
        fipe::ir::Type::Unit;
    let value = Value::Record(Rc::new(fipe::value::Record {
        meaning: boxed,
        constructor: 0,
        fields: vec![alias],
    }));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(value))),
        Outcome::Done(Ok(_))
    ));

    let direct = program_for_frozen_site();
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&direct), Outcome::Request(_)));
    let source = "use csv\ncsv.open";
    let old = (
        fipe::token::Span {
            start: source.find("csv.open").unwrap(),
            end: source.len(),
        },
        direct
            .tables
            .borrow()
            .namespaces
            .iter()
            .position(|name| name == "main")
            .unwrap(),
    );
    {
        let mut tables = direct.tables.borrow_mut();
        for node in &mut tables.nodes {
            node.span = fipe::token::Span { start: 0, end: 0 };
        }
        for meaning in &mut tables.meanings {
            if meaning.label == "Table" {
                meaning.label = "changed-private-label".into();
            }
        }
    }
    let Outcome::Done(Err(fault)) =
        machine.reply(&direct, Reply::Value(Ok(Value::Int(4))))
    else {
        panic!("type mismatch")
    };
    assert_eq!((fault.span, fault.namespace), old);
    assert!(
        matches!(fault.reason,fipe::value::Reason::Host(ref message) if message.contains("Table")&&!message.contains("changed-private-label"))
    );
}
fn program_for_frozen_site() -> fipe::mir::Program {
    program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    )
}

#[test]
fn resource_zero_length_values_skip_demand_but_identity_does_not() {
    let allowed = program(
        "pub type resource Rd<T>\npub empty: [Rd<a>; 0]",
        "use csv\ncsv.empty",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&allowed), Outcome::Request(_)));
    assert!(matches!(
        machine.reply(&allowed, Reply::Value(Ok(Value::seq(vec![])))),
        Outcome::Done(Ok(_))
    ));
    let identity = program(
        "pub type resource Rd<T>\npub impossible: Rd<[a; 0]>",
        "use csv\ncsv.impossible",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&identity), Outcome::Done(Err(_))));
}
#[test]
fn resource_unrelated_metadata_and_opaque_phantoms_do_not_force_concreteness() {
    let declarations="pub type resource Rd<T>\npub type Opaque<T>\npub unit: Rd<()>\npub pair: Rd<()> * a\npub phantom: Opaque<Rd<a>>";
    let program = program(
        declarations,
        "use csv\nx=csv.unit\ny=csv.pair\n(y,csv.phantom)",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.reply(&program, Reply::Value(Ok(alias.clone())))
    else {
        panic!("pair request")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "pair"
    );
    let paired = machine.reply(
        &program,
        Reply::Value(Ok(Value::Tuple(Rc::new(vec![alias, Value::Int(4)])))),
    );
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        paired
    else {
        panic!("phantom request: {paired:?}")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "phantom"
    );
    let Outcome::Done(Ok(Value::Tuple(values))) =
        machine.reply(&program, Reply::Value(Ok(Value::Int(8))))
    else {
        panic!("pair and opaque phantom result")
    };
    assert!(matches!(&values[0],Value::Tuple(pair) if pair[1]==Value::Int(4)));
    assert_eq!(values[1], Value::Int(8));
}
#[test]
fn resource_recursive_templates_stabilize_without_instantiation() {
    for (next, allowed) in [("Vec<T>", true), ("Rd<T>", false)] {
        let declarations=format!("pub type resource Rd<T>\npub type N<T> {{ Leaf(value:T), Next(value:N<{next}>) }}\npub get: N<a>");
        let program = program(&declarations, "use csv\ncsv.get");
        let mut machine = Machine::new(Some(100_000));
        let outcome = machine.run(&program);
        assert_eq!(
            matches!(outcome, Outcome::Request(_)),
            allowed,
            "{outcome:?}"
        );
        if !allowed {
            assert!(
                matches!(outcome,Outcome::Done(Err(fipe::vm::Fault{reason:fipe::value::Reason::Host(ref message),..})) if message=="nonconcrete resource identity")
            );
        }
    }
}

#[test]
fn resource_same_tables_and_new_machine_preserve_type_without_granting_authority(
) {
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut first = Machine::new(None);
    assert!(matches!(first.run(&program), Outcome::Request(_)));
    let ty = first.pending_resource_type().unwrap();
    let domain = ResourceDomain::new();
    let alias = domain.issue(&ty, Rc::from(&b"entry"[..]));
    assert!(matches!(
        first.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Done(Ok(_))
    ));
    let mut second = Machine::new(None);
    assert!(matches!(second.run(&program), Outcome::Request(_)));
    assert_eq!(ty, second.pending_resource_type().unwrap());
    // Type fit intentionally does not certify that the first owner is still live.
    assert!(matches!(
        second.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Done(Ok(_))
    ));
    assert_ne!(
        alias,
        ResourceDomain::new().issue(&ty, Rc::from(&b"entry"[..]))
    );
    assert_eq!(alias, domain.issue(&ty, Rc::from(&b"entry"[..])));
}
#[test]
fn resource_free_generic_empty_sum_keeps_full_zero_arguments() {
    for reverse in [false, true] {
        for wrong_at in [None, Some(0usize), Some(1usize)] {
            let expression = if reverse {
                "(ys | sum, xs | sum)"
            } else {
                "(xs | sum, ys | sum)"
            };
            let source=format!("use csv\ntype W<T>: Zero & Add {{ value: T }}\nimpl W<a>: Zero {{ zero = csv.make () }}\nimpl W<a>: Add {{ add = other self => self }}\nxs: Vec<W<Integer>> = []\nys: Vec<W<String>> = []\n{expression}");
            let program = program("pub make: () -> a", &source);
            assert!(!program.tables.borrow().has_resources);
            let meaning = program
                .tables
                .borrow()
                .meanings
                .iter()
                .position(|m| m.label == "W")
                .unwrap();
            let mut machine = Machine::new(Some(100_000));
            let mut outcome = machine.run(&program);
            let mut calls = 0;
            loop {
                match outcome {
                    Outcome::Request(fipe::embed::Request::Call {
                        declaration,
                        arguments,
                        ..
                    }) => {
                        assert!(
                            calls < 2,
                            "no later callback after a wrong reply"
                        );
                        assert_eq!(
                            program.tables.borrow().declarations[declaration]
                                .label,
                            "make"
                        );
                        assert_eq!(arguments, [Value::Unit]);
                        let integer = (calls == 0) != reverse;
                        let integer = integer != (wrong_at == Some(calls));
                        let field = if integer {
                            Value::Int(0)
                        } else {
                            Value::text("zero")
                        };
                        calls += 1;
                        outcome = machine.reply(
                            &program,
                            Reply::Value(Ok(Value::Record(Rc::new(
                                fipe::value::Record {
                                    meaning,
                                    constructor: 0,
                                    fields: vec![field],
                                },
                            )))),
                        );
                    }
                    Outcome::Done(result) => {
                        if let Some(wrong_at) = wrong_at {
                            assert_eq!(calls, wrong_at + 1);
                            assert!(
                                matches!(
                                    result,
                                    Err(fipe::vm::Fault {
                                        reason: fipe::value::Reason::Host(_),
                                        ..
                                    })
                                ),
                                "{result:?}"
                            );
                        } else {
                            assert_eq!(calls, 2);
                            let Ok(Value::Tuple(values)) = result else {
                                panic!("typed zeros")
                            };
                            for (index, value) in values.iter().enumerate() {
                                let field = if (index == 0) == reverse {
                                    Value::text("zero")
                                } else {
                                    Value::Int(0)
                                };
                                assert!(
                                    matches!(value,Value::Record(record)if record.fields==[field]),
                                    "{value:?}"
                                );
                            }
                        }
                        break;
                    }
                    other @ Outcome::Request(_) => panic!("{other:?}"),
                }
            }
        }
    }
}

#[test]
fn resource_registered_refuses_missing_ordinary_generic_and_ambiguous_types() {
    let program=program("pub type resource Rd<T>\npub type resource Table\npub get: Rd<Integer>","use csv\ncsv.get");
    let tables = program.tables.borrow();
    assert_eq!(
        ResourceType::registered(&tables, "csv", "missing"),
        Err(ResourceError::NotFound)
    );
    assert_eq!(
        ResourceType::registered(&tables, "csv", "Table"),
        Err(ResourceError::NotFound)
    );
    assert_eq!(
        ResourceType::registered(&tables, "csv", "Rd"),
        Err(ResourceError::NotMonomorphic)
    );
    assert_eq!(
        ResourceType::registered(&tables, "prelude", "Integer"),
        Err(ResourceError::NotResource)
    );
    drop(tables);
    let duplicate = {
        let tables = program.tables.borrow();
        tables
            .meanings
            .iter()
            .find(|meaning| meaning.label == "Rd")
            .unwrap()
            .clone()
    };
    program.tables.borrow_mut().meanings.push(duplicate);
    assert_eq!(
        ResourceType::registered(&program.tables.borrow(), "csv", "Rd"),
        Err(ResourceError::Ambiguous)
    );
}

#[test]
fn resource_typed_function_memo_does_not_suppress_data_only_capture_scan() {
    let program=program("pub type resource Table\npub open: Table\npub result: () -> ((() -> Integer) * Packet)","use csv\nx=csv.open\ncsv.result ()");
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(alias.clone()))),
        Outcome::Request(_)
    ));
    let function = Value::Function(Rc::new(fipe::value::Closure {
        body: 0,
        captures: vec![alias],
        meanings: vec![],
        filled: vec![],
        guards: vec![],
    }));
    assert!(matches!(
        machine.reply(
            &program,
            Reply::Value(Ok(Value::Tuple(Rc::new(vec![
                function.clone(),
                function
            ]))))
        ),
        Outcome::Done(Err(_))
    ));
}

#[test]
fn resource_packed_declared_member_preserves_concrete_receiver_alias() {
    let declarations="pub type Probe where self.inspect: Integer\npub type resource Rd<T>\npub open: Rd<()>\nimpl Rd<a>: Probe { inspect: (self: Self) -> Integer }";
    let program = program(
        declarations,
        "use csv\ng: csv.Probe -> Integer = x => x.inspect\ng csv.open",
    );
    let mut machine = Machine::new(None);
    let domain = ResourceDomain::new();
    let Outcome::Request(fipe::embed::Request::Call {
        declaration: open, ..
    }) = machine.run(&program)
    else {
        panic!("open")
    };
    assert_eq!(program.tables.borrow().declarations[open].label, "open");
    let ty = machine.pending_resource_type().unwrap();
    let alias = domain.issue(&ty, Rc::from(&b"entry"[..]));
    let outcome = machine.reply(&program, Reply::Value(Ok(alias.clone())));
    let Outcome::Request(fipe::embed::Request::Call {
        declaration: inspect,
        arguments,
        ..
    }) = outcome
    else {
        panic!("packed member must reach declared inspect: {outcome:?}")
    };
    assert_eq!(
        program.tables.borrow().declarations[inspect].label,
        "inspect"
    );
    assert_eq!(arguments, vec![alias]);
    assert!(
        machine.pending_resource_type().is_err(),
        "inspect returns Integer"
    );
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Int(9)))),
        Outcome::Done(Ok(Value::Int(9)))
    ));
}
#[test]
fn resource_dispatch_keeps_ordinary_packed_control() {
    for marker in ["", "pub type resource Unused\n"] {
        let declarations = format!("{marker}pub number: Integer");
        let program=program(&declarations,"use csv\ntype Probe where self.inspect: Integer\ntype Box: Probe { value: Integer }\nimpl Box: Probe { inspect = self => self.value + 1 }\ng: Probe -> Integer = x => x.inspect\ng (Box csv.number)");
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
            Outcome::Done(Ok(Value::Int(8)))
        ));
    }
}

#[test]
fn resource_packed_retired_argument_refuses_before_member_effect() {
    let declarations="pub type Probe where self.inspect: Integer\npub type resource Rd<T>\npub open: () -> Rd<a>\npub function: () -> ((() -> Integer) * ())\nimpl Rd<a>: Probe { inspect: (self: Self) -> Integer }";
    let mut host = Declarations::new();
    host.module("csv", declarations).unwrap();
    let mut session = fipe::session::Session::new(host);
    let mut machine = Machine::new(None);
    let setup=session_program(&mut session,"use csv\nkeep: csv.Rd<()> -> csv.Rd<()> = x => x\nmake: csv.Probe -> (() -> Integer) = p => (_ => p.inspect)");
    assert!(matches!(machine.run(&setup), Outcome::Done(Ok(_))));
    let root = setup
        .tables
        .borrow()
        .meanings
        .iter()
        .position(|meaning| meaning.label == "Rd")
        .unwrap();
    assert!(matches!(
        session.line(
            "use tags\nentry: csv.Rd<tags.Mark> = csv.open ()\nmake entry"
        ),
        Outcome::Request(_)
    ));
    let Outcome::Done(report) =
        session.reply(Reply::File(Some("pub type Mark {}".into())))
    else {
        panic!("checked resource argument")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let acquisition = compile::compile(report.unit.as_ref().unwrap());
    assert!(matches!(machine.run(&acquisition), Outcome::Request(_)));
    let reference = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    let Outcome::Done(Ok(function @ Value::Function(_))) =
        machine.reply(&acquisition, Reply::Value(Ok(reference)))
    else {
        panic!("capturing function")
    };
    // Its body was defined in setup, so withdrawing acquisition leaves the body
    // valid while retiring only the resource's ordinary type argument.
    let withdrawn = session.withdraw(report.accepted.unwrap()).unwrap();
    machine.withdraw(&withdrawn);
    assert_eq!(setup.tables.borrow().meanings[root].label, "Rd");
    assert!(!setup
        .tables
        .borrow()
        .meanings
        .iter()
        .any(|meaning| meaning.label == "Mark"));
    let replay =
        session_program(&mut session, "(thunk, _) = csv.function ()\nthunk ()");
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.run(&replay)
    else {
        panic!("function delivery")
    };
    assert_eq!(
        replay.tables.borrow().declarations[declaration].label,
        "function"
    );
    // A typed Function may carry the old alias; actual member dispatch must then
    // refuse its retired argument before requesting inspect from the Host.
    assert!(
        matches!(machine.reply(&replay,Reply::Value(Ok(Value::Tuple(Rc::new(vec![function,Value::Unit]))))),Outcome::Done(Err(fipe::vm::Fault{reason:fipe::value::Reason::Host(ref message),..})) if message.contains("retired or different context"))
    );
}

const RESOURCE_EQ:&str="pub type resource Rd<T>\npub integer: Rd<Integer>\npub text: Rd<String>\npub unit: Rd<()>\npub open: () -> Rd<a>\npub function: () -> ((() -> Bool) * ())\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }";
#[test]
fn resource_packed_eq_calls_same_type_row_once_and_skips_different_arguments() {
    for same in [false, true] {
        let right = if same { "integer" } else { "text" };
        let program = program(
            RESOURCE_EQ,
            &format!(
                "use csv\nx: Eq = csv.integer\ny: Eq = csv.{right}\nx == y"
            ),
        );
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let left = ResourceDomain::new().issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"left"[..]),
        );
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(left.clone()))),
            Outcome::Request(_)
        ));
        let right = ResourceDomain::new().issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"right"[..]),
        );
        let result = machine.reply(&program, Reply::Value(Ok(right.clone())));
        let prelude = fipe::value::Prelude::new(&program.tables.borrow());
        if same {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = result
            else {
                panic!("supplied equality")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "eq"
            );
            assert_eq!(arguments,vec![right,left],"the declared other/self order and exact issued aliases must survive");
            assert!(
                matches!(machine.reply(&program,Reply::Value(Ok(prelude.bool(true)))),Outcome::Done(Ok(value)) if value==prelude.bool(true))
            );
        } else {
            assert!(
                matches!(&result,Outcome::Done(Ok(value)) if *value==prelude.bool(false)),
                "{result:?}"
            );
        }
    }
}
#[test]
fn resource_packed_eq_rejects_retired_right_argument_before_row_effect() {
    let mut host = Declarations::new();
    host.module("csv", RESOURCE_EQ).unwrap();
    let mut session = fipe::session::Session::new(host);
    let mut machine = Machine::new(None);
    let setup=session_program(&mut session,"use csv\nkeep: csv.Rd<()> -> csv.Rd<()> = x => x\nmake: Eq -> Eq -> (() -> Bool) = a b => (_ => a == b)");
    assert!(matches!(machine.run(&setup), Outcome::Done(Ok(_))));
    assert!(matches!(session.line("use tags\nleft=csv.unit\nright: csv.Rd<tags.Mark> = csv.open ()\nmake left right"),Outcome::Request(_)));
    let Outcome::Done(report) =
        session.reply(Reply::File(Some("pub type Mark {}".into())))
    else {
        panic!("checked acquisition")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let acquisition = compile::compile(report.unit.as_ref().unwrap());
    assert!(matches!(machine.run(&acquisition), Outcome::Request(_)));
    let domain = ResourceDomain::new();
    let left_type = machine.pending_resource_type().unwrap();
    let left = domain.issue(&left_type, Rc::from(&b"left"[..]));
    assert!(matches!(
        machine.reply(&acquisition, Reply::Value(Ok(left.clone()))),
        Outcome::Request(_)
    ));
    let right = domain.issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"right"[..]),
    );
    let Outcome::Done(Ok(function @ Value::Function(_))) =
        machine.reply(&acquisition, Reply::Value(Ok(right)))
    else {
        panic!("captured comparison")
    };
    let withdrawn = session.withdraw(report.accepted.unwrap()).unwrap();
    machine.withdraw(&withdrawn);
    assert!(!setup
        .tables
        .borrow()
        .meanings
        .iter()
        .any(|meaning| meaning.label == "Mark"));
    // Reuse the original live entry, not a newly issued replacement. This would
    // fail if withdrawal accidentally retired the LEFT/root stamp as well.
    let healthy = session_program(&mut session, "csv.unit");
    assert!(matches!(machine.run(&healthy), Outcome::Request(_)));
    assert_eq!(left_type, machine.pending_resource_type().unwrap());
    assert!(
        matches!(machine.reply(&healthy,Reply::Value(Ok(left.clone()))),Outcome::Done(Ok(value)) if value==left)
    );
    let replay =
        session_program(&mut session, "(thunk, _) = csv.function ()\nthunk ()");
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.run(&replay)
    else {
        panic!("function delivery")
    };
    assert_eq!(
        replay.tables.borrow().declarations[declaration].label,
        "function"
    );
    let outcome = machine.reply(
        &replay,
        Reply::Value(Ok(Value::Tuple(Rc::new(vec![function, Value::Unit])))),
    );
    assert!(
        matches!(&outcome,Outcome::Done(Err(fipe::vm::Fault{reason:fipe::value::Reason::Host(message),..})) if message.contains("retired or different context")),
        "{outcome:?}"
    );
}
#[test]
fn resource_free_unfixed_session_result_rejects_foreign_carrier_but_admits_data(
) {
    let producer = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut issuing = Machine::new(None);
    assert!(matches!(issuing.run(&producer), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &issuing.pending_resource_type().unwrap(),
        Rc::from(&b"foreign-entry"[..]),
    );
    assert!(issuing.abandon());
    for (value, valid) in [(Value::Int(7), true), (alias, false)] {
        let mut host = Declarations::new();
        host.commands("pub unknown: a").unwrap();
        let mut session = fipe::session::Session::new(host);
        let consumer = session_program(&mut session, "[x]=[unknown]\nx");
        assert!(!consumer.tables.borrow().has_resources);
        let mut actual_unfixed_host_result = false;
        for body in &consumer.bodies {
            for block in &body.blocks {
                if let fipe::mir::Terminator::Call {
                    callee: fipe::mir::Callee::Host(_),
                    dest,
                    ..
                } = &block.terminator
                {
                    if let fipe::ir::Type::Session(slot) = body.locals[*dest].ty
                    {
                        assert!(consumer.tables.borrow().session_meanings
                            [slot]
                            .is_none());
                        actual_unfixed_host_result = true;
                    }
                }
            }
        }
        assert!(
            actual_unfixed_host_result,
            "fixture must exercise the unresolved Session admission branch"
        );
        let mut machine = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.run(&consumer)
        else {
            panic!("unknown result request")
        };
        assert_eq!(
            consumer.tables.borrow().declarations[declaration].label,
            "unknown"
        );
        let Outcome::Done(result) =
            machine.reply(&consumer, Reply::Value(Ok(value)))
        else {
            panic!("consumer terminal")
        };
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(result, Ok(Value::Int(7)));
        } else {
            assert!(matches!(
                result,
                Err(fipe::vm::Fault {
                    reason: fipe::value::Reason::Host(_),
                    ..
                })
            ));
        }
    }
}

fn next_protocol_stop(machine: &mut Machine) -> fipe::vm::Stop {
    loop {
        match machine.next_stop() {
            fipe::vm::Stop::Before(_) | fipe::vm::Stop::After(_, _) => {}
            stop => return stop,
        }
    }
}
fn resource_initial_reply_charge(
    calibration: &mut Machine,
    program: &fipe::mir::Program,
) -> u64 {
    calibration.begin(program);
    assert!(matches!(
        next_protocol_stop(calibration),
        fipe::vm::Stop::Request(_)
    ));
    assert!(calibration.abandon());
    calibration.begin(program);
    assert!(matches!(
        next_protocol_stop(calibration),
        fipe::vm::Stop::Request(_)
    ));
    let before = calibration.charged();
    assert!(calibration.abandon());
    before
}

#[test]
fn resource_stepped_reply_admits_exact_alias_and_refuses_before_later_effect() {
    let program = program(
        "pub type resource Table\npub open: Table\npub after: Table -> Integer",
        "use csv\nx=csv.open\ncsv.after x",
    );
    let mut calibration = Machine::new(None);
    let before = resource_initial_reply_charge(&mut calibration, &program);
    let foreign = self::program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let foreign_type =
        ResourceType::registered(&foreign.tables.borrow(), "csv", "Table")
            .unwrap();
    for case in ["healthy", "wrong-type", "scalar", "budget"] {
        let allowance = if case == "budget" {
            // Earlier cases may stamp ordinary result meanings. Measure the
            // current identity-cache state, not the earlier Table-only state.
            let mut current = Machine::new(None);
            current.begin(&program);
            let fipe::vm::Stop::Request(fipe::embed::Request::Call {
                declaration,
                ..
            }) = next_protocol_stop(&mut current)
            else {
                panic!("current-state open calibration")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "open"
            );
            let current_before = current.charged();
            assert!(current_before >= before);
            assert!(current.abandon());
            Some(current_before + 1)
        } else {
            None
        };
        let mut machine = Machine::new(allowance);
        machine.begin(&program);
        let fipe::vm::Stop::Request(fipe::embed::Request::Call {
            declaration,
            ..
        }) = next_protocol_stop(&mut machine)
        else {
            panic!("open request")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "open"
        );
        let ty = machine.pending_resource_type().unwrap();
        let value = match case {
            "wrong-type" => ResourceDomain::new()
                .issue(&foreign_type, Rc::from(&b"foreign"[..])),
            "scalar" => Value::Int(4),
            _ => ResourceDomain::new().issue(&ty, Rc::from(&b"entry"[..])),
        };
        let original = value.clone();
        machine.answer(Reply::Value(Ok(value)));
        let raw = machine.next_stop();
        if case == "healthy" {
            assert!(
                matches!(raw,fipe::vm::Stop::After(_,ref value) if *value==original)
            );
            let fipe::vm::Stop::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = next_protocol_stop(&mut machine)
            else {
                panic!("healthy sentinel")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments, [original]);
            machine.answer(Reply::Value(Ok(Value::Int(77))));
            assert!(matches!(
                next_protocol_stop(&mut machine),
                fipe::vm::Stop::Done(Ok(Value::Int(77)))
            ));
        } else {
            let fipe::vm::Stop::Done(Err(fault)) = raw else {
                panic!("no later effect")
            };
            if case == "budget" {
                assert_eq!(fault.reason, fipe::value::Reason::Budget);
                assert_eq!(machine.charged(), allowance.unwrap());
            } else {
                assert!(matches!(fault.reason, fipe::value::Reason::Host(_)));
            }
        }
    }
}
fn resource_input_program() -> fipe::mir::Program {
    let mut host = Declarations::new();
    host.module("csv","pub type resource Table\npub tick: Integer\npub inspect: Table -> Integer").unwrap();
    let files = [(
        "boot".into(),
        parse(lex("use csv\npub start=csv.tick")).unwrap(),
    )];
    let module=parse(lex("use csv\nuse boot\nf: csv.Table -> Integer = x => { ignored=boot.start\n csv.inspect x }\nf")).unwrap();
    let program = compile::compile(
        &check::check(&module, &host, &files, check::Options::default())
            .unwrap(),
    );
    assert!(
        !program.inits.is_empty(),
        "the fixture must execute a real file init"
    );
    program
}
fn resource_input_allowance(program: &fipe::mir::Program) -> u64 {
    // The first tick capture can stamp Integer. Warm that known identity once,
    // then measure the same cache state used by the finite ordinary control.
    let charge_to_tick = || {
        let mut calibration = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = calibration.run(program)
        else {
            panic!("calibration init")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "tick"
        );
        let charged = calibration.charged();
        assert!(calibration.abandon());
        charged
    };
    charge_to_tick();
    let allowance = charge_to_tick();
    assert!(allowance > 0);
    let mut ordinary = Machine::new(Some(allowance));
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        ordinary.run(program)
    else {
        panic!("same finite budget must reach init without input admission")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "tick"
    );
    assert!(ordinary.abandon());
    allowance
}
#[test]
fn resource_direct_and_stepped_input_gate_module_init_and_preserve_alias() {
    let program = resource_input_program();
    let same =
        ResourceType::registered(&program.tables.borrow(), "csv", "Table")
            .unwrap();
    let other = resource_input_program();
    let foreign =
        ResourceType::registered(&other.tables.borrow(), "csv", "Table")
            .unwrap();
    let alias = ResourceDomain::new()
        .issue(&same, Rc::from(&b"original-live-entry"[..]));
    let allowance = resource_input_allowance(&program);
    for stepped in [false, true] {
        for case in ["healthy", "wrong-type", "scalar", "budget"] {
            let mut machine =
                Machine::new((case == "budget").then_some(allowance));
            let input = match case {
                "wrong-type" => ResourceDomain::new()
                    .issue(&foreign, Rc::from(&b"foreign"[..])),
                "scalar" => Value::Int(3),
                _ => alias.clone(),
            };
            let first = if stepped {
                machine.begin_with_input(&program, input).unwrap();
                match next_protocol_stop(&mut machine) {
                    fipe::vm::Stop::Request(request) => {
                        Outcome::Request(request)
                    }
                    fipe::vm::Stop::Done(result) => Outcome::Done(result),
                    _ => unreachable!(),
                }
            } else {
                machine.run_with_input(&program, input).unwrap()
            };
            if case != "healthy" {
                let Outcome::Done(Err(fault)) = first else {
                    panic!("input must refuse before init")
                };
                if case == "budget" {
                    assert_eq!(fault.reason, fipe::value::Reason::Budget);
                    assert_eq!(machine.charged(), allowance);
                } else {
                    assert!(matches!(
                        fault.reason,
                        fipe::value::Reason::Host(_)
                    ));
                }
                continue;
            }
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                ..
            }) = first
            else {
                panic!("healthy init")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "tick"
            );
            let next = if stepped {
                machine.answer(Reply::Value(Ok(Value::Int(1))));
                match next_protocol_stop(&mut machine) {
                    fipe::vm::Stop::Request(request) => {
                        Outcome::Request(request)
                    }
                    _ => panic!("input use"),
                }
            } else {
                machine.reply(&program, Reply::Value(Ok(Value::Int(1))))
            };
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = next
            else {
                panic!("input inspect")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "inspect"
            );
            assert_eq!(arguments.as_slice(), std::slice::from_ref(&alias));
            if stepped {
                machine.answer(Reply::Value(Ok(Value::Int(7))));
                assert!(matches!(
                    next_protocol_stop(&mut machine),
                    fipe::vm::Stop::Done(Ok(Value::Int(7)))
                ));
            } else {
                assert!(matches!(
                    machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                    Outcome::Done(Ok(Value::Int(7)))
                ));
            }
        }
    }
}
#[test]
fn resource_stepped_invalid_input_arity_preserves_original_pending_call() {
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut machine = Machine::new(None);
    machine.begin(&program);
    assert!(matches!(
        next_protocol_stop(&mut machine),
        fipe::vm::Stop::Request(_)
    ));
    let ty = machine.pending_resource_type().unwrap();
    assert!(machine.begin_with_input(&program, Value::Unit).is_err());
    assert_eq!(machine.pending_resource_type().unwrap(), ty);
    let alias = ResourceDomain::new().issue(&ty, Rc::from(&b"entry"[..]));
    machine.answer(Reply::Value(Ok(alias.clone())));
    assert!(
        matches!(next_protocol_stop(&mut machine),fipe::vm::Stop::Done(Ok(value)) if value==alias)
    );
}

fn resource_parameters(
    program: &fipe::mir::Program,
    label: &str,
) -> (usize, Vec<usize>) {
    let tables = program.tables.borrow();
    let index = tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == label)
        .unwrap();
    (index, tables.meanings[index].parameters.clone())
}
#[test]
fn resource_metadata_undo_preserves_old_alias_and_retires_only_removed_argument(
) {
    const SOURCE: &str = "use tags\nx: csv.Rd<tags.Mark> = csv.open ()\nx";
    let mut host = Declarations::new();
    host.module(
        "csv",
        "pub type resource Rd<T>\npub unit: Rd<()>\npub open: () -> Rd<a>",
    )
    .unwrap();
    let mut session = fipe::session::Session::new(host);
    let mut machine = Machine::new(None);
    let domain = ResourceDomain::new();
    let first = session_program(&mut session, "use csv\ncsv.unit");
    let (root, original_parameters) = resource_parameters(&first, "Rd");
    assert!(!original_parameters.is_empty());
    assert!(matches!(machine.run(&first), Outcome::Request(_)));
    let original_type = machine.pending_resource_type().unwrap();
    let original =
        domain.issue(&original_type, Rc::from(&b"retained-entry"[..]));
    assert!(
        matches!(machine.reply(&first,Reply::Value(Ok(original.clone()))),Outcome::Done(Ok(value)) if value==original)
    );
    assert!(matches!(session.line(SOURCE), Outcome::Request(_)));
    let Outcome::Done(report) =
        session.reply(Reply::File(Some("pub type Mark {}".into())))
    else {
        panic!("new argument")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let second = compile::compile(report.unit.as_ref().unwrap());
    let removed_index = {
        let tables = second.tables.borrow();
        assert_eq!(
            tables
                .meanings
                .iter()
                .position(|meaning| meaning.label == "Rd"),
            Some(root)
        );
        assert_ne!(
            tables.meanings[root].parameters, original_parameters,
            "must exercise a real surviving Meaning metadata rewrite"
        );
        tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Mark")
            .unwrap()
    };
    assert!(matches!(machine.run(&second), Outcome::Request(_)));
    let removed_type = machine.pending_resource_type().unwrap();
    let removed =
        domain.issue(&removed_type, Rc::from(&b"removed-argument-entry"[..]));
    assert!(machine.abandon());
    let withdrawn = session.withdraw(report.accepted.unwrap()).unwrap();
    machine.withdraw(&withdrawn);
    {
        let tables = first.tables.borrow();
        assert_eq!(tables.meanings[root].parameters, original_parameters);
        assert!(!tables
            .meanings
            .iter()
            .any(|meaning| meaning.label == "Mark"));
    }
    let healthy = session_program(&mut session, "csv.unit");
    assert!(matches!(machine.run(&healthy), Outcome::Request(_)));
    assert_eq!(machine.pending_resource_type().unwrap(), original_type);
    assert!(
        matches!(machine.reply(&healthy,Reply::Value(Ok(original.clone()))),Outcome::Done(Ok(value)) if value==original)
    );
    assert!(matches!(session.line(SOURCE), Outcome::Request(_)));
    let Outcome::Done(report) =
        session.reply(Reply::File(Some("pub type Mark {}".into())))
    else {
        panic!("reused argument")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let reused = compile::compile(report.unit.as_ref().unwrap());
    assert_eq!(
        reused
            .tables
            .borrow()
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Mark"),
        Some(removed_index)
    );
    assert!(matches!(machine.run(&reused), Outcome::Request(_)));
    assert_ne!(machine.pending_resource_type().unwrap(), removed_type);
    assert!(matches!(
        machine.reply(&reused, Reply::Value(Ok(removed))),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Host(_),
            ..
        }))
    ));
}

#[test]
fn resource_named_packed_echo_admits_the_checked_alias_without_opaque_bypass() {
    let declarations="pub type Key: Eq\npub type resource Rd<T>\npub open: Rd<()>\npub echo: Key -> Key\nimpl Rd<a>: Key { eq = other self => True }";
    let program = program(declarations, "use csv\ncsv.echo csv.open");
    let mut boundary = false;
    for body in &program.bodies {
        for block in &body.blocks {
            if let fipe::mir::Terminator::Call {
                callee: fipe::mir::Callee::Host(declaration),
                dest,
                ..
            } = &block.terminator
            {
                let tables = program.tables.borrow();
                if tables.declarations[*declaration].label == "echo" {
                    let fipe::ir::Type::Meaning { index, .. } =
                        &body.locals[*dest].ty
                    else {
                        panic!("named packed boundary must lower to Meaning")
                    };
                    let meaning = &tables.meanings[*index];
                    assert_eq!(meaning.label, "Key");
                    assert!(!meaning.resource);
                    assert!(meaning.constructors.is_empty());
                    boundary = true;
                }
            }
        }
    }
    assert!(
        boundary,
        "must prove actual echo destination, not cached declaration metadata"
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(alias.clone())))
    else {
        panic!("typed packed echo")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "echo"
    );
    let [value @ Value::Packed(packed)] = arguments.as_slice() else {
        panic!("checker-produced packed input")
    };
    assert_eq!(packed.value(), &alias);
    assert!(machine.pending_resource_type().is_err());
    let echoed = machine.reply(&program, Reply::Value(Ok(value.clone())));
    assert!(
        matches!(&echoed,Outcome::Done(Ok(returned)) if returned==value),
        "{echoed:?}"
    );
}

#[test]
fn resource_declared_eq_and_nested_show_use_host_once_with_frozen_scalar_results(
) {
    let declarations="pub type resource Rd<T>\npub first: Rd<()>\npub second: Rd<()>\npub after: String -> String\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }\nimpl Rd<a>: Show { show: (self: Self) -> String }";
    let equality = program(
        declarations,
        "use csv\nx: Eq=csv.first\ny: Eq=csv.second\nx==y",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&equality), Outcome::Request(_)));
    let ty = machine.pending_resource_type().unwrap();
    let domain = ResourceDomain::new();
    let left = domain.issue(&ty, Rc::from(&b"left"[..]));
    let right = domain.issue(&ty, Rc::from(&b"right"[..]));
    assert!(matches!(
        machine.reply(&equality, Reply::Value(Ok(left.clone()))),
        Outcome::Request(_)
    ));
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&equality, Reply::Value(Ok(right.clone())))
    else {
        panic!("declared equality")
    };
    assert_eq!(
        equality.tables.borrow().declarations[declaration].label,
        "eq"
    );
    assert_eq!(arguments, [right, left]);
    let truth = fipe::value::Prelude::new(&equality.tables.borrow()).bool(true);
    assert!(
        matches!(machine.reply(&equality,Reply::Value(Ok(truth.clone()))),Outcome::Done(Ok(value)) if value==truth)
    );
    for wrong in [false, true] {
        let shown = program(
            declarations,
            "use csv\nx=csv.first\ns=show [x]\ncsv.after s",
        );
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&shown), Outcome::Request(_)));
        let alias = domain.issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"shown"[..]),
        );
        let Outcome::Request(fipe::embed::Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.reply(&shown, Reply::Value(Ok(alias.clone())))
        else {
            panic!("declared show")
        };
        assert_eq!(
            shown.tables.borrow().declarations[declaration].label,
            "show"
        );
        assert_eq!(arguments.as_slice(), std::slice::from_ref(&alias));
        let outcome = machine.reply(
            &shown,
            Reply::Value(Ok(if wrong {
                alias
            } else {
                Value::text("visible")
            })),
        );
        if wrong {
            assert!(matches!(
                outcome,
                Outcome::Done(Err(fipe::vm::Fault {
                    reason: fipe::value::Reason::Host(_),
                    ..
                }))
            ));
        } else {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("after show")
            };
            assert_eq!(
                shown.tables.borrow().declarations[declaration].label,
                "after"
            );
            assert_eq!(arguments, [Value::text("[visible]")]);
            assert!(
                matches!(machine.reply(&shown,Reply::Value(Ok(Value::text("done")))),Outcome::Done(Ok(value)) if value==Value::text("done"))
            );
        }
    }
}
fn assert_generic_factory_result(
    program: &fipe::mir::Program,
    result: &Result<Value, fipe::vm::Fault>,
    aliases: &[(ResourceType, Value)],
    counts: (usize, usize, usize),
    wrong_read: bool,
) {
    let (defaults, zeros, reads) = counts;
    assert_eq!((defaults, zeros), (2, 2));
    assert_eq!(reads, if wrong_read { 1 } else { 2 });
    if wrong_read {
        assert!(matches!(
            result,
            Err(fipe::vm::Fault {
                reason: fipe::value::Reason::Host(_),
                ..
            })
        ));
    } else {
        let expected = Value::Tuple(Rc::new(vec![
            aliases[0].1.clone(),
            aliases[1].1.clone(),
            aliases[0].1.clone(),
            aliases[1].1.clone(),
            fipe::value::Prelude::new(&program.tables.borrow())
                .ok(aliases[0].1.clone()),
            fipe::value::Prelude::new(&program.tables.borrow())
                .ok(aliases[1].1.clone()),
        ]));
        assert_eq!(result, &Ok(expected));
    }
}
fn issue_generic_factory_alias(
    machine: &Machine,
    domain: &ResourceDomain,
    aliases: &mut Vec<(ResourceType, Value)>,
) -> Value {
    let index = aliases.len();
    let ty = machine.pending_resource_type().unwrap();
    let alias = domain.issue(
        &ty,
        Rc::from(if index == 0 {
            &b"first"[..]
        } else {
            &b"second"[..]
        }),
    );
    aliases.push((ty, alias.clone()));
    alias
}
fn generic_read_reply(
    program: &fipe::mir::Program,
    machine: &Machine,
    arguments: &[Value],
    aliases: &[(ResourceType, Value)],
    reads: &mut usize,
    read_case: &str,
) -> Value {
    assert_eq!(
        arguments,
        [Value::text(if *reads == 0 { "first" } else { "second" })]
    );
    assert_eq!(
        machine.pending_resource_type(),
        Err(ResourceError::NotResourceResult)
    );
    let alias = aliases[if read_case == "wrong-leaf" {
        1 - *reads
    } else {
        *reads
    }]
    .1
    .clone();
    *reads += 1;
    if read_case == "bare" {
        alias
    } else {
        fipe::value::Prelude::new(&program.tables.borrow()).ok(alias)
    }
}
#[test]
fn resource_declared_generic_leaf_defaults_zero_and_whole_read_results_ignore_first_use(
) {
    let declarations="pub type resource Rd<T>\npub integer: Rd<Integer>\npub text: Rd<String>\nimpl Rd<a>: Default { default: Self }\nimpl Rd<a>: Zero { zero: Self }\nimpl Rd<a>: Add { add: Self -> (self: Self) -> Self }\nimpl Rd<a>: Read { read: String -> Result<Self, ParseError> }";
    for reverse in [false, true] {
        for read_case in ["healthy", "bare", "wrong-leaf"] {
            let wrong_read = read_case != "healthy";
            let order = if reverse {
                ["String", "Integer"]
            } else {
                ["Integer", "String"]
            };
            let factory =
                |kind| if kind == "Integer" { "integer" } else { "text" };
            let source=format!("use csv\ntype Wrap<T> {{ value: csv.Rd<T> }}\na=csv.{}\nb=csv.{}\nd0:Wrap<{}> = Wrap\nd1:Wrap<{}> = Wrap\nz0:csv.Rd<{}> = [] | sum\nz1:csv.Rd<{}> = [] | sum\nr0:Result<csv.Rd<{}>,ParseError> = read \"first\"\nr1:Result<csv.Rd<{}>,ParseError> = read \"second\"\n(d0.value,d1.value,z0,z1,r0,r1)",factory(order[0]),factory(order[1]),order[0],order[1],order[0],order[1],order[0],order[1]);
            let program = program(declarations, &source);
            let domain = ResourceDomain::new();
            let mut aliases: Vec<(ResourceType, Value)> = Vec::new();
            let mut machine = Machine::new(None);
            let mut outcome = machine.run(&program);
            let (mut defaults, mut zeros, mut reads) = (0, 0, 0);
            loop {
                match outcome {
                    Outcome::Request(fipe::embed::Request::Call {
                        declaration,
                        arguments,
                        ..
                    }) => {
                        let label = program.tables.borrow().declarations
                            [declaration]
                            .label
                            .clone();
                        let value = match label.as_str() {
                            "integer" | "text" => {
                                assert!(arguments.is_empty());
                                let index = aliases.len();
                                assert_eq!(label, factory(order[index]));
                                issue_generic_factory_alias(
                                    &machine,
                                    &domain,
                                    &mut aliases,
                                )
                            }
                            "default" | "zero" => {
                                assert!(arguments.is_empty());
                                let count = if label == "default" {
                                    &mut defaults
                                } else {
                                    &mut zeros
                                };
                                assert_eq!(
                                    machine.pending_resource_type().unwrap(),
                                    aliases[*count].0
                                );
                                let alias = aliases[*count].1.clone();
                                *count += 1;
                                alias
                            }
                            "read" => generic_read_reply(
                                &program, &machine, &arguments, &aliases,
                                &mut reads, read_case,
                            ),
                            _ => panic!("unexpected callback {label}"),
                        };
                        outcome =
                            machine.reply(&program, Reply::Value(Ok(value)));
                    }
                    Outcome::Done(result) => {
                        assert_generic_factory_result(
                            &program,
                            &result,
                            &aliases,
                            (defaults, zeros, reads),
                            wrong_read,
                        );
                        break;
                    }
                    Outcome::Request(_) => panic!("no source I/O during VM"),
                }
            }
        }
    }
}

#[test]
fn resource_declared_comparison_walks_keep_fixed_results_in_count_and_sort() {
    let declarations="pub type resource Table\npub rows: Vec<Table>\nimpl Table: Eq { eq: Self -> (self: Self) -> Bool }\nimpl Table: Ord { cmp: Self -> (self: Self) -> Ordering }";
    for operation in ["count", "sort"] {
        let program =
            program(declarations, &format!("use csv\ncsv.rows | {operation}"));
        let ty =
            ResourceType::registered(&program.tables.borrow(), "csv", "Table")
                .unwrap();
        let domain = ResourceDomain::new();
        let a = domain.issue(&ty, Rc::from(&b"a"[..]));
        let b = if operation == "count" {
            a.clone()
        } else {
            domain.issue(&ty, Rc::from(&b"b"[..]))
        };
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let Outcome::Request(fipe::embed::Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.reply(
            &program,
            Reply::Value(Ok(Value::seq(vec![a.clone(), b.clone()]))),
        )
        else {
            panic!("nested comparison callback")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            if operation == "count" { "eq" } else { "cmp" }
        );
        assert_eq!(
            arguments,
            [a.clone(), b.clone()],
            "nested comparison preserves other/self operand order"
        );
        let prelude = fipe::value::Prelude::new(&program.tables.borrow());
        let reply = if operation == "count" {
            prelude.bool(true)
        } else {
            prelude.ordering(std::cmp::Ordering::Equal)
        };
        let Outcome::Done(Ok(Value::Seq(values))) =
            machine.reply(&program, Reply::Value(Ok(reply)))
        else {
            panic!("fixed result must fit, not enclosing sequence")
        };
        if operation == "sort" {
            assert_eq!(values.as_slice(), [a, b]);
        } else {
            assert_eq!(values.len(), 1);
            let Value::Record(record) = &values[0] else {
                panic!("count record")
            };
            assert!(record.fields.contains(&a));
            assert!(record.fields.contains(&Value::Int(2)));
        }
    }
}
#[test]
fn resource_declared_show_budget_prevents_callback_with_a_finite_positive_control(
) {
    let program=program("pub type resource Table\npub first: Table\nimpl Table: Show { show: (self: Self) -> String }","use csv\nx=csv.first\nshow [x]");
    let domain = ResourceDomain::new();
    let mut alias = None;
    let mut before = 0;
    // Warm sparse identity first, then measure the identical stable-cache path.
    for _ in 0..2 {
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let value = domain.issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"entry"[..]),
        );
        alias = Some(value.clone());
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.reply(&program, Reply::Value(Ok(value)))
        else {
            panic!("show calibration")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "show"
        );
        before = machine.charged();
        assert!(machine.abandon());
    }
    let alias = alias.unwrap();
    assert!(before > 0);
    for allowed in [true, false] {
        let allowance = if allowed { before } else { before - 1 };
        let mut machine = Machine::new(Some(allowance));
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let outcome = machine.reply(&program, Reply::Value(Ok(alias.clone())));
        if allowed {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                ..
            }) = outcome
            else {
                panic!("finite positive reaches show")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "show"
            );
            assert!(machine.abandon());
        } else {
            assert!(matches!(
                outcome,
                Outcome::Done(Err(fipe::vm::Fault {
                    reason: fipe::value::Reason::Budget,
                    ..
                }))
            ));
            assert_eq!(machine.charged(), allowance);
        }
    }
}

#[test]
fn resource_full_structural_arguments_distinguish_concrete_witnesses() {
    for (left, right) in [
        ("Integer * String", "String * Integer"),
        ("Integer -> String", "String -> Integer"),
        ("[Integer; 2]", "[Integer; 3]"),
        ("Vec<Integer>", "Vec<String>"),
        ("Integer | String", "Integer | Bool"),
    ] {
        for valid in [false, true] {
            let declarations = format!("pub type resource Rd<T>\npub first: Rd<{left}>\npub second: Rd<{right}>\npub after: Rd<{right}> -> Integer");
            let program = program(
                &declarations,
                "use csv\nx=csv.first\ny=csv.second\ncsv.after y",
            );
            let mut machine = Machine::new(None);
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                ..
            }) = machine.run(&program)
            else {
                panic!("first")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "first"
            );
            let first_type = machine.pending_resource_type().unwrap();
            let domain = ResourceDomain::new();
            let first = domain.issue(&first_type, Rc::from(&b"first"[..]));
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                ..
            }) = machine.reply(&program, Reply::Value(Ok(first.clone())))
            else {
                panic!("second")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "second"
            );
            let second_type = machine.pending_resource_type().unwrap();
            assert_ne!(first_type, second_type, "{left} versus {right}");
            let second = domain.issue(&second_type, Rc::from(&b"second"[..]));
            let outcome = machine.reply(
                &program,
                Reply::Value(Ok(if valid { second.clone() } else { first })),
            );
            if valid {
                let Outcome::Request(fipe::embed::Request::Call {
                    declaration,
                    arguments,
                    ..
                }) = outcome
                else {
                    panic!("after: {outcome:?}")
                };
                assert_eq!(
                    program.tables.borrow().declarations[declaration].label,
                    "after"
                );
                assert_eq!(arguments, vec![second]);
                assert!(matches!(
                    machine.reply(&program, Reply::Value(Ok(Value::Int(77)))),
                    Outcome::Done(Ok(Value::Int(77)))
                ));
            } else {
                assert!(
                    matches!(
                        outcome,
                        Outcome::Done(Err(fipe::vm::Fault {
                            reason: fipe::value::Reason::Host(_),
                            ..
                        }))
                    ),
                    "{outcome:?}"
                );
            }
        }
    }
}

#[test]
fn resource_packed_ord_does_not_inherit_binary_ordering() {
    let mut host = Declarations::new();
    host.module("csv", "pub type resource Rd<T>\npub integer: Rd<Integer>\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }\nimpl Rd<a>: Ord { cmp: Self -> (self: Self) -> Ordering }").unwrap();
    let source = "use csv\nx: Ord = csv.integer\ny: Ord = csv.integer\nx < y";
    let module = parse(lex(source)).expect("packed ordering source parses");
    let errors = check::check(&module, &host, &[], check::Options::default())
        .expect_err("packed Ord does not inherit binary Ord");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let check::Error::Type(fipe::infer::Error {
        span,
        reason: fipe::infer::Reason::MissingClaim { ty, claim },
    }) = &errors[0]
    else {
        panic!("expected the established packed Ord refusal: {errors:?}")
    };
    let ord = fipe::types::Type::Named {
        name: fipe::types::TypeName {
            origin: fipe::resolve::Origin::Prelude,
            name: "Ord".into(),
        },
        arguments: vec![],
    };
    assert_eq!(ty, &ord);
    assert_eq!(claim, &ord);
    assert_eq!(&source[span.start..span.end], "<");
}

#[test]
fn resource_wrapper_owned_equality_keeps_different_live_payload_types() {
    for expression in ["same x y", "(x, 0) == (y, 0)"] {
        wrapper_owned_equality_retains_aliases(expression);
    }
}

fn wrapper_owned_equality_retains_aliases(expression: &str) {
    let mut host = Declarations::new();
    host.module("csv", "pub type Key: Eq\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool }\npub type resource Rd<T>\npub integer: Rd<Integer>\npub text: Rd<String>\npub choose: () -> Bool\nimpl Rd<a>: Key { eq: Self -> (self: Self) -> Bool }").unwrap();
    let mut session = fipe::session::Session::new(host);
    let setup = session_program(
        &mut session,
        "use csv\nsame: Eq a => a -> a -> Bool = lhs rhs => lhs == rhs",
    );
    assert_generic_value_dispatch(&setup);
    let mut machine = Machine::new(None);
    assert!(matches!(
        machine.run(&setup),
        Outcome::Done(Ok(Value::Unit))
    ));
    let program = session_program(
        &mut session,
        &format!(
            "x: csv.Key = csv.integer\ny: csv.Key = csv.text\n{expression}"
        ),
    );
    let (key, expected) = {
        let tables = program.tables.borrow();
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        let target = tables.dispatch.iter().filter(|member| member.label == "eq").flat_map(|member| &member.rows).find_map(|row| {
        if matches!(row.receiver, fipe::ir::Type::Meaning { index, .. } if index == key) {
            if let fipe::ir::Implementation::Declared(declaration) = row.target { return Some(declaration); }
        }
        None
    }).expect("wrapper-owned supplied equality row");
        (key, target)
    };
    assert!(!program.tables.borrow().meanings[key].resource);
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.run(&program)
    else {
        panic!("integer request")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "integer"
    );
    let left = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"left"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.reply(&program, Reply::Value(Ok(left.clone())))
    else {
        panic!("text request")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "text"
    );
    let right = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"right"[..]),
    );
    let outcome = machine.reply(&program, Reply::Value(Ok(right.clone())));
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = outcome
    else {
        panic!("wrapper equality must not shortcut to false: {outcome:?}")
    };
    assert_eq!(declaration, expected);
    let [Value::Packed(other), Value::Packed(receiver)] = arguments.as_slice()
    else {
        panic!("wrapper row must retain both packs")
    };
    assert_eq!(other.value(), &right);
    assert_eq!(receiver.value(), &left);
    let (Value::Resource(actual_right), Value::Resource(expected_right)) =
        (other.value(), &right)
    else {
        panic!("right resource")
    };
    let (Value::Resource(actual_left), Value::Resource(expected_left)) =
        (receiver.value(), &left)
    else {
        panic!("left resource")
    };
    assert!(Rc::ptr_eq(actual_right, expected_right));
    assert!(Rc::ptr_eq(actual_left, expected_left));
    let yes = fipe::value::Prelude::new(&program.tables.borrow()).bool(true);
    assert!(
        matches!(machine.reply(&program, Reply::Value(Ok(yes.clone()))), Outcome::Done(Ok(value)) if value == yes)
    );
}

fn assert_generic_value_dispatch(setup: &fipe::mir::Program) {
    let mut weak = Vec::new();
    for body in &setup.bodies {
        for block in &body.blocks {
            if let fipe::mir::Terminator::Call {
                callee:
                    fipe::mir::Callee::Dispatch {
                        witness:
                            fipe::mir::Witness::Value(fipe::mir::Operand::Local(
                                local,
                            )),
                        ..
                    },
                ..
            } = &block.terminator
            {
                if let fipe::ir::Type::Parameter(parameter) =
                    body.locals[*local].ty
                {
                    weak.push(parameter);
                }
            }
        }
    }
    assert!(!weak.is_empty(), "must exercise an existing generic dispatch body, not a direct known Host call");
}

#[test]
fn resource_descriptive_source_and_target_cannot_authorize_a_member_effect() {
    use fipe::ir::Type;
    use fipe::value::Packed;
    let header = "pub type resource Rd<T>\npub give: Eq\npub after: () -> Bool\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }";
    let source = "use csv\nordinary: Integer -> Integer = x => x\nkeep: csv.Rd<()> -> csv.Rd<()> = x => x\ntype Wrap<T>: Eq { value: T }\nimpl Wrap<a>: Eq { eq = other self => csv.after () }\nx: Eq = csv.give\nx == x";
    let program = program(header, source);
    let nominal = |label: &str, arguments| {
        let tables = program.tables.borrow();
        let index = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == label)
            .unwrap();
        Type::Meaning { index, arguments }
    };
    let rd = nominal("Rd", vec![Type::Unit]);
    let eq = nominal("Eq", vec![]);
    let wrap = nominal("Wrap", vec![rd.clone()]);
    for (source, target, valid) in [
        (nominal("Integer", vec![]), eq.clone(), true),
        (rd.clone(), eq, false),
        (Type::Unit, rd, false),
        (Type::Unit, wrap, false),
        (
            Type::Meaning {
                index: usize::MAX,
                arguments: vec![],
            },
            nominal("Eq", vec![]),
            false,
        ),
        (
            Type::Unit,
            Type::Meaning {
                index: usize::MAX,
                arguments: vec![],
            },
            false,
        ),
    ] {
        let mut machine = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.run(&program)
        else {
            panic!("give request")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "give"
        );
        let description =
            Packed::from_data(source, target, &Value::Int(7)).unwrap();
        let outcome = machine.reply(
            &program,
            Reply::Value(Ok(Value::Packed(Rc::new(description)))),
        );
        if valid {
            let yes =
                fipe::value::Prelude::new(&program.tables.borrow()).bool(true);
            assert!(
                matches!(&outcome, Outcome::Done(Ok(value)) if value == &yes),
                "{outcome:?}"
            );
        } else {
            assert!(matches!(&outcome, Outcome::Done(Err(_))), "a descriptive label must fail before eq/after callbacks: {outcome:?}");
        }
    }
}

#[test]
fn resource_data_constructor_rejects_hidden_aliases_without_taking_payload_ownership(
) {
    use fipe::value::Packed;
    let program = program(
        "pub type resource Table\npub open: Table",
        "use csv\ncsv.open",
    );
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let reference = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"entry"[..]),
    );
    for filled in [false, true] {
        let payload = Value::Function(Rc::new(fipe::value::Closure {
            body: usize::MAX,
            captures: if filled {
                vec![]
            } else {
                vec![reference.clone()]
            },
            meanings: vec![],
            filled: if filled {
                vec![reference.clone()]
            } else {
                vec![]
            },
            guards: vec![],
        }));
        let mut source = fipe::ir::Type::Unit;
        let mut target = fipe::ir::Type::Unit;
        for _ in 0..4096 {
            source = fipe::ir::Type::Not(Box::new(source));
            target = fipe::ir::Type::Not(Box::new(target));
        }
        assert!(Packed::from_data(source, target, &payload).is_none());
        let Value::Function(retained) = payload else {
            unreachable!()
        };
        assert_eq!(
            if filled {
                &retained.filled[0]
            } else {
                &retained.captures[0]
            },
            &reference
        );
    }
    assert!(Packed::from_data(
        fipe::ir::Type::Unit,
        fipe::ir::Type::Unit,
        &Value::Int(7)
    )
    .is_some());
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(reference))),
        Outcome::Done(Ok(Value::Resource(_)))
    ));
}

#[test]
fn resource_packed_result_only_alias_uses_the_pending_snapshot() {
    let header = "pub type Key: Eq\npub type resource Rd<T>\npub open: Rd<()>\npub echo: Key -> Key\npub saved: Key\nimpl Rd<a>: Key { eq: Self -> (self: Self) -> Bool }";
    let program = program(header, "use csv\ntype Box { value: Integer }\nkeep: Box -> Box = x => x\n(csv.echo csv.open, csv.saved)");
    let mut machine = Machine::new(None);
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.run(&program)
    else {
        panic!("open")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "open"
    );
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"saved"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(alias.clone())))
    else {
        panic!("echo")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "echo"
    );
    let [value @ Value::Packed(packed)] = arguments.as_slice() else {
        panic!("checked association")
    };
    assert_eq!(packed.value(), &alias);
    let packed = value.clone();
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.reply(&program, Reply::Value(Ok(packed.clone())))
    else {
        panic!("saved")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "saved"
    );
    assert!(
        machine.pending_resource_type().is_err(),
        "result-only Key is not a direct resource witness"
    );
    {
        let mut tables = program.tables.borrow_mut();
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        let boxed = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Box")
            .unwrap();
        assert!(tables.meanings[key].constructors.is_empty());
        tables.meanings[key].constructors =
            tables.meanings[boxed].constructors.clone();
    }
    let outcome = machine.reply(&program, Reply::Value(Ok(packed.clone())));
    let Outcome::Done(Ok(Value::Tuple(parts))) = outcome else {
        panic!("frozen result-only association: {outcome:?}")
    };
    assert_eq!(parts.as_slice(), [packed.clone(), packed]);
}

#[test]
fn resource_packed_budget_stops_before_echo_and_before_post_reply_effect() {
    let header = "pub type Key: Eq\npub type resource Rd<T>\npub open: Rd<()>\npub echo: Key -> Key\npub after: Key -> Integer\nimpl Rd<a>: Key { eq: Self -> (self: Self) -> Bool }";
    let program = program(header, "use csv\ncsv.after (csv.echo csv.open)");
    let domain = ResourceDomain::new();
    let mut reference = Value::Unit;
    let mut allowance = 0;
    for _ in 0..2 {
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        reference = domain.issue(
            &machine.pending_resource_type().unwrap(),
            Rc::from(&b"entry"[..]),
        );
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.reply(&program, Reply::Value(Ok(reference.clone())))
        else {
            panic!("calibration echo")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "echo"
        );
        allowance = machine.charged();
        assert!(machine.abandon());
    }
    assert!(allowance > 1);
    for finite in [allowance - 1, allowance, allowance + 1] {
        let mut machine = Machine::new(Some(finite));
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let outcome =
            machine.reply(&program, Reply::Value(Ok(reference.clone())));
        if finite < allowance {
            assert!(matches!(
                outcome,
                Outcome::Done(Err(fipe::vm::Fault {
                    reason: fipe::value::Reason::Budget,
                    ..
                }))
            ));
        } else {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("finite positive echo: {outcome:?}")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "echo"
            );
            let [packed @ Value::Packed(_)] = arguments.as_slice() else {
                panic!("actual pack")
            };
            assert!(
                matches!(
                    machine.reply(&program, Reply::Value(Ok(packed.clone()))),
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: fipe::value::Reason::Budget,
                        ..
                    }))
                ),
                "no after callback may occur"
            );
        }
        assert_eq!(machine.charged(), finite);
    }
}

#[test]
fn resource_derived_record_and_structural_array_packs_preserve_full_sources() {
    let header = "pub type resource Rd<T>\npub open: Rd<()>\npub echo: Eq -> Eq\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }";
    for (source, record) in [
        ("use csv\ntype Pair<T: Eq>: Eq { value: T }\ncsv.echo (Pair csv.open)", true),
        ("use csv\ncsv.echo [csv.open]", false),
    ] {
        let program = program(header, source);
        let mut machine = Machine::new(None);
        assert!(matches!(machine.run(&program), Outcome::Request(_)));
        let alias = ResourceDomain::new().issue(&machine.pending_resource_type().unwrap(), Rc::from(&b"entry"[..]));
        let Outcome::Request(fipe::embed::Request::Call { declaration, arguments, .. }) = machine.reply(&program, Reply::Value(Ok(alias.clone()))) else { panic!("echo") };
        assert_eq!(program.tables.borrow().declarations[declaration].label, "echo");
        let [value @ Value::Packed(packed)] = arguments.as_slice() else { panic!("actual checked pack") };
        let fipe::ir::Type::Meaning { index, arguments } = packed.target_type() else { panic!("Eq target") };
        assert_eq!(program.tables.borrow().meanings[*index].label, "Eq");
        assert!(arguments.is_empty());
        let inner = if record {
            let fipe::ir::Type::Meaning { index, arguments } = packed.source_type() else { panic!("full Pair source") };
            assert_eq!(program.tables.borrow().meanings[*index].label, "Pair");
            assert_eq!(arguments.len(), 1);
            let Value::Record(payload) = packed.value() else { panic!("record payload") };
            assert_eq!(payload.fields.len(), 1);
            &payload.fields[0]
        } else {
            assert!(matches!(packed.source_type(), fipe::ir::Type::Array { .. }));
            let Value::Seq(payload) = packed.value() else { panic!("array payload") };
            assert_eq!(payload.len(), 1);
            &payload[0]
        };
        let (Value::Resource(actual), Value::Resource(original)) = (inner, &alias) else { panic!("genuine nested alias") };
        assert!(Rc::ptr_eq(actual, original));
        let outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
        assert!(matches!(&outcome, Outcome::Done(Ok(returned)) if returned == value), "{outcome:?}");
    }
}

#[test]
fn resource_packed_phantom_indices_are_checked_before_static_wrapper_priority()
{
    use fipe::{ir::Type, value::Packed};
    let header = "pub type Key<T>: Eq\npub give: Key<Integer>\nimpl Key<a>: Eq { eq: Self -> (self: Self) -> Bool }";
    let program = program(header, "use csv\nsame: Eq a => a -> a -> Bool = lhs rhs => lhs == rhs\nx = csv.give\nsame x x");
    let (key, integer, expected_eq) = {
        let tables = program.tables.borrow();
        let index = |label| {
            tables
                .meanings
                .iter()
                .position(|meaning| meaning.label == label)
                .unwrap()
        };
        let key = index("Key");
        let expected = tables.dispatch.iter().filter(|member| member.label == "eq").flat_map(|member| &member.rows).find_map(|row| {
            if matches!(row.receiver, Type::Meaning { index, .. } if index == key) {
                if let fipe::ir::Implementation::Declared(declaration) = row.target { return Some(declaration); }
            }
            None
        }).expect("Key's supplied eq row");
        (key, index("Integer"), expected)
    };
    let integer = Type::Meaning {
        index: integer,
        arguments: vec![],
    };
    let good_key = Type::Meaning {
        index: key,
        arguments: vec![integer.clone()],
    };
    let bad_key = Type::Meaning {
        index: key,
        arguments: vec![Type::Meaning {
            index: usize::MAX,
            arguments: vec![],
        }],
    };
    for (source, target, valid) in [
        (integer.clone(), good_key.clone(), true),
        (bad_key.clone(), good_key, false),
        (integer, bad_key, false),
    ] {
        let mut machine = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.run(&program)
        else {
            panic!("give")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "give"
        );
        let value = Value::Packed(Rc::new(
            Packed::from_data(source, target, &Value::Int(7)).unwrap(),
        ));
        let outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
        if valid {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("lawful own eq: {outcome:?}")
            };
            assert_eq!(declaration, expected_eq);
            assert_eq!(arguments, vec![value.clone(), value]);
            let yes =
                fipe::value::Prelude::new(&program.tables.borrow()).bool(true);
            assert!(
                matches!(machine.reply(&program, Reply::Value(Ok(yes.clone()))), Outcome::Done(Ok(returned)) if returned == yes)
            );
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(_))),
                "invalid phantom must refuse before own eq: {outcome:?}"
            );
        }
    }
}

#[test]
fn resource_packed_closed_field_contract_is_captured_before_request() {
    let header = "pub type Key: Eq\npub type Holder { pub item: Key }\npub type resource Rd<T>\npub open: Rd<()>\npub echo: Holder -> Holder\nimpl Rd<a>: Key { eq: Self -> (self: Self) -> Bool }";
    let source = "use csv\ntype Box { value: Integer }\nkeep: Box -> Box = x => x\ncsv.echo (csv.Holder csv.open)";
    let program = program(header, source);
    let mut machine = Machine::new(None);
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        machine.run(&program)
    else {
        panic!("open")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "open"
    );
    let alias = ResourceDomain::new().issue(
        &machine.pending_resource_type().unwrap(),
        Rc::from(&b"closed-field"[..]),
    );
    let Outcome::Request(fipe::embed::Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.reply(&program, Reply::Value(Ok(alias.clone())))
    else {
        panic!("Holder echo")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "echo"
    );
    let [value @ Value::Record(record)] = arguments.as_slice() else {
        panic!("checker-created Holder")
    };
    let [Value::Packed(packed)] = record.fields.as_slice() else {
        panic!("closed Key field must retain checked pack")
    };
    let (Value::Resource(actual), Value::Resource(original)) =
        (packed.value(), &alias)
    else {
        panic!("genuine alias")
    };
    assert!(Rc::ptr_eq(actual, original));
    {
        let mut tables = program.tables.borrow_mut();
        let holder = &tables.meanings[record.meaning];
        assert_eq!(holder.label, "Holder");
        assert!(
            holder.parameters.is_empty(),
            "Key must come from the closed field, not root arguments"
        );
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        let boxed = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Box")
            .unwrap();
        assert_eq!(
            packed.target_type(),
            &fipe::ir::Type::Meaning {
                index: key,
                arguments: vec![]
            }
        );
        assert!(tables.meanings[key].constructors.is_empty());
        tables.meanings[key].constructors =
            tables.meanings[boxed].constructors.clone();
    }
    let outcome = machine.reply(&program, Reply::Value(Ok(value.clone())));
    assert!(
        matches!(&outcome, Outcome::Done(Ok(returned)) if returned == value),
        "closed field association must use the request snapshot: {outcome:?}"
    );
}

#[test]
fn resource_packed_invalid_other_index_is_rejected_before_retained_own_eq() {
    use fipe::{ir::Type, value::Packed};
    let header = "pub type Key<T>: Eq\npub left: Key<Integer>\npub right: Key<Integer>\nimpl Key<a>: Eq { eq: Self -> (self: Self) -> Bool }";
    let program =
        program(header, "use csv\nsame: Eq a => a -> a -> Bool = lhs rhs => lhs == rhs\nx = csv.left\ny = csv.right\nsame x y");
    let (integer, key, expected_eq) = {
        let tables = program.tables.borrow();
        let key = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Key")
            .unwrap();
        let integer = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Integer")
            .unwrap();
        let eq = tables.dispatch.iter().filter(|member| member.label == "eq").flat_map(|member| &member.rows).find_map(|row| {
            if matches!(row.receiver, Type::Meaning { index, .. } if index == key) {
                if let fipe::ir::Implementation::Declared(declaration) = row.target { return Some(declaration); }
            }
            None
        }).unwrap();
        (
            Type::Meaning {
                index: integer,
                arguments: vec![],
            },
            key,
            eq,
        )
    };
    let good = Type::Meaning {
        index: key,
        arguments: vec![integer.clone()],
    };
    let bad = Type::Meaning {
        index: key,
        arguments: vec![Type::Meaning {
            index: usize::MAX,
            arguments: vec![],
        }],
    };
    for (source, target, valid) in [
        (integer.clone(), good.clone(), true),
        (bad.clone(), good.clone(), false),
        (integer.clone(), bad, false),
    ] {
        let mut machine = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.run(&program)
        else {
            panic!("left")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "left"
        );
        let left = Value::Packed(Rc::new(
            Packed::from_data(integer.clone(), good.clone(), &Value::Int(1))
                .unwrap(),
        ));
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.reply(&program, Reply::Value(Ok(left.clone())))
        else {
            panic!("right")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "right"
        );
        let right = Value::Packed(Rc::new(
            Packed::from_data(source, target, &Value::Int(2)).unwrap(),
        ));
        let outcome = machine.reply(&program, Reply::Value(Ok(right.clone())));
        if valid {
            let Outcome::Request(fipe::embed::Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("retained own eq: {outcome:?}")
            };
            assert_eq!(declaration, expected_eq);
            assert_eq!(arguments, vec![right, left]);
            let yes =
                fipe::value::Prelude::new(&program.tables.borrow()).bool(true);
            assert!(
                matches!(machine.reply(&program, Reply::Value(Ok(yes.clone()))), Outcome::Done(Ok(value)) if value == yes)
            );
        } else {
            assert!(matches!(outcome, Outcome::Done(Err(_))), "invalid OTHER metadata must not be passed to own eq: {outcome:?}");
        }
    }
}

#[test]
fn resource_packed_opened_eq_checks_other_source_before_unequal_shortcut() {
    use fipe::{ir::Type, value::Packed};
    let header = "pub type resource Rd<T>\npub left: Eq\npub right: Eq\nimpl Rd<a>: Eq { eq: Self -> (self: Self) -> Bool }";
    let source = "use csv\nkeep: csv.Rd<()> -> csv.Rd<()> = x => x\ntext: String -> String = x => x\ninteger: Integer -> Integer = x => x\nx = csv.left\ny = csv.right\nx == y";
    let program = program(header, source);
    let ty = |label: &str, arguments| {
        let tables = program.tables.borrow();
        Type::Meaning {
            index: tables
                .meanings
                .iter()
                .position(|meaning| meaning.label == label)
                .unwrap(),
            arguments,
        }
    };
    for valid in [true, false] {
        let mut machine = Machine::new(None);
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.run(&program)
        else {
            panic!("left")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "left"
        );
        let left = Value::Packed(Rc::new(
            Packed::from_data(
                ty("Integer", vec![]),
                ty("Eq", vec![]),
                &Value::Int(1),
            )
            .unwrap(),
        ));
        let Outcome::Request(fipe::embed::Request::Call {
            declaration, ..
        }) = machine.reply(&program, Reply::Value(Ok(left)))
        else {
            panic!("right")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "right"
        );
        let (source, payload) = if valid {
            (ty("String", vec![]), Value::text("different"))
        } else {
            (ty("Rd", vec![Type::Unit]), Value::Int(7))
        };
        let right = Value::Packed(Rc::new(
            Packed::from_data(source, ty("Eq", vec![]), &payload).unwrap(),
        ));
        let outcome = machine.reply(&program, Reply::Value(Ok(right)));
        if valid {
            let no =
                fipe::value::Prelude::new(&program.tables.borrow()).bool(false);
            assert!(
                matches!(&outcome, Outcome::Done(Ok(value)) if value == &no),
                "valid different sources keep ordinary false: {outcome:?}"
            );
        } else {
            assert!(matches!(outcome, Outcome::Done(Err(_))), "active fake Rd source must fail before false or supplied eq: {outcome:?}");
        }
    }
}

// Reuse that file's public program/ResourceDomain/Reply/Value/Rc imports.
fn ordinary_frozen_request_label(
    program: &fipe::mir::Program,
    outcome: &Outcome<Result<Value, fipe::vm::Fault>>,
    expected: &str,
) {
    let Outcome::Request(fipe::embed::Request::Call { declaration, .. }) =
        outcome
    else {
        panic!("a Host request, not {outcome:?}")
    };
    assert_eq!(
        program.tables.borrow().declarations[*declaration].label,
        expected
    );
}

fn ordinary_frozen_late_token(
    program: &fipe::mir::Program,
    label: &str,
) -> Value {
    {
        let mut tables = program.tables.borrow_mut();
        let token = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Token")
            .unwrap();
        assert!(!tables.meanings[token].resource);
        assert!(tables.meanings[token].constructors.is_empty());
        tables.meanings[token].resource = true;
        tables.has_resources = true;
    }
    // The SAME checked program now captures a resource result in a second Machine.
    let mut producer = Machine::new(None);
    ordinary_frozen_request_label(program, &producer.run(program), label);
    let witness = producer
        .pending_resource_type()
        .expect("new result is marked");
    let alias =
        ResourceDomain::new().issue(&witness, Rc::from(&b"late-token"[..]));
    assert!(producer.abandon());
    alias
}

#[test]
fn ordinary_frozen_opaque_keeps_its_data_policy_after_live_resource_marking() {
    for kind in 0..4 {
        let program =
            program("pub type Token\npub get: Token", "use csv\ncsv.get");
        assert!(!program.tables.borrow().has_resources);
        let mut consumer = Machine::new(None);
        ordinary_frozen_request_label(&program, &consumer.run(&program), "get");
        assert_eq!(
            consumer.pending_resource_type(),
            Err(ResourceError::NotResourceResult)
        );
        let alias = ordinary_frozen_late_token(&program, "get");
        assert_eq!(
            consumer.pending_resource_type(),
            Err(ResourceError::NotResourceResult)
        );
        let reply = match kind {
            0 => alias,
            1 | 2 => Value::Function(Rc::new(fipe::value::Closure {
                body: program.main,
                captures: if kind == 1 {
                    vec![alias.clone()]
                } else {
                    vec![]
                },
                meanings: vec![],
                filled: if kind == 2 { vec![alias] } else { vec![] },
                guards: vec![],
            })),
            _ => Value::Int(7),
        };
        let outcome = consumer.reply(&program, Reply::Value(Ok(reply)));
        if kind == 3 {
            assert_eq!(outcome, Outcome::Done(Ok(Value::Int(7))));
        } else {
            assert!(
                matches!(
                    outcome,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: fipe::value::Reason::Host(_),
                        ..
                    }))
                ),
                "opaque case {kind}: {outcome:?}"
            );
        }
    }
}

#[test]
fn ordinary_frozen_explicit_function_position_still_accepts_genuine_alias_capture(
) {
    let program=program(
        "pub type Token\npub token: Token\npub result: () -> ((() -> Integer) * ())",
        "use csv\nx=csv.token\ncsv.result ()",
    );
    assert!(!program.tables.borrow().has_resources);
    let mut consumer = Machine::new(None);
    ordinary_frozen_request_label(&program, &consumer.run(&program), "token");
    ordinary_frozen_request_label(
        &program,
        &consumer.reply(&program, Reply::Value(Ok(Value::Int(7)))),
        "result",
    );
    let alias = ordinary_frozen_late_token(&program, "token");
    let function = Value::Function(Rc::new(fipe::value::Closure {
        body: program.main,
        captures: vec![alias],
        meanings: vec![],
        filled: vec![],
        guards: vec![],
    }));
    let value = Value::Tuple(Rc::new(vec![function, Value::Unit]));
    assert_eq!(
        consumer.reply(&program, Reply::Value(Ok(value.clone()))),
        Outcome::Done(Ok(value))
    );
    // Function receipt is deliberately kind-opaque. Do not call this artificial
    // opaque callback or infer its body/capture signature from its receipt.
    // Existing retired-alias checked-function invocation regression stays unchanged.
}
