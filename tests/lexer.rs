use dlang::lexer;
use dlang::token::{Kind, Token};

#[test]
fn test_lexer() {
    let input = "
    let ten = 10;
    let two = 2;

    let add = fn(x1,y1) {
    x1 + y1;
    };

    let result = add(ten,two);
    let flag = false;

    let test = fn(x,y) {
    if (x>y) {
    return x*y;
    }else if (x<y) {
    return x/y;
    }else {
    flag = true;
    return x%y;
    }

    ten <= two;
    ten >= two;
    ten == two;
    ten != two;

    ten & two;
    ten | two;

    true && false;
    true || false;

    []
    ";

    let mut expects = Vec::new();
    let mut lex = lexer::Lexer::new(input.to_string());

    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::Int, "10"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::Int, "2"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "add"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::Function, "fn"));
    expects.push(Token::with(Kind::LPAREN, "("));
    expects.push(Token::with(Kind::Ident, "x1"));
    expects.push(Token::with(Kind::Comma, ","));
    expects.push(Token::with(Kind::Ident, "y1"));
    expects.push(Token::with(Kind::RPAREN, ")"));
    expects.push(Token::with(Kind::LBRACE, "{"));
    expects.push(Token::with(Kind::Ident, "x1"));
    expects.push(Token::with(Kind::Plus, "+"));
    expects.push(Token::with(Kind::Ident, "y1"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::RBRACE, "}"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "result"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::Ident, "add"));
    expects.push(Token::with(Kind::LPAREN, "("));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::Comma, ","));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::RPAREN, ")"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "flag"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::False, "false"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Let, "let"));
    expects.push(Token::with(Kind::Ident, "test"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::Function, "fn"));
    expects.push(Token::with(Kind::LPAREN, "("));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::Comma, ","));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::RPAREN, ")"));
    expects.push(Token::with(Kind::LBRACE, "{"));
    expects.push(Token::with(Kind::If, "if"));
    expects.push(Token::with(Kind::LPAREN, "("));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::GT, ">"));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::RPAREN, ")"));
    expects.push(Token::with(Kind::LBRACE, "{"));
    expects.push(Token::with(Kind::Return, "return"));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::Product, "*"));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::RBRACE, "}"));
    expects.push(Token::with(Kind::Else, "else"));
    expects.push(Token::with(Kind::If, "if"));
    expects.push(Token::with(Kind::LPAREN, "("));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::LT, "<"));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::RPAREN, ")"));
    expects.push(Token::with(Kind::LBRACE, "{"));
    expects.push(Token::with(Kind::Return, "return"));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::Divide, "/"));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::RBRACE, "}"));
    expects.push(Token::with(Kind::Else, "else"));
    expects.push(Token::with(Kind::LBRACE, "{"));
    expects.push(Token::with(Kind::Ident, "flag"));
    expects.push(Token::with(Kind::Assign, "="));
    expects.push(Token::with(Kind::True, "true"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Return, "return"));
    expects.push(Token::with(Kind::Ident, "x"));
    expects.push(Token::with(Kind::Mod, "%"));
    expects.push(Token::with(Kind::Ident, "y"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::RBRACE, "}"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::LT_OR_EQ, "<="));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::GT_OR_EQ, ">="));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::EQ, "=="));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::NOT_EQ, "!="));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::Bit_And, "&"));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::Ident, "ten"));
    expects.push(Token::with(Kind::Bit_Or, "|"));
    expects.push(Token::with(Kind::Ident, "two"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::True, "true"));
    expects.push(Token::with(Kind::And, "&&"));
    expects.push(Token::with(Kind::False, "false"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::True, "true"));
    expects.push(Token::with(Kind::Or, "||"));
    expects.push(Token::with(Kind::False, "false"));
    expects.push(Token::with(Kind::Semicolon, ";"));
    expects.push(Token::with(Kind::LBRACKET, "["));
    expects.push(Token::with(Kind::RBRACKET, "]"));
    expects.push(Token::with(Kind::EOF, "EOF"));

    for expect in expects {
        let cur_token = lex.next_token();
        assert_eq!(expect.kind, cur_token.kind);
        assert_eq!(expect.literal, cur_token.literal);
    }
}
