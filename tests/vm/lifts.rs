//! Pattern bindings on an outcome: a hit binds and runs the rest of the
//! body, a miss ends the body with the failure.
use super::run::{session, shown};

#[test]
fn the_implicit_main_is_a_body_a_miss_ends() {
    assert_eq!(shown("Just x = Nothing\nx + 1"), "Nothing");
    assert_eq!(shown("Just x = Just 3\nx + 1"), "Just 4");
    assert_eq!(shown("[:x] = Just 3\nx * 2"), "Just 6");
    assert_eq!(
        shown("r: Result<Integer, String> = Err \"e\"\nOk x = r\nx + 1"),
        "Err \"e\""
    );
}

#[test]
fn a_lift_ends_a_function_body_with_the_failure() {
    let f = "f: Maybe<Integer> -> Maybe<Integer> = m => {\n    Just x = m\n    Just (x + 1)\n}\n";
    assert_eq!(
        shown(&format!("{f}(f Nothing, f (Just 1))")),
        "(Nothing, Just 2)"
    );
}

#[test]
fn nested_lifts_nest() {
    let add = "add: Maybe<Integer> -> Maybe<Integer> -> Maybe<Integer> = a b => {\n    Just x = a\n    Just y = b\n    x + y\n}\n";
    assert_eq!(
        shown(&format!(
            "{add}(add (Just 1) (Just 2), add (Just 1) Nothing, add Nothing (Just 2))"
        )),
        "(Just 3, Nothing, Nothing)"
    );
}

#[test]
fn a_lift_payload_splits_into_parts() {
    assert_eq!(shown("Just (a, b) = Just (3, 4)\na + b"), "Just 7");
}

#[test]
fn a_lines_pattern_names_serve_later_lines() {
    assert_eq!(session(&["(a, b) = (1, 2)", "a + b"]), ["()", "3"]);
    assert_eq!(session(&["Just x = Just 1", "x"]), ["Just ()", "1"]);
    assert_eq!(
        session(&["Just x = Just 1\ny = x + 1\ny", "y"]),
        ["Just 2", "2"]
    );
    assert_eq!(
        session(&[
            "type P { x: Integer, y: Integer }",
            "P { x, y } = P 1 2",
            "x + y"
        ]),
        ["()", "()", "3"]
    );
}

#[test]
fn grouped_lifts_preserve_hits_and_misses() {
    assert_eq!(shown("(Just x) = Just 3\nx + 1"), "Just 4");
    assert_eq!(shown("(Just x) = Nothing\nx + 1"), "Nothing");
    assert_eq!(shown("((Just (a, b))) = Just (3, 4)\na + b"), "Just 7");
    assert_eq!(
        shown(
            "n: Maybe<Integer * Integer> = Nothing\n((Just (a, b))) = n\na + b"
        ),
        "Nothing"
    );
}
