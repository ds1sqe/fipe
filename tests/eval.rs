use dlang::{
    ast::Nodetrait,
    heap::Heap,
    lexer,
    object::{environment::Environment, Int, Object, ObjectTrait, ObjectType},
    parser,
    test::Tests,
    token::Kind,
    treewalker::eval::{
        errors::{ArgumentsLength, EvalError},
        evaluate,
    },
};

fn test_eval(input: String) -> Result<Option<Object>, EvalError> {
    let lex = lexer::Lexer::new(input);

    let mut lex_cl = lex.clone();
    loop {
        let tok = lex_cl.next_token();
        if tok.kind == Kind::EOF {
            break;
        }
    }
    let mut parser = parser::Parser::new(lex);
    let prog = parser.parse().unwrap();

    let mut heap = Heap::new();
    let mut env = Environment::new();

    evaluate(prog.to_node(), &mut heap, &mut env)
}

fn test_integer_object_with_result(
    idx: usize,
    expect: i64,
    res: Result<Option<Object>, EvalError>,
) {
    let res = res.unwrap().unwrap();

    match res {
        Object::Int(obj) => {
            if obj.value != expect {
                panic!("[{idx}]: {:?} not matched with {expect}", obj.value);
            }
        }
        any => {
            panic!("[{idx}]:{:?} is not a Int", any);
        }
    }
}

#[test]
fn test_eval_integer_expression() {
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
        let res = test_eval(test.input.clone());
        test_integer_object_with_result(idx, test.expect, res);
    }
}

fn test_bool_object_with_result(
    idx: usize,
    expect: bool,
    res: Result<Option<Object>, EvalError>,
) {
    let res = res.unwrap().unwrap();

    match res {
        Object::Bool(obj) => {
            if obj.value != expect {
                panic!("[{idx}]: {:?} not matched with {expect}", obj.value);
            }
        }
        any => {
            panic!("[{idx}]:{:?} is not a Bool", any);
        }
    }
}

#[test]
fn test_eval_bool_expression() {
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
        let res = test_eval(test.input.clone());
        test_bool_object_with_result(idx, test.expect, res);
    }
}

fn test_string_object_with_result(
    idx: usize,
    expect: String,
    res: Result<Option<Object>, EvalError>,
) {
    let res = res.unwrap().unwrap();

    match res {
        Object::String(obj) => {
            if obj.value != expect {
                panic!("[{idx}]: {:?} not matched with {expect}", obj.value);
            }
        }
        any => {
            panic!("[{idx}]:{:?} is not a Bool", any);
        }
    }
}

#[test]
fn test_eval_string_expression() {
    let mut tests: Tests<String> = Tests::new();
    tests.add(("\"foo\"", "foo".to_string()));
    tests.add(("\"bar\"", "bar".to_string()));
    tests.add(("\"foo\" + \" \" + \"bar\"", "foo bar".to_string()));

    for (idx, test) in tests.cases.iter().enumerate() {
        let res = test_eval(test.input.clone());
        test_string_object_with_result(idx, test.expect.clone(), res);
    }
}

fn test_if_object_with_result(
    idx: usize,
    expect: Option<Object>,
    res: Result<Option<Object>, EvalError>,
) {
    let obj = res.unwrap();

    if obj.is_none() != expect.is_none() {
        panic!("[{idx}]: {:?} not matched with {:?}", obj, expect);
    }
    if obj.is_some() {
        let obj = obj.unwrap();
        let expect = expect.unwrap();
        if obj.get_type() != expect.get_type() {
            panic!("[{idx}]: {:?} not matched with {:?}", obj, expect);
        }
        if obj.to_str() != expect.to_str() {
            panic!("[{idx}]: {:?} not matched with {:?}", obj, expect);
        }
    }
}

#[test]
fn test_eval_if_expression() {
    let mut tests: Tests<Option<Object>> = Tests::new();

    tests.add(("if (true) { 10 }", Some(Object::Int(Int { value: 10 }))));
    tests.add(("if (false) { 10 }", None));
    tests.add(("if (1<2) {10}", Some(Object::Int(Int { value: 10 }))));
    tests.add(("if (1>2) {10}", None));
    tests.add((
        "if (1>2) {10} else {20}",
        Some(Object::Int(Int { value: 20 })),
    ));
    tests.add((
        "if (1<2) {10} else {20}",
        Some(Object::Int(Int { value: 10 })),
    ));

    for (idx, test) in tests.cases.iter().enumerate() {
        let res = test_eval(test.input.clone());
        test_if_object_with_result(idx, test.expect.clone(), res);
    }
}

fn test_error_with_result(
    idx: usize,
    expect: EvalError,
    res: Result<Option<Object>, EvalError>,
) {
    let err = res.err().unwrap();

    if err != expect {
        panic!("[{idx}]: {:?} not matched with {:?}", err, expect);
    }
}

#[test]
fn test_eval_errors() {
    let mut tests: Tests<EvalError> = Tests::new();

    tests.add(("let foo;", EvalError::LetStatementValueIsNone));
    tests.add(("let bar;", EvalError::LetStatementValueIsNone));

    tests.add((
        "if (1) {true}",
        EvalError::NotABoolean(Object::Int(Int { value: 1 })),
    ));

    tests.add((
        "if (0) {true}",
        EvalError::NotABoolean(Object::Int(Int { value: 0 })),
    ));

    tests.add(("foo", EvalError::IdentifierNotFound("foo".to_string())));

    tests.add(("5 + false", EvalError::NotSameType));
    tests.add(("10 - true", EvalError::NotSameType));
    tests.add(("10 + \"hello\"", EvalError::NotSameType));

    tests.add((
        "fn (x,y,z) { x + y + z}(1,2)",
        EvalError::FunctionArgLengthNotMatched(ArgumentsLength {
            function_args: 3,
            called_with: 2,
        }),
    ));

    tests.add((
        "fn (x,y) { x + y }(1,2,3)",
        EvalError::FunctionArgLengthNotMatched(ArgumentsLength {
            function_args: 2,
            called_with: 3,
        }),
    ));

    tests.add(("100/0", EvalError::DivideWithZero));
    tests.add(("0/0", EvalError::DivideWithZero));

    tests.add((
        "!\"Hello\"",
        EvalError::InvalidPrefixOperationTarget(ObjectType::String, Kind::Bang),
    ));
    tests.add((
        "-\"World\"",
        EvalError::InvalidPrefixOperationTarget(
            ObjectType::String,
            Kind::Minus,
        ),
    ));

    tests.add((
        "\"Hello\"-\"World\"",
        EvalError::InvalidStringInfixOperation(Kind::Minus),
    ));
    tests.add((
        "\"Hello\"%\"World\"",
        EvalError::InvalidStringInfixOperation(Kind::Mod),
    ));
    tests.add((
        "\"Hello\"/\"World\"",
        EvalError::InvalidStringInfixOperation(Kind::Divide),
    ));
    tests.add((
        "\"Hello\"*\"World\"",
        EvalError::InvalidStringInfixOperation(Kind::Product),
    ));

    tests.add(("-true", EvalError::InvalidBoolPrefixOperation(Kind::Minus)));
    tests.add(("-false", EvalError::InvalidBoolPrefixOperation(Kind::Minus)));

    tests.add((
        "true-true",
        EvalError::InvalidBoolInfixOperation(Kind::Minus),
    ));
    tests.add((
        "true+true",
        EvalError::InvalidBoolInfixOperation(Kind::Plus),
    ));
    tests.add((
        "true/false",
        EvalError::InvalidBoolInfixOperation(Kind::Divide),
    ));
    tests.add((
        "false%false",
        EvalError::InvalidBoolInfixOperation(Kind::Mod),
    ));

    for (idx, test) in tests.cases.iter().enumerate() {
        let res = test_eval(test.input.clone());
        test_error_with_result(idx, test.expect.clone(), res);
    }
}

fn test_let_stm_with_result(
    idx: usize,
    expect: Object,
    res: Result<Option<Object>, EvalError>,
) {
    let obj = res.unwrap().unwrap();

    if obj.to_str() != expect.to_str() {
        panic!("[{idx}]: {:?} not matched with {:?}", obj, expect);
    }
}

#[test]
fn test_let_expression() {
    let mut tests: Tests<Object> = Tests::new();

    tests.add(("let foo = 100; foo", Object::Int(Int { value: 100 })));

    tests.add(("let foo = 10 * 10; foo", Object::Int(Int { value: 100 })));

    tests.add((
        "let a = 100; let b = a + 1; let c = b + 20; a+b+c",
        Object::Int(Int { value: 322 }),
    ));

    for (idx, test) in tests.cases.iter().enumerate() {
        let res = test_eval(test.input.clone());
        test_let_stm_with_result(idx, test.expect.clone(), res);
    }
}

fn test_function_with_result(
    idx: usize,
    expect: Object,
    res: Result<Option<Object>, EvalError>,
) {
    let obj = res.unwrap().unwrap();

    if obj.to_str() != expect.to_str() {
        panic!("[{idx}]: {:?} not matched with {:?}", obj, expect);
    }
}

#[test]
fn test_eval_function() {
    let mut tests: Tests<Object> = Tests::new();

    tests.add((
        "let func = fn (x) { x }; func(5)",
        Object::Int(Int { value: 5 }),
    ));

    tests.add((
        "let func = fn (x) { return x; }; func(5)",
        Object::Int(Int { value: 5 }),
    ));

    tests.add(("fn func(x) { x } \n func(5)", Object::Int(Int { value: 5 })));

    tests.add((
        "fn func(x) { return x; } \n func(5)",
        Object::Int(Int { value: 5 }),
    ));

    tests.add((
        "fn add(x,y) { return x+y; } \n add(5,10)",
        Object::Int(Int { value: 15 }),
    ));

    tests.add((
        "fn add(x,y) { return x+y; } \n add(add(5,10),add(15,20))",
        Object::Int(Int { value: 50 }),
    ));

    tests.add(("fn (x,y) { x*y }(5,10)", Object::Int(Int { value: 50 })));

    tests.add((
        "fn product(x,y) { x*y }(5,10)",
        Object::Int(Int { value: 50 }),
    ));

    // tests.add((
    //     "
    //     let createAdder = fn (x) {
    //         let adder = fn (y) { return y + x;};
    //         return adder;
    //     };
    //
    //     let add_ten = createAdder(10);
    //
    //     add_ten(10)
    //     ",
    //     Object::Int(Int { value: 20 }),
    // )); // This test will failed until i fix weak reference problem

    for (idx, test) in tests.cases.iter().enumerate() {
        let res = test_eval(test.input.clone());
        dbg!(&res);
        test_function_with_result(idx, test.expect.clone(), res);
    }
}
