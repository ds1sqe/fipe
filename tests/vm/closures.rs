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
fn a_generic_local_lambda_takes_its_witnesses_at_each_use() {
    // The local binding generalizes over `Literal a`; each use supplies
    // the witness, as a definition's use does.
    assert_eq!(
        shown("f = s => { body = n x => n + 1\n s | lines | fold 0 body }\nf \"a\\nb\""),
        "2"
    );
    assert_eq!(
        shown("f = s => { inc = n => n + 1\n (inc 1, inc 1.5) }\nf ()"),
        "(2, 2.5)"
    );
    // A nested use captures what the lambda captures.
    assert_eq!(
        shown(
            "f = k => { add = n => n + k\n [1, 2] | map (x => add x) }\nf 10"
        ),
        "[11, 12]"
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

#[test]
fn a_pipe_into_a_function_value_applies_it_to_the_input() {
    assert_eq!(shown("3 | { 3 => \"a\", _ => \"b\" }"), "\"a\"");
    assert_eq!(shown("4 | { 3 => \"a\", _ => \"b\" }"), "\"b\"");
    assert_eq!(shown("3 | (y => y + 1)"), "4");
    assert_eq!(shown("inc = y => y + 1\n3 | (inc)"), "4");
    assert_eq!(shown("2 | (y => y * 10) | (y => y + 1)"), "21");
    assert_eq!(
        shown("f = n => n | { 0 => 1, k => k * 2 }\n(f 0, f 5)"),
        "(1, 10)"
    );
    assert_eq!(
        session(&["g = y => (y, y)", "1 | (if True { g } else { g })"]),
        ["()", "(1, 1)"]
    );
}

#[test]
fn a_generic_value_definition_takes_its_use_s_witnesses() {
    assert_eq!(
        shown("inc = x => x + 1\ng = inc\n(g 1, g 1.5, [1] | map g)"),
        "(2, 2.5, [2])"
    );
    assert_eq!(
        session(&["inc = x => x + 1", "h = Compile (inc)", "h 2", "h 2.5"]),
        ["()", "()", "3", "3.5"]
    );
}

#[test]
fn a_packed_binding_is_told_apart_by_the_gate_pattern() {
    let source = "id: Integer | String = 3\n\
                  id | { Integer n => n.show, String s => s }";
    assert_eq!(shown(source), "\"3\"");
    let source = "id: Integer | String = \"x\"\n\
                  id | { Integer n => n.show, String s => s }";
    assert_eq!(shown(source), "\"x\"");
}

#[test]
fn a_piped_value_replaces_a_field_default() {
    let source = "type P { x: Integer = 300 }\np = 1 | P\nx: U8 = p.x\nx";
    assert_eq!(shown(source), "1");
}

#[test]
fn operands_past_the_spots_apply_the_returned_function() {
    let mk = "mk = c => (x => x + c)";
    assert_eq!(
        session(&[mk, "k = n => mk 1 n", "k 2", "j = n => (mk 1) n", "j 2"]),
        ["()", "()", "3", "()", "3"]
    );
    assert_eq!(shown(&format!("{mk}\nk = n => mk n 1\nk 2")), "3");
    let a = "a = x => (y => (z => x + y + z))";
    assert_eq!(shown(&format!("{a}\na 1 2 3")), "6");
    assert_eq!(shown(&format!("{a}\nb = y => a 1 y 3\nb 2")), "6");
    let g = "g = x y => (z => x + y + z)";
    assert_eq!(shown(&format!("{g}\nr = n => g 1 2 n\nr 3")), "6");
    // A named function returning a partial call.
    let part = "part = n => mk n\nq = n => part 1 n\nq 2";
    assert_eq!(shown(&format!("{mk}\n{part}")), "3");
    // A pipe into a stage whose call is over-applied.
    assert_eq!(
        shown(&format!("{mk}\np = n => n | (v => mk 1 v)\np 5")),
        "6"
    );
}
