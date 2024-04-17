use dlang::{
    bytecode::compiler::Compiler, lexer::Lexer, parser::Parser, test::Tests,
};

#[test]
fn test_bytecode_compiler_integer() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("5 + 5 + 5 + 5 - 10", 10));

    todo!()
}

#[test]
fn test_bytecode_compiler_output() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("5 + 5 + 5 + 5 - 10", 10));

    for test in tests.cases {
        let lexer = Lexer::new(test.input);
        let program = Parser::new(lexer).parse().unwrap();

        let mut comp = Compiler::new();
        comp.compile(program);
        let bytecode = comp.bytecode();

        println!("{}", bytecode.to_string());
    }
}
