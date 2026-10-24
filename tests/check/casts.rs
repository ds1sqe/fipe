use super::check;
use fipe::{
    check as checker, decl::Declarations, lex::lex, parse::parse,
    typed::CastRow,
};

#[test]
fn casts_are_recorded_at_places() {
    for (text, row) in [
        ("k: I64 = round 2.5", CastRow::Number),
        ("x = 2.0\nn: I64 = x", CastRow::Number),
        ("n: I64 = \"42\"", CastRow::Read),
        ("n: U16 = \"8080\"", CastRow::Read),
        ("s: String = 42", CastRow::Show),
        ("s: Vec<U8> = \"abc\"", CastRow::Bytes),
        ("s: Show = 42", CastRow::Pack),
    ] {
        let (_, typed, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
        assert!(
            typed.casts.iter().flatten().any(|c| c.row == row),
            "{text}: {:?}",
            typed.casts
        );
    }
    for text in ["f: F64 = 2", "2", "\"42\"", "n: I64 = 2.0"] {
        let (_, typed, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
        assert!(typed.casts.iter().all(Option::is_none));
    }
    for text in ["n: I64 = 2.5", "s: String = (x => x)", "x: F64 = True"] {
        assert!(!check(text, &Declarations::new()).2.is_empty(), "{text}");
    }
}

#[test]
fn field_arithmetic_cast_keeps_member_obligations() {
    let text = "type Point { x: I64, y: I64 }\nimpl Point { dist: Self -> Self -> F64 = self q => hypot (self.x - q.x) (self.y - q.y) }";
    let (_, typed, errors) = check(text, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(typed.unsolved, []);
}

#[test]
fn codec_and_read_require_their_exact_pairs() {
    let mut host = Declarations::new();
    host.commands("type Packet\ntype Textual: Read\nimpl Textual: Read { read: String -> Result<Self, ParseError> }").unwrap();
    for (text, row) in [
        ("p: Packet = b\"x\"", CastRow::Codec),
        ("p: Textual = \"x\"", CastRow::Read),
    ] {
        let (_, typed, errors) = check(text, &host);
        assert_eq!(errors, [], "{text}");
        assert!(typed.casts.iter().flatten().any(|cast| cast.row == row));
    }
    for text in [
        "p: Packet = \"x\"",
        "p: Textual = True",
        "p: Packet = [1]",
        "p: Vec<I64> = \"x\"",
    ] {
        assert!(!check(text, &host).2.is_empty(), "{text}");
    }
    assert_ne!(
        check("type Packet {}\np: Packet = b\"x\"", &Declarations::new()).2,
        []
    );
}

#[test]
fn casts_do_not_join_inferred_branches_or_elements() {
    let (_, typed, _) = check(
        "n = 1\ntype P {}\nimpl P { f = True => n, False => 2.5 }",
        &Declarations::new(),
    );
    assert!(typed.casts.iter().all(Option::is_none));
    for text in [
        "n = 1\n[n, \"x\"]",
        "n = 1\nc = True\nif c { n } else { \"x\" }",
    ] {
        assert!(!check(text, &Declarations::new()).2.is_empty(), "{text}");
    }
    for text in [
        "s: Show = (x => x)",
        "s: Vec<U8> = (x => x)",
        "type Own { x: Integer }\nimpl Own { show = self => \"x\" }\ns: String = Own 1",
    ] {
        assert!(!check(text, &Declarations::new()).2.is_empty(), "{text}");
    }
}

#[test]
fn fallible_casts_leave_only_target_limits_open() {
    let (_, typed, errors) = check("k: I64 = round 2.5", &Declarations::new());
    assert_eq!(errors, []);
    assert_eq!(typed.casts.iter().flatten().count(), 1);
    assert!(typed
        .checks
        .iter()
        .all(|c| c.name == "I64" && c.known.range.is_none()));
    let mut session = fipe::session::Session::new(Declarations::new());
    session.strict(true);
    let fipe::embed::Outcome::Done(report) =
        session.line("n: Integer = \"42\"")
    else {
        panic!()
    };
    assert_eq!(report.diagnostics, []);
    // A written literal's read has its verdict at compile time.
    let fipe::embed::Outcome::Done(report) = session.line("n: Integer = \"x\"")
    else {
        panic!()
    };
    assert!(matches!(
        report.diagnostics.as_slice(),
        [fipe::session::Diagnostic::Compile {
            reason: fipe::value::Reason::Cast,
            ..
        }]
    ));
}

#[test]
fn every_declared_place_uses_the_table() {
    for text in [
        "type Box { value: String = 42 }\nBox",
        "type Box { value: String }\nBox { value: 42 }",
        "f: Integer -> String = n => n\nf 42",
        "v: Vec<String> = [1, 2]",
        "m: Maybe<String> = [:42]",
        "m: Maybe<String> = Just 42",
        "f: String -> String = s => s\n42 | f",
    ] {
        let (_, typed, errors) = check(text, &Declarations::new());
        assert_eq!(errors, [], "{text}");
        assert!(
            typed.casts.iter().flatten().any(|c| c.row == CastRow::Show),
            "{text}"
        );
    }
}

#[test]
fn byte_defaults_preserve_shape_without_changing_variable_binding() {
    assert_eq!(super::ty("b = b\"x\""), "Vec<U8>");
    assert_eq!(super::ty("d = x => x\nu: U8 = 1\nd u"), "Integer");
    assert_ne!(
        check(
            "total: Vec<Integer> -> Integer = xs => 0\n5 | total",
            &Declarations::new()
        )
        .2,
        []
    );
    let (_, typed, errors) = check(
        "total: Vec<Integer> -> Integer = xs => 0\ntotal 5",
        &Declarations::new(),
    );
    assert_eq!(errors, []);
    assert!(typed.casts.iter().all(Option::is_none));
    assert_eq!(
        super::ty("type Blob: Vec<U8> where self.len > 0\nb: Blob = \"x\""),
        "Blob"
    );
    let mut host = Declarations::new();
    host.commands("type Packet").unwrap();
    for text in [
        "b = b\"x\"\np: Packet = b",
        "f = _ => b\"x\"\np: Packet = f ()",
    ] {
        let (_, typed, errors) = check(text, &host);
        assert_eq!(errors, [], "{text}");
        assert!(typed
            .casts
            .iter()
            .flatten()
            .any(|c| c.row == CastRow::Codec));
    }
}

#[test]
fn a_fraction_literal_at_an_integer_place_is_exact_or_refused() {
    use fipe::infer::Reason;
    for text in [
        "n: I64 = 2.0",
        "n: Integer = 1e3",
        "n: U8 = 250e-1",
        "n: I64 = -2.0e1",
        "n: I64 = (2.0)",
        "f: I64 -> I64 = x => x\nf (2.0)",
    ] {
        let (_, typed, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
        assert!(typed.casts.iter().all(Option::is_none), "{text}");
    }
    for text in [
        "n: I64 = 2.5",
        "n: I64 = (2.5)",
        "n: I64 = ((25e-1))",
        "f: I64 -> I64 = x => x\nf (2.5)",
        "g: U8 -> U8 = x => x\ng 2.5",
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.reason, Reason::NotExact { .. })),
            "{text}: {errors:?}"
        );
    }
}

#[test]
fn resource_types_exclude_automatic_codec_and_data_construction() {
    let mut host = Declarations::new();
    host.commands("pub type resource Table").unwrap();
    for source in [
        "x: Table = b\"x\"",
        "bytes = b\"x\"\nx: Table = bytes",
        "f = _ => b\"x\"\nx: Table = f ()",
        "x: Table = 1",
        "x: Table = \"x\"",
        "x = Table {}",
        "x = Table ()",
        "x = Table.default",
    ] {
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        assert!(
            fipe::check::check(
                &module,
                &host,
                &[],
                fipe::check::Options::default()
            )
            .is_err(),
            "{source}"
        );
    }
    host.module("data", "pub type Table").unwrap();
    assert!(check("use data\nx: data.Table = b\"x\"", &host)
        .1
        .casts
        .iter()
        .flatten()
        .any(|c| c.row == CastRow::Codec));
}

#[test]
fn resource_supplied_operations_and_containing_alias_types_stay_legal() {
    let mut host = Declarations::new();
    host.commands("pub type resource Table\npub get: Table\nimpl Table: Read { read: String -> Result<Self, ParseError> }\nimpl Table: Default { default: Self }\nimpl Table: Show { show: Self -> String }").unwrap();
    for (source, row) in [
        ("x: Table = \"source\"", CastRow::Read),
        ("x: String = get", CastRow::Show),
    ] {
        let (_, typed, errors) = check(source, &host);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert!(typed.casts.iter().flatten().any(|c| c.row == row));
    }
    for source in [
        "f: Table -> Vec<Table> = x => [x]",
        "f: Table -> (() -> Table) = x => (_ => x)",
        "type Holder { value: Table }\nf: Table -> Holder = x => Holder x",
        "f = _ => Table.default",
        "f: () -> Vec<Table> = _ => Vec.default",
    ] {
        let (_, _, errors) = check(source, &host);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
    let module = fipe::parse::parse(fipe::lex::lex("x = Compile get")).unwrap();
    let errors = fipe::check::check(
        &module,
        &host,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, fipe::check::Error::Type(fipe::infer::Error { reason: fipe::infer::Reason::Limit { message }, .. }) if message.starts_with("timing requires compile") && message.contains("get"))), "{errors:?}");
}

#[test]
fn resource_aliases_cannot_restore_codec_but_packet_aliases_keep_it() {
    let mut host = Declarations::new();
    host.commands(
        "pub type resource Table\npub type resource Rd<A>\npub type Packet",
    )
    .unwrap();
    for (prefix, target, base) in [
        ("type Alias: Table where True\n", "Alias", "Table"),
        (
            "type Alias: Table where True\ntype Next: Alias where True\n",
            "Next",
            "Table",
        ),
        ("type Alias<A>: Rd<A> where True\n", "Alias<()>", "Rd<()>"),
    ] {
        let source = format!("{prefix}f: {target} -> {base} = x => x");
        let module = fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
        fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
        let source = format!("{prefix}bytes = b\"x\"\nx: {target} = bytes");
        let (_, typed, errors) = check(&source, &host);
        assert!(
            errors.iter().any(|e| matches!(
                e.reason,
                fipe::infer::Reason::Mismatch { .. }
                    | fipe::infer::Reason::Conversion { .. }
            )),
            "{source}: {errors:?}"
        );
        assert!(typed
            .casts
            .iter()
            .flatten()
            .all(|c| c.row != CastRow::Codec));
        let module = fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
        assert!(fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default()
        )
        .unwrap_err()
        .iter()
        .any(|e| matches!(
            e,
            fipe::check::Error::Type(fipe::infer::Error {
                reason: fipe::infer::Reason::Mismatch { .. }
                    | fipe::infer::Reason::Conversion { .. },
                ..
            })
        )));
    }
    let (_, typed, errors) =
        check("type Alias: Packet where True\nx: Alias = b\"x\"", &host);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(typed
        .casts
        .iter()
        .flatten()
        .any(|c| c.row == CastRow::Codec));
}

#[test]
fn resource_record_syntax_also_rejects_ordinary_opaque_types() {
    for header in ["pub type Table", "pub type resource Table"] {
        let mut host = Declarations::new();
        host.commands(header).unwrap();
        for source in [
            "x = Table {}",
            "f: Table -> Integer = { Table {} => 1, _ => 2 }",
        ] {
            let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
            let errors = fipe::check::check(
                &module,
                &host,
                &[],
                fipe::check::Options::default(),
            )
            .unwrap_err();
            assert!(errors.iter().any(|e| matches!(e, fipe::check::Error::Type(fipe::infer::Error { span, reason: fipe::infer::Reason::NotRecord }) if &source[span.start..span.end] == "Table {}")), "{source}: {errors:?}");
        }
    }
    // Proper records and sum payload record patterns retain their full check.
    let host = Declarations::new();
    for source in [
        "type Point { value: Integer }\nf: Point -> Integer = Point { value } => value",
        "type Choice { Some(value: Integer), None }\nf: Choice -> Integer = { Some { value } => value, None => 0 }",
    ] {
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
    }
    let module = fipe::parse::parse(fipe::lex::lex("x = Missing {}")).unwrap();
    let errors = fipe::check::check(
        &module,
        &host,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            errors.as_slice(),
            [fipe::check::Error::Resolve(fipe::resolve::Error {
                reason: fipe::resolve::Reason::UnknownType,
                ..
            })]
        ),
        "{errors:?}"
    );
}

#[test]
fn resource_host_written_constructor_full_check() {
    // This dependent regression must pass after the separately owned02.09 fix.
    // Inference-only success is not a substitute for public full-check success.
    for header in ["pub type Table", "pub type resource Table"] {
        let mut host = Declarations::new();
        host.commands(&format!("{header}\npub get: Table\npub type Wrap {{ pub value: Table }}\nimpl Wrap {{ () => Self {{ value: get }} }}")).unwrap();
        let source = "f = _ => Wrap ()";
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
    }
}

#[test]
fn foreign_written_constructor_constraints_are_checked() {
    for body in [
        "x => 7",
        "x => missing",
        "x if 1 => Self { value: x }",
        "0 => Self { value: 1 }",
    ] {
        let mut host = Declarations::new();
        host.commands(&format!(
            "pub type Wrap {{ pub value: Integer }}\nimpl Wrap {{ {body} }}"
        ))
        .unwrap();
        let module =
            fipe::parse::parse(fipe::lex::lex("f = _ => Wrap 1")).unwrap();
        assert!(
            fipe::check::check(
                &module,
                &host,
                &[],
                fipe::check::Options::default()
            )
            .is_err(),
            "{body}"
        );
    }
    for (declarations, source) in [
        (
            "pub type Wrap { pub value: Integer }\nimpl Wrap { 0 => Self { value: 1 }, n => Self { value: n + 1 } }",
            "f = _ => Wrap 2",
        ),
        (
            "pub type Wrap { pub value: Integer }\nimpl Wrap { x (y = 2) => Self { value: x + y } }",
            "f = _ => Wrap 3",
        ),
        (
            "pub type Wrap<T> { pub value: T }\nimpl Wrap<T> { x => Self { value: x } }",
            "f = _ => (Wrap 1, Wrap \"text\")",
        ),
        (
            "pub type Wrap { pub value: Integer }\nimpl Wrap { n if n > 0 => Wrap (n - 1), _ => Self { value: 0 } }",
            "f = _ => Wrap 2",
        ),
    ] {
        let mut host = Declarations::new();
        host.commands(declarations).unwrap();
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
    }
}

#[test]
fn direct_file_written_constructor_reuses_its_checked_definition() {
    let file = "pub type Point { pub x: Integer }\nimpl Point { x => Self { x: x + 1 } }\npub make: Integer -> Point = x => Point x";
    let files = [(
        "point".to_owned(),
        fipe::parse::parse(fipe::lex::lex(file)).unwrap(),
    )];
    for source in [
        "use point\nf = _ => point.Point 1",
        "use point\nf = _ => point.make 1",
    ] {
        let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        let unit = fipe::check::check(
            &module,
            &Declarations::new(),
            &files,
            fipe::check::Options::default(),
        )
        .unwrap();
        assert!(unit.tables().definitions.iter().any(|d| d.label == "Self"));
    }
}

#[test]
fn invalid_foreign_constructor_diagnostics_keep_main_and_owner_spans_separate()
{
    let source = "f = _ => Wrap 1";
    let supplied = format!(
        "# {}\npub type Wrap {{ pub value: Integer }}\nimpl Wrap {{ x => missing }}",
        "padding".repeat(200)
    );
    let mut host = Declarations::new();
    host.commands(&supplied).unwrap();
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let errors = fipe::check::check(
        &module,
        &host,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_err();
    assert!(errors.iter().any(|error| matches!(error, fipe::check::Error::Type(fipe::infer::Error { span, reason: fipe::infer::Reason::DeclarationValue { declaration, .. } }) if span.end <= source.len() && declaration.start > source.len())));
    for error in errors {
        if let fipe::check::Error::Type(error) = error {
            assert!(error.span.end <= source.len(), "{error:?}");
        }
    }
    let file_source = supplied.replace("missing", "7");
    let files = [(
        "point".to_owned(),
        fipe::parse::parse(fipe::lex::lex(&file_source)).unwrap(),
    )];
    let source = "use point\nf = _ => point.Wrap 1";
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let errors = fipe::check::check(
        &module,
        &Declarations::new(),
        &files,
        fipe::check::Options::default(),
    )
    .unwrap_err();
    for error in errors {
        if let fipe::check::Error::Type(error) = error {
            assert!(error.span.end <= source.len(), "{error:?}");
        }
    }
}

#[test]
fn supplied_constructor_checks_its_group_without_unrelated_impl_bodies() {
    for (declarations, valid) in [
        (
            "pub type Wrap { pub value: Integer }\nimpl Wrap { x => Self { value: x } }\nimpl Wrap { bad = missing }",
            true,
        ),
        (
            "pub type Wrap { pub value: Integer }\nimpl Wrap { x => Self { value: x }, bad = missing }",
            false,
        ),
    ] {
        let mut host = Declarations::new();
        host.commands(declarations).unwrap();
        let module =
            fipe::parse::parse(fipe::lex::lex("f = _ => Wrap 1")).unwrap();
        let result = fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        );
        assert_eq!(result.is_ok(), valid, "{result:?}");
    }
}

#[test]
fn claim_qualifiers_local_contract_controls() {
    for (bound, member) in [
        ("", "inspect = self => 1"),
        (": Eq", "inspect: Eq U => Self -> Integer = self => 1"),
    ] {
        let source = format!("type Key where self.inspect: Integer\ntype Cell<T{bound}>: Key {{ value: T }}\nimpl Cell<U>: Key {{ {member} }}\n0");
        let module = fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
        let result = fipe::check::check(
            &module,
            &Declarations::new(),
            &[],
            fipe::check::Options::default(),
        );
        assert!(result.is_ok(), "{source}: {result:?}");
    }
}

#[test]
fn claim_qualifiers_local_written_and_inferred_are_not_extra_assumptions() {
    for member in [
        "inspect: Eq U => Self -> Integer = self => 1",
        "inspect = self => if self.value == self.value { 1 } else { 0 }",
    ] {
        for (bound, valid) in [("", false), (": Eq", true)] {
            let source = format!("type Key where self.inspect: Integer\ntype Cell<T{bound}>: Key {{ value: T }}\nimpl Cell<U>: Key {{ {member} }}\n0");
            let module = fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
            let result = fipe::check::check(
                &module,
                &Declarations::new(),
                &[],
                fipe::check::Options::default(),
            );
            assert_eq!(result.is_ok(), valid, "{source}: {result:?}");
            if let Err(errors) = result {
                assert!(
                    errors.iter().any(|e| match e {
                        fipe::check::Error::Type(fipe::infer::Error {
                            reason:
                                fipe::infer::Reason::ClaimBlock { problem, .. },
                            ..
                        }) => problem == "has stronger member prerequisites",
                        fipe::check::Error::Type(fipe::infer::Error {
                            reason:
                                fipe::infer::Reason::MissingClaim { claim, .. },
                            ..
                        }) =>
                            member.starts_with("inspect =")
                                && claim.to_string() == "Eq",
                        _ => false,
                    }),
                    "{errors:?}"
                );
            }
        }
    }
}

#[test]
fn claim_qualifiers_supplied_bounds_and_conditional_derived_controls() {
    let mut host = Declarations::new();
    host.module("csv", "pub type Key where self.inspect: Integer\npub type Packet\npub type resource Rd<T: Eq>\npub get: Rd<Integer>\npub bad: Rd<Packet>\npub echo: Key -> Key\nimpl Rd<T>: Key { inspect: Eq T => (self: Self) -> Integer }").unwrap();
    for text in ["use csv\ncsv.echo csv.get", "use csv\n(csv.get).inspect"] {
        let module = fipe::parse::parse(fipe::lex::lex(text)).unwrap();
        assert!(
            fipe::check::check(
                &module,
                &host,
                &[],
                fipe::check::Options::default()
            )
            .is_ok(),
            "{text}"
        );
    }
    for text in ["use csv\ncsv.echo csv.bad", "use csv\n(csv.bad).inspect"] {
        let module = fipe::parse::parse(fipe::lex::lex(text)).unwrap();
        let errors = fipe::check::check(
            &module,
            &host,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(
                e,
                fipe::check::Error::Type(fipe::infer::Error {
                    reason: fipe::infer::Reason::MissingClaim { .. },
                    ..
                })
            )),
            "{errors:?}"
        );
    }
    for container in ["Vec", "Deque"] {
        for class in ["Eq", "Ord", "Show"] {
            for (element, valid) in [("Integer", true), ("Packet", false)] {
                let source = format!("use csv\nf: {container}<{element}> -> () = x => {{ y: {class} = x\n() }}");
                let module =
                    fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
                let result = fipe::check::check(
                    &module,
                    &host,
                    &[],
                    fipe::check::Options::default(),
                );
                assert_eq!(result.is_ok(), valid, "{source}: {result:?}");
            }
        }
    }
    let source = "type Pair<T: Eq & Show>: Eq & Show { value: T }\nf: Pair<Integer> -> Eq = p => p\ng: Pair<Integer> -> Show = p => p";
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let result = fipe::check::check(
        &module,
        &Declarations::new(),
        &[],
        fipe::check::Options::default(),
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn claim_qualifiers_inferred_literal_keeps_its_owner_mapping() {
    for (bound, valid) in [("", false), (": Literal", true)] {
        let source = format!("type Key<T> where self.make: T\ntype Cell<T{bound}>: Key<T> {{ value: T }}\nimpl Cell<T>: Key<T> {{ make: Self -> T = self => 1 }}\n0");
        let module = fipe::parse::parse(fipe::lex::lex(&source)).unwrap();
        let result = fipe::check::check(
            &module,
            &Declarations::new(),
            &[],
            fipe::check::Options::default(),
        );
        assert_eq!(result.is_ok(), valid, "{source}: {result:?}");
        if let Err(errors) = result {
            assert!(
                errors.iter().any(|e| matches!(
                    e,
                    fipe::check::Error::Type(fipe::infer::Error {
                        reason: fipe::infer::Reason::ClaimBlock { .. },
                        ..
                    })
                )),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn claim_qualifiers_local_receiver_cannot_narrow_universal_owner() {
    let source = "type Key where self.inspect: Integer\ntype Cell<T>: Key { value: T }\nimpl Cell<U>: Key { inspect: Cell<Integer> -> Integer = self => 1 }\n0";
    let module = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
    let errors = fipe::check::check(
        &module,
        &Declarations::new(),
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            e,
            fipe::check::Error::Type(fipe::infer::Error {
                reason: fipe::infer::Reason::Mismatch { .. },
                ..
            })
        )),
        "{errors:?}"
    );
}

#[test]
fn marked_pipe_records_payload_bytes_conversion() {
    let text = "take: Vec<U8> -> Integer = xs => len xs\nr: Result<String, String> = Ok \"abc\"\nr |: take";
    let (_, typed, errors) = check(text, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    assert!(typed
        .casts
        .iter()
        .flatten()
        .any(|cast| cast.row == CastRow::Bytes));
    let (_, _, errors) = check(&text.replace("|:", "|"), &Declarations::new());
    assert!(
        errors.iter().any(|error| matches!(
            error.reason,
            fipe::infer::Reason::OutcomeText { .. }
        )),
        "{errors:?}"
    );
}

#[test]
fn marked_payload_entries_retain_the_receiving_call() {
    let text = "take: U8 -> Integer = n => n\nr: Result<String,String> = Ok \"7\"\nr |: take";
    let (module, typed, errors) = check(text, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    let fipe::ast::Entry::Line(line) = module.entries.last().unwrap() else {
        panic!("pipe line")
    };
    let fipe::ast::ExprKind::Pipe { left, right, .. } =
        module.expr(*line).unwrap().kind
    else {
        panic!("marked pipe")
    };
    let receiving = module.expr(right).unwrap().span;
    let entries: Vec<_> = typed
        .checks
        .iter()
        .filter(|check| check.value == Some(left))
        .collect();
    assert!(!entries.is_empty());
    assert!(entries
        .iter()
        .all(|check| check.receiving_call == Some(receiving)));
}

const MARKED_PRIME: &str = "trial = n d => if d * d > n { True } else { if n % d == 0 { False } else { trial n (d + 1) } }\nis_prime = n => n > 1 && trial n 2\ntype Prime: Integer where is_prime self\ntake: Prime -> Integer = n => n\n";

#[test]
fn marked_compile_entries_keep_payload_context_budget_and_left_validation() {
    use fipe::{
        check::{self, Options},
        lex::lex,
        parse::parse,
    };
    let declarations = Declarations::new();
    let active =
        format!("{MARKED_PRIME}r: Result<Integer,String> = Ok 7\nr |: take");
    let module = parse(lex(&active)).unwrap();
    let proved =
        check::check(&module, &declarations, &[], Options::default()).unwrap();
    assert!(!proved.plan.iter().any(|entry| entry.label == "Prime"));
    let budgeted = check::check(
        &module,
        &declarations,
        &[],
        Options {
            compile_budget: 1,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(budgeted.plan.iter().any(|entry| entry.label == "Prime"
        && entry.reason == fipe::ir::Reason::CompileBudget));
    let invalid_left = format!(
        "{MARKED_PRIME}n: Prime = 8\nr: Result<Prime,String> = Ok n\nr |: take"
    );
    let module = parse(lex(&invalid_left)).unwrap();
    let errors = check::check(&module, &declarations, &[], Options::default())
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, check::Error::Compile { .. })),
        "{errors:?}"
    );
}

#[test]
fn nested_marked_parameter_entries_preserve_relative_element_path() {
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub values: Result<Vec<Integer>,String>")
        .unwrap();
    let text = "use host\ntake: Vec<U8> -> Integer = xs => len xs\nhost.values |: take";
    let (module, typed, errors) = check(text, &declarations);
    assert!(errors.is_empty(), "{errors:?}");
    let fipe::ast::Entry::Line(line) = module.entries.last().unwrap() else {
        panic!("pipe line")
    };
    let fipe::ast::ExprKind::Pipe { right, .. } =
        module.expr(*line).unwrap().kind
    else {
        panic!("marked pipe")
    };
    let stage = module.expr(right).unwrap().span;
    assert!(
        typed
            .checks
            .iter()
            .any(|entry| entry.receiving_call == Some(stage)
                && entry.path == vec![fipe::ir::Projection::Element]),
        "{:?}",
        typed.checks
    );
}

#[test]
fn contextual_entry_does_not_answer_effectful_left_during_checking() {
    use fipe::{
        check::{self, Options},
        compile,
        embed::{Outcome, Request},
        lex::lex,
        parse::parse,
        vm::Machine,
    };
    let mut declarations = Declarations::new();
    declarations.module("host", "pub tick: () -> ()").unwrap();
    let text=format!("{MARKED_PRIME}use host\nr: Result<Integer,String> = Ok 7\n{{ host.tick ()\nr }} |: take");
    let module = parse(lex(&text)).unwrap();
    let unit =
        check::check(&module, &declarations, &[], Options::default()).unwrap();
    assert!(
        unit.plan.iter().any(|entry| entry.label == "Prime"
            && entry.reason == fipe::ir::Reason::CompileTimeEvaluation),
        "{:?}",
        unit.plan
    );
    {
        let tables = unit.tables();
        let stage = tables
            .nodes
            .iter()
            .find_map(|node| match node.kind {
                fipe::ir::Kind::Marked { stage, .. } => Some(stage),
                _ => None,
            })
            .expect("marked receiving stage");
        let fipe::ir::Kind::Call { supplies, .. } = &tables.nodes[stage].kind
        else {
            panic!("receiving call")
        };
        assert!(supplies.iter().any(|supply| matches!(tables.nodes[supply.value].kind,
            fipe::ir::Kind::Enter {entry,..} if tables.entries[entry].label=="Prime"
                && tables.entries[entry].reason==fipe::ir::Reason::CompileTimeEvaluation)));
    }

    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(100_000));
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("runtime tick")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "tick"
    );
    assert_eq!(arguments, vec![fipe::value::Value::Unit]);
    assert!(machine.abandon());
}

#[test]
fn nested_compile_entries_preserve_all_enclosing_marked_conditions() {
    use fipe::{
        check::{self, Options},
        lex::lex,
        parse::parse,
    };
    let declarations = Declarations::new();
    let prefix=format!("{MARKED_PRIME}combine: Result<Integer,String> -> Integer -> Integer = inner n => n\ninner: Result<Integer,String> = Ok 8\n");
    for tail in [
        "outer: Result<Integer,String> = Err \"outer\"\nouter |: combine (inner |: take)",
        "outer: Result<Integer,String> = Ok 1\nmiddle: Result<Integer,String> = Err \"middle\"\nouter |: combine (middle |: combine (inner |: take))",
        "outer: Result<Integer,String> = Ok 1\nouter |: combine (if False { inner |: take } else { Ok 0 })",
        "outer: Maybe<Integer> = Just 1\nouter |! (inner |: take)",
    ] {
        let text=format!("{prefix}{tail}");
        let (..,typed,errors)=super::check(&text,&declarations);
        assert!(errors.is_empty(),"{errors:?}");
        assert!(typed.checks.iter().any(|check| check.name=="Prime" && check.reason==fipe::typed::CheckReason::CompileTimeEvaluation),"{:?}",typed.checks);
        let module=parse(lex(&text)).unwrap();
        let unit=check::check(&module,&declarations,&[],Options::default()).unwrap();
        assert!(unit.plan.iter().any(|entry| entry.label=="Prime"),"inactive/conditional must not be falsely proved");
    }
    let text=format!("{prefix}outer: Result<Integer,String> = Ok 1\nouter |: combine (inner |: take)");
    let module = parse(lex(&text)).unwrap();
    let errors = check::check(&module, &declarations, &[], Options::default())
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, check::Error::Compile { .. })),
        "{errors:?}"
    );
}

#[test]
fn marked_applied_stage_keeps_the_actual_inner_call_binding() {
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub flags: Result<Bool,String>")
        .unwrap();
    let source="use host\ntype Ready: Bool where self\ntake: Ready -> Integer -> Integer -> Integer = gate xs => (n => xs + n)\nhost.flags |: take 7 1";
    let (module, typed, errors) = check(source, &declarations);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(
        typed.bound.values().any(|bound| bound.applied.len() == 1
            && bound
                .supplies
                .iter()
                .any(|(_, s)| matches!(s, fipe::typed::Supply::Pipe(_)))),
        "actual applied bound required"
    );
    let fipe::ast::Entry::Line(line) = module.entries.last().unwrap() else {
        panic!("pipe line")
    };
    let fipe::ast::ExprKind::Pipe { right, .. } =
        module.expr(*line).unwrap().kind
    else {
        panic!("marked pipe")
    };
    let stage = module.expr(right).unwrap().span;
    assert!(
        typed.checks.iter().any(|entry| entry.name == "Ready"
            && entry.receiving_call == Some(stage)),
        "receiving limit entry: {:?}",
        typed.checks
    );
}

fn automatic_packed_claim_check_generic_key(marked: bool) {
    let resource = "pub type Key: Eq\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool }\npub type resource Rd<T>\npub integer: Rd<Integer>\npub text: Rd<String>\npub choose: () -> Bool\nimpl Rd<a>: Key { eq: Self -> (self: Self) -> Bool }";
    let source = "use csv\nsame: Eq a => a -> a -> Bool = lhs rhs => lhs == rhs\nx: csv.Key = csv.integer\ny: csv.Key = csv.text\nsame x y";
    let header = if marked {
        resource.to_string()
    } else {
        resource.replacen("resource ", "", 1)
    };
    let mut declarations = Declarations::new();
    declarations.module("csv", &header).unwrap();
    let module = parse(lex(source)).unwrap();
    let checked = checker::check(
        &module,
        &declarations,
        &[],
        checker::Options::default(),
    );
    assert!(checked.is_ok(), "{header}: {checked:?}");
}

#[test]
fn automatic_packed_claim_keeps_generic_resource_key_unambiguous() {
    automatic_packed_claim_check_generic_key(true);
}

#[test]
fn automatic_packed_claim_keeps_generic_opaque_key_unambiguous() {
    automatic_packed_claim_check_generic_key(false);
}

#[test]
fn automatic_packed_claim_preserves_inherited_and_receiver_only_members() {
    for (header, source) in [
        ("pub type Key: Eq\npub value: Key", "use csv\nsame: Eq a => a -> a -> Bool = x y => x == y\nsame csv.value csv.value"),
        ("pub type Base: Eq\npub type Key: Base\nimpl Key: Eq { eq: Self -> (self: Self) -> Bool }\npub value: Key", "use csv\nsame: Eq a => a -> a -> Bool = x y => x == y\nsame csv.value csv.value"),
        ("pub type Key: Show\nimpl Key: Show { show: (self: Self) -> String }\npub value: Key", "use csv\nrender: Show a => a -> String = x => show x\nrender csv.value"),
    ] {
        let mut declarations = Declarations::new();
        declarations.module("csv", header).unwrap();
        let module = parse(lex(source)).unwrap();
        let checked = checker::check(&module, &declarations, &[], checker::Options::default());
        assert!(checked.is_ok(), "{header}\n{source}: {checked:?}");
    }
}

#[test]
fn automatic_packed_claim_preserves_distinct_conversion_arguments() {
    let prefix = "type P: From<Integer> & From<String> { n: Integer }\nimpl P: From<Integer> { from: Integer -> Self = n => Self n }\nimpl P: From<String> { from: String -> Self = s => Self 0 }\n";
    let source = format!("{prefix}(1 | to<P>, \"x\" | to<P>)");
    let module = parse(lex(&source)).unwrap();
    assert!(checker::check(
        &module,
        &Declarations::new(),
        &[],
        checker::Options::default()
    )
    .is_ok());
    for call in ["P.from True", "bad = x => P.from x"] {
        let source = format!("{prefix}{call}");
        let module = parse(lex(&source)).unwrap();
        let errors = checker::check(
            &module,
            &Declarations::new(),
            &[],
            checker::Options::default(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, checker::Error::Type(error) if matches!(&error.reason, fipe::infer::Reason::Conversion { .. }))), "{source}: {errors:?}");
    }
}

#[test]
fn host_written_members_keep_binding_reentry_in_one_source() {
    let header="pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: Integer -> Integer = n => helper n }\npub helper: Integer -> Integer = n => A.f n";
    let mut host = fipe::decl::Declarations::new();
    host.module("probe", header).unwrap();
    let module =
        fipe::parse::parse(fipe::lex::lex("use probe\nprobe.A 0")).unwrap();
    let unit = fipe::check::check(
        &module,
        &host,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    for label in ["f", "helper", "Self"] {
        assert!(
            tables.definitions.iter().any(|d| d.label == label),
            "{label}"
        );
    }
}

#[test]
fn host_written_members_keep_cross_owner_groups_and_default_scope() {
    for header in [
        "pub type A { pub n: Integer }\npub type B { pub n: Integer }\nimpl A { n => Self { n: n }, f: Integer -> Integer = n => B.g n }\nimpl B { g: Integer -> Integer = n => A.f n }",
        "pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: Integer -> Integer = n => entry n }\npub entry: (seed: Integer) -> (n: Integer = (A.f seed)) -> Integer",
        "pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: Integer -> Integer = n => entry n }\npub entry: (seed: Integer) -> (n: Integer = seed) -> Integer",
    ] {
        eprintln!("HOST_CASE {header}");
        let mut host=fipe::decl::Declarations::new();host.module("probe",header).unwrap();
        let module=fipe::parse::parse(fipe::lex::lex("use probe\nprobe.A 0")).unwrap();
        fipe::check::check(&module,&host,&[],fipe::check::Options::default()).unwrap_or_else(|errors|panic!("{header}: {errors:?}"));
    }
}

#[test]
fn host_written_defaults_keep_prior_root_and_repeated_instances_separate() {
    for (header,main) in [
        ("pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, factory: Self = Self { n: 2 }, f: () -> Integer = _ => { prior = A.factory.n\nentry 7 } }\npub entry: (seed: Integer) -> (n: Integer = seed) -> Integer", "use probe\nprobe.A.f ()"),
        ("pub type A { pub n: Integer }\nimpl A { n => Self { n: n }, f: () -> (Integer * String) = _ => { first: Integer = entry 7\nsecond: String = entry \"word\"\n(first, second) } }\npub entry: (seed: a) -> (n: a = seed) -> a", "use probe\nprobe.A.f ()"),
    ] {
        let mut host=fipe::decl::Declarations::new();host.module("probe",header).unwrap();
        let module=fipe::parse::parse(fipe::lex::lex(main)).unwrap();
        fipe::check::check(&module,&host,&[],fipe::check::Options::default()).unwrap_or_else(|errors|panic!("{header}: {errors:?}"));
    }
}

#[test]
fn host_written_binding_annotations_preserve_qualified_constraints_and_lazy_errors(
) {
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("types", "pub type Box { pub n: Integer }")
        .expect("types header");
    declarations.module("probe", "pub good: Integer -> Integer = x => x\npub take: Eq a => a -> a = x => x\npub typed: types.Box -> types.Box = x => x\npub bad: Missing -> Integer = x => 0\npub type A { pub n: Integer }\nimpl A { good_case: Integer -> Integer = n => good n, take_case: Integer -> Integer = n => take n, typed_case: types.Box -> types.Box = x => typed x }\npub type B { pub n: Integer }\nimpl B { bad_case: Integer -> Integer = n => bad n }").expect("declaration header");
    for (source, label) in [
        ("use probe\nprobe.good 7", "good"),
        ("use probe\nprobe.take 7", "take"),
        (
            "use probe\nkeep: types.Box -> types.Box = probe.typed",
            "typed",
        ),
        ("use probe\nprobe.A.good_case 7", "good_case"),
        ("use probe\nprobe.A.take_case 7", "take_case"),
        (
            "use probe\nkeep: types.Box -> types.Box = probe.A.typed_case",
            "typed_case",
        ),
    ] {
        let module = fipe::parse::parse(fipe::lex::lex(source)).expect("main");
        let unit = fipe::check::check(
            &module,
            &declarations,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        assert!(
            unit.tables()
                .definitions
                .iter()
                .any(|definition| definition.label == label),
            "checked body {label}"
        );
    }
    let module = fipe::parse::parse(fipe::lex::lex("use probe\nprobe.bad 7"))
        .expect("bad main");
    let errors = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .expect_err("reached bad annotation");
    assert!(errors.iter().any(|error| matches!(error, fipe::check::Error::Type(error) if matches!(&error.reason, fipe::infer::Reason::DeclarationValue { name, .. } if name == "bad"))), "{errors:?}");
    let module =
        fipe::parse::parse(fipe::lex::lex("use probe\nprobe.B.bad_case 7"))
            .expect("bad group main");
    let errors = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .expect_err("reached bad annotation through group");
    assert!(errors.iter().any(|error| matches!(error, fipe::check::Error::Type(error) if matches!(&error.reason, fipe::infer::Reason::DeclarationValue { .. }))), "{errors:?}");
}

#[test]
fn host_written_default_owners_keep_direct_binding_recursion_distinct_from_member_backedges(
) {
    for (body, valid) in [("helper n", false), ("n", true), ("A.f n", true)] {
        let header = format!("pub type A {{ pub n: Integer }}\nimpl A {{ f: Integer -> Integer = n => helper n }}\npub helper: Integer -> Integer = n => {body}");
        let mut declarations = fipe::decl::Declarations::new();
        declarations
            .module("probe", &header)
            .expect("recursive source header");
        let module =
            fipe::parse::parse(fipe::lex::lex("use probe\nprobe.A.f 7"))
                .expect("main");
        let result = fipe::check::check(
            &module,
            &declarations,
            &[],
            fipe::check::Options::default(),
        );
        if valid {
            let unit = result.unwrap_or_else(|errors| {
                panic!("member/healthy source {body}: {errors:?}")
            });
            assert!(unit
                .tables()
                .definitions
                .iter()
                .any(|definition| definition.label == "f"));
            assert!(unit
                .tables()
                .definitions
                .iter()
                .any(|definition| definition.label == "helper"));
        } else {
            let errors = result.expect_err(
                "direct Checking binding recursion remains refused",
            );
            assert!(errors.iter().any(|error| matches!(error, fipe::check::Error::Type(error) if matches!(&error.reason, fipe::infer::Reason::DeclarationValue { .. }))), "{errors:?}");
        }
    }
}
