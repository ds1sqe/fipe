use dlang::{
    bytecode::compiler::Compiler, lexer::Lexer, object::Object, parser::Parser, test::Tests, vm::VM,
};

#[test]
fn test_vm_integer_operation() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("5", 5));
    tests.add(("10", 10));
    tests.add(("-5", -5));
    tests.add(("-10", -10));
    tests.add(("5 + 5 + 5 + 5 - 10", 10));
    tests.add(("2 * 2 * 2 * 2 * 2", 32));
    tests.add(("-50 + 100 + -50", 0));
    tests.add(("5 * 2 + 10", 20));
    tests.add(("5 + 2 * 10", 25));
    tests.add(("20 + 2 * -10", 0));
    tests.add(("50 / 2 * 2 + 10", 60));
    tests.add(("2 * (5 + 10)", 30));
    tests.add(("3 * 3 * 3 + 10", 37));
    tests.add(("3 * (3 * 3) + 10", 37));
    tests.add(("(5 + 10 * 2 + 15 / 3) * 2 + -10", 50));

    for (idx, test) in tests.cases.iter().enumerate() {
        println!("Testing {:03}", idx);
        println!("Input: {}", test.input);
        println!("expect: {}", test.expect);

        let lexer = Lexer::new(test.input.clone());
        let program = Parser::new(lexer).parse().unwrap();

        let mut comp = Compiler::new();
        comp.compile(program);
        let bytecode = comp.bytecode();

        println!("Bytecode\n{}", bytecode.to_string());

        let mut vm = VM::new(bytecode);

        while vm.is_runable() {
            vm.run_single();
        }

        let rst = vm.top().unwrap();
        match rst {
            Object::Int(int) => {
                assert!(int.value == test.expect)
            }
            not_int => {
                panic!("{:?} is not a int", not_int);
            }
        }
    }
}
