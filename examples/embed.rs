//! Compile and run without granting any host capabilities.
use fipe::{
    check, compile, decl::Declarations, embed::Outcome, lex::lex, parse::parse,
    value::Value, vm::Machine,
};

fn main() {
    let module =
        parse(lex("[1, 2, 3] | map (n => n * 2) | fold 0 (a b => a + b)"))
            .expect("valid syntax");
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .expect("valid types and limits");
    let program = compile::compile(&unit);
    let mut machine = Machine::new(Some(10_000));
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("the pure program should finish within its budget");
    };
    assert_eq!(value, Value::Int(12));
    println!("{value:?}");
}
