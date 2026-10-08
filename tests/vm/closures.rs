use super::run::{session, shown};

#[test]
fn a_lambda_copies_its_captures_at_creation() {
    assert_eq!(shown("outer = x => (y => (x, y))\n(outer 1) 2"), "(1, 2)");
    assert_eq!(
        session(&["x = 1", "f = () => x", "x = 2", "f ()", "x"]),
        ["()", "()", "()", "1", "2"]
    );
}

#[test]
fn a_generic_body_dispatches_through_its_witness() {
    assert_eq!(shown("g = xs => xs | map (n => n + 1)\ng [1, 2]"), "[2, 3]");
    assert_eq!(shown("g = xs => xs | map (n => n + 1)\ng [1.5]"), "[2.5]");
    assert_eq!(
        shown("same = a b => a == b\n(same 1 1, same \"a\" \"b\")"),
        "(True, False)"
    );
}

#[test]
fn arms_switch_on_the_constructor() {
    let source = "m: Maybe<Integer> -> Integer = { Just v => v, Nothing => 0 }\n(m (Just 2), m Nothing)";
    assert_eq!(shown(source), "(2, 0)");
    assert_eq!(
        shown("f: Integer -> String = { 0 => \"a\", _ => \"b\" }\n(f 0, f 1)"),
        "(\"a\", \"b\")"
    );
    assert_eq!(
        shown(
            "k: Integer -> Integer = { x if x == 1 => 1, _ => 2 }\n(k 1, k 5)"
        ),
        "(1, 2)"
    );
    assert_eq!(
        shown("w: Vec<Integer> -> Integer = { [x, y] => x + y, [] => 0, _ => 1 }\n(w [1, 2], w [], w [1, 2, 3])"),
        "(3, 0, 1)"
    );
    assert_eq!(shown("(a, b) = (1, 2)\nb"), "2");
    assert_eq!(shown("[x, y] = [1, 2]\nx"), "1");
    assert_eq!(
        shown("type Point { x: Integer, y: Integer }\nq: Point -> Integer = { Point { x, .. } => x }\nq (Point 1 2)"),
        "1"
    );
    assert_eq!(
        shown("p: Integer | String -> Integer = { Integer n => n, String _ => 0 }\n(p 3, p \"x\")"),
        "(3, 0)"
    );
}

#[test]
fn a_gate_runs_its_limit_test() {
    let source = "type Even: Integer where self % 2 == 0\n\
                  m: Integer -> Integer = { Even n => n, _ => 0 }\n\
                  (m 4, m 3)";
    assert_eq!(shown(source), "(4, 0)");
}
