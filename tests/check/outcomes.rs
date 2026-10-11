use super::{check, ty};
use fipe::{decl::Declarations, infer::Reason};

#[test]
fn families_can_be_learned_from_siblings() {
    assert_eq!(
        ty("type Present: Maybe<Integer> where True\nm: Present = [:1]"),
        "Present"
    );
    for text in [
        "c = True\nif c { [:1] } else { Just 2 }",
        "c = True\nif c { Just 2 } else { [:1] }",
        "f = True => [:1], False => Just 2\nTrue | f",
        "f = [:x] => x, Nothing => 0\nf (Just 1)",
    ] {
        let result = ty(text);
        assert!(
            result == "Maybe<Integer>" || result == "Integer",
            "{text}: {result}"
        );
    }
    for (text, reason) in [
        ("m = [:1]", Reason::UnknownFamily),
        ("m: Maybe<Integer> = [!1]", Reason::NothingPayload),
        ("Just 1 | { [!e] => e }", Reason::NothingPayload),
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(
            errors.iter().any(|e| e.reason == reason),
            "{text}: {errors:?}"
        );
    }
}

#[test]
fn outcome_text_guard_checks_only_the_source_family() {
    for value in ["Just 1", "Ok 1", "Left 1"] {
        for target in ["String", "Vec<U8>"] {
            let text = format!("s: {target} = {value}");
            let (_, _, errors) = check(&text, &Declarations::new());
            assert!(
                errors
                    .iter()
                    .any(|e| matches!(e.reason, Reason::OutcomeText { .. })),
                "{text}: {errors:?}"
            );
        }
    }
    assert_eq!(ty("s: String = [Just 1]"), "String");
}

#[test]
fn outcome_readings_are_recorded_for_lowering() {
    use fipe::typed::{Mode, Rebuild};
    for (text, expected) in [
        ("Just 1 |: (x => x + 1)", Rebuild::Wrap),
        ("Just 1 |: (_ => Just 2)", Rebuild::Direct),
        ("Nothing |! 1", Rebuild::Discard),
    ] {
        let (_, typed, errors) = super::check(text, &Declarations::new());
        assert_eq!(errors, []);
        assert!(typed.rebuilds.values().any(|v| *v == expected));
        assert!(typed
            .rebuilds
            .keys()
            .all(|site| typed.rebuild_types.contains_key(site)));
    }
    for (text, expected) in [
        ("True || False", Mode::Bool),
        ("Nothing || Just 4", Mode::Family),
        ("Nothing || 4", Mode::Payload),
    ] {
        let (_, typed, errors) = super::check(text, &Declarations::new());
        assert_eq!(errors, []);
        assert!(typed.modes.values().any(|v| *v == expected));
    }
}

#[test]
fn a_stage_result_a_literal_fixes_later_reads_plain_or_whole() {
    use fipe::typed::Rebuild;
    for (text, expected) in [
        ("Err \"e\" |! (e => e ++ \"!\")", Rebuild::Wrap),
        ("Err \"e\" |! (e => Err (e ++ \"!\"))", Rebuild::Direct),
        ("Just \"a\" |: (s => s ++ \"b\")", Rebuild::Wrap),
    ] {
        let (_, typed, errors) = super::check(text, &Declarations::new());
        assert_eq!(errors, [], "{text}");
        assert!(
            typed.rebuilds.values().any(|v| *v == expected),
            "{text}: {:?}",
            typed.rebuilds
        );
    }
    assert_eq!(ty("Just \"a\" |: (s => s ++ \"b\")"), "Maybe<String>");
}
