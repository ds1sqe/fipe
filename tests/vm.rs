use dlang::{
    bytecode_vm::{bytecode::compiler::Compiler, vm::VM},
    lexer::Lexer,
    object::{Array, Bool, Int, Object},
    parser::Parser,
    test::Tests,
};

fn run_vm_test(tests: Tests<Option<Object>>) {
    for (idx, test) in tests.cases.iter().enumerate() {
        println!("Testing {:03}", idx);
        println!("Input: {}", test.input);
        println!("expect: {:?}", test.expect);

        let lexer = Lexer::new(test.input.clone());
        let program = Parser::new(lexer).parse().unwrap();

        let mut comp = Compiler::create().unwrap();
        if let Err(e) = comp.compile(program) {
            panic!("Compile error {:?}", e);
        }
        let bytecode = comp.bytecode().unwrap();

        println!("Bytecode\n{}", bytecode.to_string());

        let mut vm = VM::new(bytecode);

        while vm.is_runable() {
            if let Err(err) = vm.run_single() {
                panic!("VmError {:?}", err)
            }
        }
        println!("VM STACK:\n {}", vm.stack_to_string());

        let rst = vm.last_pop().as_ref();

        match rst {
            Some(obj) => {
                if test.expect.is_none() {
                    panic!(
                        "Result is not a None: result is {:?}, but expected was {:?}",
                        obj, test.expect
                    )
                }
                assert_eq!(obj, test.expect.as_ref().unwrap());
            }
            None => {
                if test.expect.is_some() {
                    panic!(
                        "Result is None: result is {:?}, but expected was {:?}",
                        rst, test.expect
                    )
                }
            }
        }
    }
}

#[test]
fn test_vm_integer_operation() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add(("5", Some(Object::Int(Int { value: 5 }))));
    tests.add(("10", Some(Object::Int(Int { value: 10 }))));
    tests.add(("-5", Some(Object::Int(Int { value: -5 }))));
    tests.add(("-10", Some(Object::Int(Int { value: -10 }))));
    tests.add(("5 + 5 + 5 + 5 - 10", Some(Object::Int(Int { value: 10 }))));
    tests.add(("2 * 2 * 2 * 2 * 2", Some(Object::Int(Int { value: 32 }))));
    tests.add(("-50 + 100 + -50", Some(Object::Int(Int { value: 0 }))));
    tests.add(("5 * 2 + 10", Some(Object::Int(Int { value: 20 }))));
    tests.add(("5 + 2 * 10", Some(Object::Int(Int { value: 25 }))));
    tests.add(("20 + 2 * -10", Some(Object::Int(Int { value: 0 }))));
    tests.add(("50 / 2 * 2 + 10", Some(Object::Int(Int { value: 60 }))));
    tests.add(("2 * (5 + 10)", Some(Object::Int(Int { value: 30 }))));
    tests.add(("3 * 3 * 3 + 10", Some(Object::Int(Int { value: 37 }))));
    tests.add(("3 * (3 * 3) + 10", Some(Object::Int(Int { value: 37 }))));
    tests.add((
        "(5 + 10 * 2 + 15 / 3) * 2 + -10",
        Some(Object::Int(Int { value: 50 })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_bool_operation() {
    let mut tests: Tests<Option<Object>> = Tests::new();
    tests.add(("true", Some(Object::Bool(Bool { value: true }))));
    tests.add(("false", Some(Object::Bool(Bool { value: false }))));

    tests.add(("1 < 2", Some(Object::Bool(Bool { value: true }))));
    tests.add(("1 > 2", Some(Object::Bool(Bool { value: false }))));
    tests.add(("1 < 1", Some(Object::Bool(Bool { value: false }))));
    tests.add(("1 > 1", Some(Object::Bool(Bool { value: false }))));

    tests.add(("1 <= 2", Some(Object::Bool(Bool { value: true }))));
    tests.add(("1 >= 2", Some(Object::Bool(Bool { value: false }))));
    tests.add(("1 <= 1", Some(Object::Bool(Bool { value: true }))));
    tests.add(("1 >= 1", Some(Object::Bool(Bool { value: true }))));

    tests.add(("1 == 1", Some(Object::Bool(Bool { value: true }))));
    tests.add(("1 != 1", Some(Object::Bool(Bool { value: false }))));
    tests.add(("1 == 2", Some(Object::Bool(Bool { value: false }))));
    tests.add(("1 != 2", Some(Object::Bool(Bool { value: true }))));

    tests.add(("true == true", Some(Object::Bool(Bool { value: true }))));
    tests.add(("false == false", Some(Object::Bool(Bool { value: true }))));
    tests.add(("true == false", Some(Object::Bool(Bool { value: false }))));
    tests.add(("true != false", Some(Object::Bool(Bool { value: true }))));
    tests.add(("false != true", Some(Object::Bool(Bool { value: true }))));

    tests.add(("(1 < 2) == true", Some(Object::Bool(Bool { value: true }))));
    tests.add((
        "(1 < 2) == false",
        Some(Object::Bool(Bool { value: false })),
    ));
    tests.add(("(1 > 2) == true", Some(Object::Bool(Bool { value: false }))));
    tests.add(("(1 > 2) == false", Some(Object::Bool(Bool { value: true }))));

    tests.add(("!true", Some(Object::Bool(Bool { value: false }))));
    tests.add(("!false", Some(Object::Bool(Bool { value: true }))));
    tests.add(("!!true", Some(Object::Bool(Bool { value: true }))));
    tests.add(("!!false", Some(Object::Bool(Bool { value: false }))));

    tests.add((
        "\"Hello\"==\"Hello\"",
        Some(Object::Bool(Bool { value: true })),
    ));
    tests.add((
        "\"Hello\"==\"World\"",
        Some(Object::Bool(Bool { value: false })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_jump_operation() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "if (true) { 10 } else { 20 }",
        Some(Object::Int(Int { value: 10 })),
    ));
    tests.add((
        "if (false) { 10 } else { 20 }",
        Some(Object::Int(Int { value: 20 })),
    ));
    tests.add(("if (true) { 10 }", Some(Object::Int(Int { value: 10 }))));
    tests.add(("if (false) { 10 }", None));
    tests.add(("if (10<20) { 1 }", Some(Object::Int(Int { value: 1 }))));
    tests.add(("if (10<=20) { 2 }", Some(Object::Int(Int { value: 2 }))));
    tests.add(("if (10>20) { 3 }", None));
    tests.add(("if (10>=20) { 4 }", None));
    tests.add(("if (10==10) { 5 }", Some(Object::Int(Int { value: 5 }))));
    tests.add(("if (10!=10) { 6 }", None));
    tests.add((
        "if ( if ( 20 > 0 ) {true} else { false }) {
            if ( 30 > 100) { 200 } else { 300 } 
         } else {
            if ( 20 > 10 ) { -200 } else { 100 }
         }",
        Some(Object::Int(Int { value: 300 })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_let_stm_operation() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add(("let foo = 5; foo * 5", Some(Object::Int(Int { value: 25 }))));
    tests.add((
        "let foo = 5; let bar = 5; bar * foo * 5",
        Some(Object::Int(Int { value: 125 })),
    ));
    tests.add((
        "let foo = 5; let bar = 5; let some_val = bar * foo * 5; some_val",
        Some(Object::Int(Int { value: 125 })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_array_creation() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "[1,2,3,4,5]",
        Some(Object::Array(Array {
            elements: vec![
                Object::Int(Int { value: 1 }),
                Object::Int(Int { value: 2 }),
                Object::Int(Int { value: 3 }),
                Object::Int(Int { value: 4 }),
                Object::Int(Int { value: 5 }),
            ],
        })),
    ));
    tests.add((
        "[10-2,20-2,30-2,40-2,50-2]",
        Some(Object::Array(Array {
            elements: vec![
                Object::Int(Int { value: 8 }),
                Object::Int(Int { value: 18 }),
                Object::Int(Int { value: 28 }),
                Object::Int(Int { value: 38 }),
                Object::Int(Int { value: 48 }),
            ],
        })),
    ));
    tests.add((
        "[1 * 2,2 * 3,3*4,4*5,5*6]",
        Some(Object::Array(Array {
            elements: vec![
                Object::Int(Int { value: 2 }),
                Object::Int(Int { value: 6 }),
                Object::Int(Int { value: 12 }),
                Object::Int(Int { value: 20 }),
                Object::Int(Int { value: 30 }),
            ],
        })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_array_index() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "let arr = [1,2,3,4,5];\narr[3]",
        Some(Object::Int(Int { value: 4 })),
    ));
    tests.add((
        "[10-2,20-2,30-2,40-2,50-2][2]",
        Some(Object::Int(Int { value: 28 })),
    ));
    tests.add((
        "[10-2,20-2,30-2,40-2,50-2][2+1]",
        Some(Object::Int(Int { value: 38 })),
    ));
    tests.add((
        "[1 * 2,2 * 3,3*4,4*5,5*6][0+2]",
        Some(Object::Int(Int { value: 12 })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_function_not_closure() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "
let no_return = fn() { };no_return() no_return() no_return() no_return()
",
        None,
    ));

    tests.add((
        "
let fun = fn() { 10 + 20 }; fun()
",
        Some(Object::Int(Int { value: 30 })),
    ));
    tests.add((
        "
let ten = fn() { 5 + 5 }; let five = fn() { 2 + 3 }; ten() + five()
",
        Some(Object::Int(Int { value: 15 })),
    ));
    tests.add((
        "
let ten = fn() { 5 + 5 }; let five = fn() { 2 + 3 }; ten() * five() * ten()
",
        Some(Object::Int(Int { value: 500 })),
    ));

    tests.add((
        "
let five = fn() { 2 + 3 };
let ten = fn() { 5 + 5 };
let fun = fn() { return five() + ten(); };
fun() * fun()
",
        Some(Object::Int(Int { value: 225 })),
    ));

    tests.add((
        "
let add = fn(a,b) { a + b }; add(10,20)
",
        Some(Object::Int(Int { value: 30 })),
    ));
    tests.add((
        "
let add = fn(a,b) { a + b }; add(add(10,20),add(30,40))
",
        Some(Object::Int(Int { value: 100 })),
    ));
    tests.add((
        "
let args = fn(a, b, c) { a; b; c };
args(24, 25, 26)
",
        Some(Object::Int(Int { value: 26 })),
    ));

    tests.add((
        "
fn local(a,b) { let value = 20; let foo = 40; return (a + b) * (value + foo); } local(3,7)
",
        Some(Object::Int(Int { value: 600 })),
    ));

    tests.add((
        "
let sum = fn(a, b) {
  let c = a + b;
  c
};
let outer = fn() {
  sum(1, 2) + sum(3, 4);
};
outer()
",
        Some(Object::Int(Int { value: 10 })),
    ));

    tests.add((
        "
let globalNum = 10;
let sum = fn(a, b) {
  let c = a + b;
  c + globalNum;
};
let outer = fn() {
  sum(1, 2) + sum(3, 4) + globalNum;
};
outer() + globalNum
",
        Some(Object::Int(Int { value: 50 })),
    ));

    run_vm_test(tests)
}
#[test]
fn test_vm_function_closure() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "
let new_adder = fn (a) { fn(b) {a+b}}; let adder = new_adder(1); adder(2)
",
        Some(Object::Int(Int { value: 3 })),
    ));

    tests.add((
        "
let new_adder = fn (a,b) { return fn(c) {a+b+c} }; let adder = new_adder(1,2); adder(3)
",
        Some(Object::Int(Int { value: 6 })),
    ));

    tests.add((
        "
let new_adder = fn (one,two) {
    let three = one + two;
    fn(four) {
        let seven = three + four;
        fn(six) { six + seven };
    }
};

let adder_1 = new_adder(1,2);
let adder_2 = adder_1(4);
let result = adder_2(6);
result


",
        Some(Object::Int(Int { value: 13 })),
    ));

    tests.add((
        "let new_closure = fn(a, b) {
let one = fn() { a; };
let two = fn() { b; };
fn() { one() + two(); };
};
let closure = new_closure(9, 90);
closure();",
        Some(Object::Int(Int { value: 99 })),
    ));

    tests.add((
        "
let a = 1;
let new_adder_outer = fn(b) {
    fn(c) {
        fn(d) { a + b + c + d }
    }
};
let new_adder_inner = new_adder_outer(2);
let adder = new_adder_inner(3);
adder(4);
",
        Some(Object::Int(Int {
            value: 1 + 2 + 3 + 4, //  10
        })),
    ));

    run_vm_test(tests)
}

#[test]
fn test_vm_function_recursive() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add((
        "
let count_down = fn(x) {
    if (x == 0) {
        return 0;
    } else {
        return count_down(x - 1);
    }
};
count_down(10);

",
        Some(Object::Int(Int { value: 0 })),
    ));

    tests.add((
        "
let count_down = fn(x) {
    if (x == 0) {
        return 0;
    } else {
        return count_down(x - 1);
    }
};

let wrapper = fn() {
    count_down(10)
};

wrapper()

",
        Some(Object::Int(Int { value: 0 })),
    ));

    tests.add((
        "


let wrapper = fn() {
    let count_down = fn(x) {
        if (x == 0) {
            return 0;
        } else {
            return count_down(x - 1);
        }
    };

    count_down(10)
};

wrapper()

",
        Some(Object::Int(Int { value: 0 })),
    ));

    run_vm_test(tests)
}
