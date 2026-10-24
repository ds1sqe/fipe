//! Marked pipes and `&&` at run time: the five rows of the marked-pipe
//! rule on `Maybe`, `Result` and `Either`, and skipped sides.
use super::run::{charged, shown};

#[test]
fn a_same_family_return_is_used_directly_not_nested() {
    assert_eq!(shown("Just 1 |: (_ => Just 2)"), "Just 2");
    assert_eq!(shown("Just 1 |: (n => n + 1)"), "Just 2");
}

#[test]
fn the_inactive_side_keeps_its_value_and_the_stage_is_not_called() {
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    let ok = "r: Result<Integer, String> = Ok 1\n";
    assert_eq!(shown(&format!("{err}r |: (n => n + 1)")), "Err \"e\"");
    assert_eq!(shown(&format!("{ok}r |! (e => e ++ \"!\")")), "Ok 1");
    assert_eq!(shown(&format!("{err}r |: (_ => fault \"x\")")), "Err \"e\"");
    assert_eq!(
        shown("m: Maybe<Integer> = Nothing\nm |: (n => n + 1)"),
        "Nothing"
    );
}

#[test]
fn the_marked_side_is_rebuilt_from_the_stage() {
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    let ok = "r: Result<Integer, String> = Ok 1\n";
    assert_eq!(shown(&format!("{ok}r |: (n => n + 1)")), "Ok 2");
    assert_eq!(shown(&format!("{err}r |! (e => e ++ \"!\")")), "Err \"e!\"");
    assert_eq!(
        shown(&format!("{err}r |! (e => Err (e ++ \"!\"))")),
        "Err \"e!\""
    );
    assert_eq!(shown(&format!("{err}r |! (_ => Ok 0)")), "Ok 0");
    assert_eq!(shown("Err \"e\" |! (e => e ++ \"!\")"), "Err \"e!\"");
}

#[test]
fn maybes_bad_side_takes_a_value_and_discards_a_plain_one() {
    let nothing = "m: Maybe<Integer> = Nothing\n";
    assert_eq!(shown(&format!("{nothing}m |! Just 5")), "Just 5");
    assert_eq!(shown(&format!("{nothing}m |! 5")), "Nothing");
    assert_eq!(shown("Just 1 |! Just 5"), "Just 1");
}

#[test]
fn either_has_left_bad_and_right_good() {
    let left = "e: Either<Integer, String> = Left 1\n";
    let right = "e: Either<Integer, String> = Right \"a\"\n";
    assert_eq!(shown(&format!("{left}e |! (n => n + 1)")), "Left 2");
    assert_eq!(shown(&format!("{left}e |: (s => s ++ \"!\")")), "Left 1");
    assert_eq!(
        shown(&format!("{right}e |: (s => s ++ \"!\")")),
        "Right \"a!\""
    );
    assert_eq!(shown(&format!("{right}e |! (n => n + 1)")), "Right \"a\"");
    assert_eq!(
        shown(&format!("{left}e |! (_ => Right \"b\")")),
        "Right \"b\""
    );
}

#[test]
fn and_is_a_good_side_stage_that_ignores_the_payload() {
    assert_eq!(shown("Just 1 && Just 5"), "Just 5");
    assert_eq!(shown("Just 1 && 5"), "Just 5");
    assert_eq!(shown("m: Maybe<Integer> = Nothing\nm && Just 5"), "Nothing");
    assert_eq!(
        shown("r: Result<Integer, String> = Err \"e\"\nr && Ok 2"),
        "Err \"e\""
    );
    assert_eq!(shown("(True && False, True && True)"), "(False, True)");
    assert_eq!(shown("False && fault \"x\""), "False");
}

#[test]
fn a_skipped_side_charges_nothing() {
    let nothing = "m: Maybe<Integer> = Nothing\n";
    assert_eq!(
        charged(&format!("{nothing}m |: (n => n + 1 + 2 + 3 + 4)")),
        charged(&format!("{nothing}m |: (n => n)"))
    );
    assert_eq!(
        charged("False && (1 + 2 + 3 + 4 == 10)"),
        charged("False && True")
    );
    assert_eq!(charged("Just 1 || 1 + 2 + 3 + 4"), charged("Just 1 || 1"));
    assert!(charged("True && (1 + 2 + 3 + 4 == 10)") > charged("True && True"));
    assert!(
        charged("Just 1 |: (n => n + 1 + 2)") > charged("Just 1 |: (n => n)")
    );
}

#[test]
fn a_bracket_at_a_maybe_place_is_its_good_side() {
    assert_eq!(shown("m: Maybe<Integer> = [:1]\nm"), "Just 1");
}

#[test]
fn marked_payload_bytes_conversion_runs_only_on_the_selected_side() {
    let take = "take: Vec<U8> -> Integer = xs => len xs\n";
    for (source, expected) in [
        ("r: Result<String, String> = Ok \"abc\"\nr |: take", "Ok 3"),
        (
            "r: Result<String, String> = Err \"abc\"\nr |! take",
            "Err 3",
        ),
        (
            "r: Result<String, String> = Err \"abc\"\nr |: take",
            "Err \"abc\"",
        ),
        (
            "r: Result<String, String> = Ok \"abc\"\nr |! take",
            "Ok \"abc\"",
        ),
        (
            "r: Result<String, String> = Ok \"abc\"\nr |: (x => take x)",
            "Ok 3",
        ),
        ("r: Result<String,String> = Ok \"abc\"\n(r) |: take", "Ok 3"),
        (
            "r: Result<String,String> = Ok \"abc\"\nr |: take |: (n => n + 1)",
            "Ok 4",
        ),
    ] {
        assert_eq!(shown(&format!("{take}{source}")), expected);
    }
}

#[test]
fn marked_host_pipeline_supplies_only_payload_bytes_once() {
    use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
    use fipe::{
        compile,
        embed::{Reply, Request},
        value::{structural, Prelude},
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub cat: String -> Vec<U8>\npub grep: (pattern: String) -> (input: Vec<U8> = []) -> Result<String, String>").unwrap();
    let source =
        "use host.cat\nuse host.grep\ncat /file | grep \"a\" |: grep \"^beta\"";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let prelude = Prelude::new(&program.tables.borrow());
    for first_ok in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let mut outcome = machine.run(&program);
        let mut calls = Vec::new();
        loop {
            match outcome {
                Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    let label = program.tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                    calls.push(label.clone());
                    let reply = match calls.len() {
                        1 => {
                            assert_eq!(label, "cat");
                            assert_eq!(arguments, vec![Value::text("/file")]);
                            Value::bytes(b"alpha\nbeta")
                        }
                        2 => {
                            assert_eq!(label, "grep");
                            assert_eq!(
                                arguments,
                                vec![
                                    Value::text("a"),
                                    Value::bytes(b"alpha\nbeta")
                                ]
                            );
                            if first_ok {
                                prelude.ok(Value::text("beta"))
                            } else {
                                prelude.err(Value::text("missing"))
                            }
                        }
                        3 => {
                            assert!(first_ok);
                            assert_eq!(label, "grep");
                            assert_eq!(
                                arguments,
                                vec![
                                    Value::text("^beta"),
                                    Value::bytes(b"beta")
                                ]
                            );
                            prelude.ok(Value::text("selected"))
                        }
                        _ => panic!("left or stage repeated"),
                    };
                    outcome = machine.reply(&program, Reply::Value(Ok(reply)));
                }
                Outcome::Done(Ok(value)) => {
                    assert_eq!(calls.len(), if first_ok { 3 } else { 2 });
                    assert_eq!(
                        structural::show(&value, &program.tables.borrow()),
                        if first_ok {
                            "Ok \"selected\""
                        } else {
                            "Err \"missing\""
                        }
                    );
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
    }
}

#[test]
fn marked_payload_show_operation_is_branch_local() {
    use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
    use fipe::{
        compile,
        embed::{Reply, Request},
        value::structural,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type Label: Show { pub n: Integer }\nimpl Label: Show { show: (self: Self) -> String }").unwrap();
    for (constructor, expected_calls, expected) in [
        ("Ok (host.Label 1)", 1, "Ok 3"),
        ("Err \"skip\"", 0, "Err \"skip\""),
    ] {
        let source = format!("use host\ntake: String -> Integer = s => len s\nr: Result<host.Label,String> = {constructor}\nr |: take");
        let module = parse(lex(&source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let program = compile::compile(&unit);
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let mut outcome = machine.run(&program);
        let mut calls = 0;
        loop {
            match outcome {
                Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    calls += 1;
                    assert_eq!(calls, 1);
                    assert_eq!(
                        program.tables.borrow().declarations[declaration].label,
                        "show"
                    );
                    assert!(
                        matches!(arguments.as_slice(), [Value::Record(record)] if record.fields == vec![Value::Int(1)])
                    );
                    outcome = machine
                        .reply(&program, Reply::Value(Ok(Value::text("abc"))));
                }
                Outcome::Done(Ok(value)) => {
                    assert_eq!(calls, expected_calls);
                    assert_eq!(
                        structural::show(&value, &program.tables.borrow()),
                        expected
                    );
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
    }
}

#[test]
fn marked_payload_limits_check_only_the_selected_payload() {
    let take = "take: U8 -> Integer = n => n\n";
    for (source, expected) in [
        ("r: Result<Integer,String> = Ok 7\nr |: take", "Ok 7"),
        (
            "r: Result<Integer,String> = Err \"skip\"\nr |: take",
            "Err \"skip\"",
        ),
        ("r: Result<String,Integer> = Err 7\nr |! take", "Err 7"),
        (
            "r: Result<String,Integer> = Ok \"skip\"\nr |! take",
            "Ok \"skip\"",
        ),
        ("r: Result<String,String> = Ok \"7\"\nr |: take", "Ok 7"),
        ("r: Result<String,String> = Err \"7\"\nr |! take", "Err 7"),
    ] {
        assert_eq!(shown(&format!("{take}{source}")), expected);
    }
    let source = format!("{take}big = _ => 300\nr: Result<Integer,String> = Ok (big ())\nr |: take");
    let fault = super::run::run(&source).unwrap_err();
    assert!(
        matches!(fault.reason, fipe::value::Reason::Failed { .. }),
        "{fault:?}"
    );
}

#[test]
fn nested_marked_entry_contexts_keep_every_enclosing_guard() {
    let prefix = "take: U8 -> Integer = n => n\ncombine: Result<Integer,String> -> Integer -> Integer = inner n => n\ninner: Result<String,String> = Ok \"300\"\n";
    for (text, expected) in [
        ("outer: Result<Integer,String> = Err \"outer\"\nouter |: combine (inner |: take)", "Err \"outer\""),
        ("outer: Result<Integer,String> = Ok 1\nmiddle: Result<Integer,String> = Err \"middle\"\nouter |: combine (middle |: combine (inner |: take))", "Ok 1"),
        ("outer: Result<Integer,String> = Ok 1\nouter |: combine (if False { inner |: take } else { Ok 0 })", "Ok 1"),
        ("outer: Maybe<Integer> = Just 1\nouter |! (inner |: take)", "Just 1"),
    ] {
        assert_eq!(shown(&format!("{prefix}{text}")), expected);
    }
}

#[test]
fn marked_applied_stage_converts_the_inner_call_input() {
    use super::{check, lex, parse, Declarations, Machine, Outcome, Value};
    use fipe::{
        compile,
        embed::{Reply, Request},
        value::{structural, Prelude},
    };
    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub flags: Result<Bool,String>\npub step: Integer -> Integer",
        )
        .unwrap();
    let source="use host\ntype Ready: Bool where self\ntake: Ready -> Integer -> Integer -> Integer = gate xs => (n => host.step (xs + n))\nhost.flags |: take 7 1";
    let module = parse(lex(source)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    {
        let tables = unit.tables();
        assert!(tables.nodes.iter().any(|node|match &node.kind {
            fipe::ir::Kind::Call {supplies,..}=>supplies.iter().any(|supply|matches!(tables.nodes[supply.value].kind,
                fipe::ir::Kind::Enter {entry,..} if tables.entries[entry].label=="Ready")),
            _=>false,
        }),"actual inner receiving Enter required");
    }
    let program = compile::compile(&unit);
    let prelude = Prelude::new(&program.tables.borrow());
    for active in [true, false] {
        let mut machine = Machine::new(Some(super::run::BUDGET));
        let mut outcome = machine.run(&program);
        let mut calls = 0;
        loop {
            match outcome {
                Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) => {
                    calls += 1;
                    let label = program.tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                    let reply = match calls {
                        1 => {
                            assert_eq!(label, "flags");
                            assert!(arguments.is_empty());
                            if active {
                                prelude.ok(prelude.bool(true))
                            } else {
                                prelude.err(Value::text("skip"))
                            }
                        }
                        2 => {
                            assert!(active);
                            assert_eq!(label, "step");
                            assert_eq!(arguments, vec![Value::Int(8)]);
                            Value::Int(8)
                        }
                        _ => panic!("repeated stage"),
                    };
                    outcome = machine.reply(&program, Reply::Value(Ok(reply)));
                }
                Outcome::Done(Ok(value)) => {
                    assert_eq!(calls, if active { 2 } else { 1 });
                    assert_eq!(
                        structural::show(&value, &program.tables.borrow()),
                        if active { "Ok 8" } else { "Err \"skip\"" }
                    );
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
