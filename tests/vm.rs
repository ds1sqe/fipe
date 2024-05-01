use dlang::{
    bytecode_vm::bytecode::compiler::Compiler,
    bytecode_vm::vm::VM,
    lexer::Lexer,
    object::{Array, Int, Object},
    parser::Parser,
    test::Tests,
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

        let rst = vm.top().as_ref().unwrap();
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

#[test]
fn test_vm_bool_operation() {
    let mut tests: Tests<bool> = Tests::new();
    tests.add(("true", true));
    tests.add(("false", false));

    tests.add(("1 < 2", true));
    tests.add(("1 > 2", false));
    tests.add(("1 < 1", false));
    tests.add(("1 > 1", false));

    tests.add(("1 <= 2", true));
    tests.add(("1 >= 2", false));
    tests.add(("1 <= 1", true));
    tests.add(("1 >= 1", true));

    tests.add(("1 == 1", true));
    tests.add(("1 != 1", false));
    tests.add(("1 == 2", false));
    tests.add(("1 != 2", true));

    tests.add(("true == true", true));
    tests.add(("false == false", true));
    tests.add(("true == false", false));
    tests.add(("true != false", true));
    tests.add(("false != true", true));

    tests.add(("(1 < 2) == true", true));
    tests.add(("(1 < 2) == false", false));
    tests.add(("(1 > 2) == true", false));
    tests.add(("(1 > 2) == false", true));

    tests.add(("!true", false));
    tests.add(("!false", true));
    tests.add(("!!true", true));
    tests.add(("!!false", false));

    tests.add(("\"Hello\"==\"Hello\"", true));
    tests.add(("\"Hello\"==\"World\"", false));

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

        let rst = vm.top().as_ref().unwrap();
        match rst {
            Object::Bool(obj) => {
                assert!(obj.value == test.expect)
            }
            not_bool => {
                panic!("{:?} is not a bool", not_bool);
            }
        }
    }
}

#[test]
fn test_vm_jump_operation() {
    let mut tests: Tests<Option<i64>> = Tests::new();

    tests.add(("if (true) { 10 } else { 20 }", Some(10)));
    tests.add(("if (false) { 10 } else { 20 }", Some(20)));
    tests.add(("if (true) { 10 }", Some(10)));
    tests.add(("if (false) { 10 }", None));
    tests.add(("if (10<20) { 1 }", Some(1)));
    tests.add(("if (10<=20) { 2 }", Some(2)));
    tests.add(("if (10>20) { 3 }", None));
    tests.add(("if (10>=20) { 4 }", None));
    tests.add(("if (10==10) { 5 }", Some(5)));
    tests.add(("if (10!=10) { 6 }", None));
    tests.add((
        "if ( if ( 20 > 0 ) {true} else { false }) {
            if ( 30 > 100) { 200 } else { 300 } 
         } else {
            if ( 20 > 10 ) { -200 } else { 100 }
         }",
        Some(300),
    ));

    for (idx, test) in tests.cases.iter().enumerate() {
        println!("Testing {:03}", idx);
        println!("Input: {}", test.input);
        println!("expect: {:?}", test.expect);

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

        println!("{}", vm.stack_to_string());

        let rst = vm.top();
        match rst {
            Some(obj) => match obj {
                Object::Int(int) => {
                    assert!(int.value == test.expect.unwrap())
                }
                not_int => {
                    panic!("{:?} is not a int", not_int);
                }
            },
            None => {
                assert!(test.expect.is_none())
            }
        }
    }
}

#[test]
fn test_vm_let_stm_operation() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("let foo = 5; foo * 5", 25));

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

        let rst = vm.top().as_ref().unwrap();
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

#[test]
fn test_vm_array_creation() {
    let mut tests: Tests<Option<Array>> = Tests::new();

    tests.add((
        "[1,2,3,4,5]",
        Some(Array {
            elements: vec![
                Object::Int(Int { value: 1 }),
                Object::Int(Int { value: 2 }),
                Object::Int(Int { value: 3 }),
                Object::Int(Int { value: 4 }),
                Object::Int(Int { value: 5 }),
            ],
        }),
    ));
    tests.add((
        "[10-2,20-2,30-2,40-2,50-2]",
        Some(Array {
            elements: vec![
                Object::Int(Int { value: 8 }),
                Object::Int(Int { value: 18 }),
                Object::Int(Int { value: 28 }),
                Object::Int(Int { value: 38 }),
                Object::Int(Int { value: 48 }),
            ],
        }),
    ));
    tests.add((
        "[1 * 2,2 * 3,3*4,4*5,5*6]",
        Some(Array {
            elements: vec![
                Object::Int(Int { value: 2 }),
                Object::Int(Int { value: 6 }),
                Object::Int(Int { value: 12 }),
                Object::Int(Int { value: 20 }),
                Object::Int(Int { value: 30 }),
            ],
        }),
    ));

    for (idx, test) in tests.cases.iter().enumerate() {
        println!("Testing {:03}", idx);
        println!("Input: {}", test.input);
        println!("expect: {:?}", test.expect);

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

        println!("VM STACK:\n {}", vm.stack_to_string());

        let rst = vm.top().as_ref().unwrap();
        match rst {
            Object::Array(arr) => {
                assert!(*arr == test.expect.clone().unwrap())
            }
            not_int => {
                panic!("{:?} is not a int", not_int);
            }
        }
    }
}

#[test]
fn test_vm_array_index() {
    let mut tests: Tests<i64> = Tests::new();

    tests.add(("let arr = [1,2,3,4,5];\narr[3]", 4));
    tests.add(("[10-2,20-2,30-2,40-2,50-2][2]", 28));
    tests.add(("[10-2,20-2,30-2,40-2,50-2][2+1]", 38));
    tests.add(("[1 * 2,2 * 3,3*4,4*5,5*6][0+2]", 12));

    for (idx, test) in tests.cases.iter().enumerate() {
        println!("Testing {:03}", idx);
        println!("Input: {}", test.input);
        println!("expect: {:?}", test.expect);

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

        println!("VM STACK:\n {}", vm.stack_to_string());

        let rst = vm.top().as_ref().unwrap();
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
