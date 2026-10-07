use super::check;
use fipe::{decl::Declarations, typed::CastRow};

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
    for text in ["f: F64 = 2", "2", "\"42\""] {
        let (_, typed, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
        assert!(typed.casts.iter().all(Option::is_none));
    }
    for text in [
        "n: I64 = 2.5",
        "n: I64 = 2.0",
        "s: String = (x => x)",
        "x: F64 = True",
    ] {
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
    for text in ["s: Show = (x => x)", "s: Vec<U8> = (x => x)", "type Own { x: Integer }\nimpl Own { show = self => \"x\" }\ns: String = Own 1"] {
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
    let fipe::embed::Outcome::Done(report) = session.line("n: Integer = \"x\"")
    else {
        panic!()
    };
    assert_eq!(report.diagnostics, []);
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
