use dlang::{
    ast::{ArrayLiteral, Nodetrait},
    lexer::Lexer,
    parser::Parser,
};

#[test]
fn test_let_statement() {
    let mut inputs = Vec::new();

    inputs.push("let x = 50;".to_string());
    inputs.push("let y = x;".to_string());
    inputs.push("let result;".to_string());

    for input in inputs {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        assert!(res.statements[0].to_str() == input);
    }
}

#[test]
fn test_return_statement() {
    let mut inputs = Vec::new();

    inputs.push("return;".to_string());
    inputs.push("return x;".to_string());
    inputs.push("return 10;".to_string());

    for input in inputs {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        assert!(res.statements[0].to_str() == input);
    }
}
#[test]
fn test_bool_literal() {
    let mut inputs = Vec::new();

    inputs.push("let x = false;".to_string());
    inputs.push("let y = true;".to_string());

    for input in inputs {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        assert!(res.statements[0].to_str() == input);
    }
}

#[test]
fn test_parentheses() {
    let mut tests = Vec::new();

    tests.push((
        "((true || (false || true)))".to_string(),
        "(true || (false || true))".to_string(),
    ));

    for (input, expect) in tests {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        assert!(res.statements[0].to_str() == expect);
    }
}

#[test]
fn test_infix() {
    let input = "10 * 10 + 20 * 20 + 100".to_string();
    let expect = "(((10 * 10) + (20 * 20)) + 100)".to_string();

    let lexer = Lexer::new(input.clone());

    let mut parser = Parser::new(lexer);
    let res = parser.parse().ok().unwrap();
    assert!(res.statements[0].to_str() == expect);
}

#[test]
fn test_prefix_expression() {
    let input = "!(true&&!(10<20))".to_string();
    let expect = "!((true && !((10 < 20))))".to_string();

    let lexer = Lexer::new(input.clone());

    let mut parser = Parser::new(lexer);
    let res = parser.parse().ok().unwrap();
    let result = res.statements[0].to_str();

    assert!(result == expect);
}

#[test]
fn test_if_expression() {
    let mut tests = Vec::new();

    tests.push((
        "if (x > 10) { return x; } else {return 0;}".to_string(),
        "if (x > 10) {return x;} else {return 0;}".to_string(),
    ));
    tests.push((
        "if (flag) {return true;}".to_string(),
        "if flag {return true;}".to_string(),
    ));

    for (idx, (input, expect)) in tests.iter().enumerate() {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        let result = res.statements[0].to_str();
        assert!(&result == expect);
    }
}

#[test]
fn test_call_expression() {
    let mut tests = Vec::new();

    tests.push(("call(foo, bar)".to_string(), "call(foo, bar)".to_string()));
    tests.push(("call2()".to_string(), "call2()".to_string()));
    tests.push(("call3(30)".to_string(), "call3(30)".to_string()));

    for (input, expect) in tests {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        let result = res.statements[0].to_str();

        assert!(result == expect);
    }
}

#[test]
fn test_function_literal() {
    let mut tests = Vec::new();

    tests.push((
        "fn call(foo) {
         let foo = true;
         return foo;
         }"
        .to_string(),
        "fn call(foo) {let foo = true;return foo;}".to_string(),
    ));
    tests.push((
        "fn call2() {
         let hello = true;
         return hello;
         }"
        .to_string(),
        "fn call2() {let hello = true;return hello;}".to_string(),
    ));
    tests.push((
        "let hey = fn () {
         let hello = true;
         return hello;
         };"
        .to_string(),
        "let hey = fn hey() {let hello = true;return hello;};".to_string(),
    ));
    for (input, expect) in tests {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        let result = res.statements[0].to_str();

        assert!(result == expect);
    }
}

#[test]
fn test_array_literal() {
    let mut tests = Vec::new();

    tests.push((
        "[]".to_string(),
        ArrayLiteral {
            elements: Vec::new(),
        }
        .to_str(),
    ));

    tests.push(("[1,2,3,4]".to_string(), "[1, 2, 3, 4]".to_string()));

    tests.push((
        "[[1,2,3,4],[5,6,7,8]]".to_string(),
        "[[1, 2, 3, 4], [5, 6, 7, 8]]".to_string(),
    ));

    for (input, expect) in tests {
        let lexer = Lexer::new(input.clone());

        let mut parser = Parser::new(lexer);
        let res = parser.parse().ok().unwrap();
        let result = res.statements[0].to_str();

        assert!(result == expect);
    }
}
