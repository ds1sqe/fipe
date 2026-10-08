use fipe::{
    check, compile, decl::Declarations, ir, lex::lex, mir, parse::parse,
    value::Value,
};

/// Sources every row and the walk share: the straight line of 08.1.
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
#[path = "mir/walk.rs"]
mod walk;
