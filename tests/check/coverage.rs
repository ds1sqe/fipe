use super::*;

#[test]
fn coverage_names_missing_constructors_and_ignores_guards() {
    for (text, missing) in [
        ("f = { Just x => x }", "Nothing"),
        ("f = { True => 1 }", "False"),
        ("f = { 0 => 1, 1 => 2 }", "_"),
        ("f = { x if x > 0 => x }", "_"),
        ("f = { (True, True) => 1, (False, False) => 2 }", "False"),
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(errors.iter().any(|e| matches!(&e.reason,
            fipe::infer::Reason::Limit { message } if message.contains("uncovered") && message.contains(missing))), "{text}: {errors:?}");
    }
}

#[test]
fn coverage_specializes_complete_rows() {
    for text in [
        "f = { Just True => 1, Just False => 2, Nothing => 3 }",
        "f = { (True, _) => 1, (False, True) => 2, (False, False) => 3 }",
        "max = x y if x > y => x, _ y => y",
        "f: Vec<Integer> -> Integer = { [x] => x, _ => 0 }",
        "type Pair { b: Bool, n: Integer }\nf = { Pair { b: True, .. } => 1, Pair { b: False, .. } => 0 }",
        "id: Integer | String = 3\nid | { Integer n => n.show, String s => s }",
        "type Small: Integer where self < 10\ntype Other: Integer & !Small\nf = { Small n => n, Other n => n }",
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
    }
}

#[test]
fn gates_preserve_other_head_columns() {
    let text = "type Small: Integer where self < 10\ntype Other: Integer & !Small\nf = { Small n True => 1, Other n False => 2 }";
    let (_, _, errors) = check(text, &Declarations::new());
    assert_ne!(errors, []);
}

#[test]
fn bindings_require_total_patterns_or_good_sides() {
    for text in ["v: Vec<Integer> = [1]\n[x] = v", "Just 0 = Just 1"] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(!errors.is_empty(), "{text}");
    }
    for text in [
        "[x, y] = [1, 2]",
        "(a, b) = (1, True)",
        "Just x = Just 1",
        "Ok x = Ok 1",
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(errors.is_empty(), "{text}: {errors:?}");
    }
}

#[test]
fn main_is_lifted_by_good_side_bindings() {
    let (_, typed, errors) = check("Just x = Just 1\nx", &Declarations::new());
    assert_eq!(errors, []);
    assert_eq!(typed.main.unwrap().to_string(), "Maybe<Integer>");
    let (_, typed, errors) =
        check("Just x = Just (1, True)\nx", &Declarations::new());
    assert_eq!(errors, []);
    assert_eq!(typed.main.unwrap().to_string(), "Maybe<Integer * Bool>");
}

#[test]
fn complementary_gates_cover_finite_meanings() {
    let text = "type Yes: Bool where self\ntype No: Bool & !Yes\nf = { Yes b => 1, No b => 0 }";
    let (_, _, errors) = check(text, &Declarations::new());
    assert_eq!(errors, []);
}

#[test]
fn record_fields_cover_their_limited_meaning() {
    let text = "type Flag: Bool & Bits<1>\ntype P { bit: Flag }\nf = { P { bit: True } => 1, P { bit: False } => 0 }";
    let (_, _, errors) = check(text, &Declarations::new());
    assert_eq!(errors, []);
}

#[test]
fn written_constructors_require_total_arms() {
    for (source, total) in [
        ("type Flag { b: Bool }\nimpl Flag { True => Self { b: True } }", false),
        ("type Flag { b: Bool }\nimpl Flag { True => Self { b: True }, False => Self { b: False } }", true),
    ] {
        let (_, _, errors) = check(source, &Declarations::new());
        assert_eq!(errors.is_empty(), total, "{source}: {errors:?}");
        if !total {
            assert!(errors.iter().any(|e| matches!(&e.reason,
                fipe::infer::Reason::Limit { message } if message.contains("uncovered False"))), "{errors:?}");
        }
    }
}
