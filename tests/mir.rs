use fipe::{
    check, compile, decl::Declarations, ir, lex::lex, mir, parse::parse,
    value::Value,
};

/// Sources every row and the walk share: the straight line.
const STRAIGHT_LINES: &[&str] = &[
    "16",
    "[1, 2]",
    "(1, \"a\")",
    "if 1 == 1 { 2 } else { 3 }",
    "with cwd /tmp { 1 }",
    "{\n    x = 1\n    x\n}",
    "Compile 1",
    "f = (w = 80) (c = (w / 8)) => c\nf 16",
    "add2 = x => x + 2\nadd2 4",
    "pair = a b => (a, b)\npair --b 2",
    "g: Vec<Integer> -> Integer = xs => len xs\ng 1 2 3",
    "id = len\nid [1]",
    "Just",
    "outer = x => (y => (x, y))\nouter 1",
    "p = Point 1 2\np.x\ntype Point { x: Integer, y: Integer }",
    "2.5",
    "x: F32 = 1.5\nx",
    "n = 9223372036854775808\nn",
    "\"text\"",
    "(x => x + 2) 2",
    "k = 1\n(k, k)",
    "Just 1 |: (_ => Just 2)",
    "Just [1, 2] |: map (n => n + 1)",
    "Ok 1 |! (e => 3)",
    "Nothing |! 1",
    "Nothing |! Just 2",
    "Just 1 || Nothing",
    "Just 1 || 2",
    "Just 1 && Just 2",
    "Just 1 && 2",
    "(1 == 1) || (2 == 2)",
    "(1 == 1) && (2 == 2)",
    "{\n    [:v] = Just 1\n    v\n}",
    "{\n    [:v] = Ok 1\n    Just v\n}",
    "{\n    Just x = Just 1\n    x + 1\n}",
    "Just x = Just 1",
    "1 | to<U8>",
    "1 | try<U8>",
    "\"3\" | read<I64>",
    "1 < 2",
    "Nothing",
    "g: Integer -> F64 = n => n\ng 1",
    "f = n => {\n    x: U8 = n\n    x\n}\nf 1",
    "p: Integer | String = 1",
    "type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | to<Box>",
    "type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | try<Box>",
    "type Box: TryFrom<Integer, String> { value: Integer }\nimpl Box: TryFrom<Integer, String> { try_from = n => Ok (Self n) }\n1 | to<Box>",
];

fn program(source: &str) -> (ir::Unit, mir::Program) {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let program = compile::compile(&unit);
    (unit, program)
}

#[test]
fn a_value_is_sixteen_bytes() {
    assert_eq!(core::mem::size_of::<Value>(), 16);
}

#[test]
fn an_operand_is_sixteen_bytes() {
    assert_eq!(core::mem::size_of::<mir::Operand>(), 16);
}

#[path = "mir/calls.rs"]
mod calls;
#[path = "mir/examples.rs"]
mod examples;
#[path = "mir/names.rs"]
mod names;
#[path = "mir/outcomes.rs"]
mod outcomes;
#[path = "mir/patterns.rs"]
mod patterns;
#[path = "mir/walk.rs"]
mod walk;
