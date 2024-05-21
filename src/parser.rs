use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression, IndexExpression,
        InfixExpression, IntegerLiteral, LetStatement, PrefixExpression, Program, ReturnStatement,
        Statement, StringLiteral,
    },
    lexer::Lexer,
    parser::errors::InfixFunctionError,
    token::{Kind, Token},
};

use self::errors::{ParserError, PrefixFunctionError};

pub mod errors;

#[derive(PartialEq, PartialOrd)]
enum Precedence {
    Lowest,
    Or,
    And,
    BitOr,
    BitAnd,
    Equals,  // ==
    Cmp,     // > or <
    Sum,     // +
    Product, // *
    Prefix,  // -(val) or !(val)
    Index,
    Call, // calling function like func(val)
}

fn find_precedences(kind: Kind) -> Precedence {
    match kind {
        Kind::Or => Precedence::Or,
        Kind::And => Precedence::And,
        Kind::Bit_Or => Precedence::BitOr,
        Kind::Bit_And => Precedence::BitAnd,
        Kind::EQ | Kind::NOT_EQ => Precedence::Equals,
        Kind::LT | Kind::LT_OR_EQ | Kind::GT | Kind::GT_OR_EQ => Precedence::Cmp,
        Kind::Plus | Kind::Minus => Precedence::Sum,
        Kind::Product | Kind::Divide | Kind::Mod => Precedence::Product,
        Kind::LBRACKET => Precedence::Index,
        Kind::LPAREN => Precedence::Call,
        __ => Precedence::Lowest,
    }
}

fn is_infix(kind: &Kind) -> bool {
    match kind {
        Kind::Plus
        | Kind::Minus
        | Kind::Divide
        | Kind::Product
        | Kind::Mod
        | Kind::EQ
        | Kind::NOT_EQ
        | Kind::LT
        | Kind::LT_OR_EQ
        | Kind::GT
        | Kind::GT_OR_EQ
        | Kind::Bit_Or
        | Kind::Bit_And
        | Kind::Or
        | Kind::And
        | Kind::LBRACKET
        | Kind::LPAREN => true,
        __ => false,
    }
}

pub struct Parser {
    lexer: Lexer,
    cur_token: Token,
    next_token: Token,
    // do we need
    // errors: Vec<Box<dyn ParserError>> ?
}
impl Parser {
    pub fn new(lexer: Lexer) -> Self {
        let mut lexer = lexer;
        let cur_token = lexer.next();
        let next_token = lexer.next();

        Self {
            lexer,
            cur_token,
            next_token,
            // errors: Vec::new(),
        }
    }

    fn next(&mut self) {
        self.cur_token = self.next_token.clone();
        self.next_token = self.lexer.next();
    }

    fn peek_next(&self) -> &Token {
        &self.next_token
    }
    fn peek_next_is(&self, kind: &Kind) -> bool {
        &self.next_token.kind == kind
    }

    fn expect_next_is(&mut self, kind: &Kind) -> bool {
        if &self.next_token.kind == kind {
            self.next();
            true
        } else {
            false
        }
    }

    fn cur_precedence(&self) -> Precedence {
        find_precedences(self.cur_token.kind)
    }
    fn peek_precedence(&self) -> Precedence {
        find_precedences(self.next_token.kind)
    }

    pub fn parse(&mut self) -> Result<Program, Vec<Vec<Box<dyn ParserError>>>> {
        let mut program = Program::new();
        let mut errs: Vec<Vec<Box<dyn ParserError>>> = Vec::new();

        while self.cur_token.kind != Kind::EOF {
            let cur_stm = self.parse_statement();
            if cur_stm.is_ok() {
                program.push_stm(cur_stm.ok().unwrap());
            } else {
                let err = cur_stm.err().unwrap();
                errs.push(err);
            }
            self.next()
        }

        if errs.is_empty() {
            Ok(program)
        } else {
            Err(errs)
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, Vec<Box<dyn ParserError>>> {
        match self.cur_token.kind {
            Kind::Let => {
                let res = self.parse_let_statement();
                if res.is_ok() {
                    Ok(Statement::LetStatement(res.ok().unwrap()))
                } else {
                    let mut err_vec: Vec<Box<dyn ParserError>> = Vec::new();
                    // deeper first.
                    err_vec.push(Box::new(res.err().unwrap()));
                    err_vec.push(Box::new(errors::ParseError {
                        detail: "faild to parse let statement".to_string(),
                        position: self.lexer.get_pos(),
                    }));

                    Err(err_vec)
                }
            }
            Kind::Return => {
                let res = self.parse_return_statement();
                if res.is_ok() {
                    Ok(Statement::ReturnStatement(res.ok().unwrap()))
                } else {
                    Err(res.err().unwrap())
                }
            }
            __ => {
                let res = self.parse_expression_statement();
                if res.is_ok() {
                    Ok(Statement::ExpressionStatement(res.ok().unwrap()))
                } else {
                    Err(res.err().unwrap())
                }
            }
        }
    }

    fn parse_let_statement(&mut self) -> Result<LetStatement, errors::ParseError> {
        let cur_token = self.cur_token.clone();

        if !self.peek_next_is(&Kind::Ident) {
            return Err(errors::ParseError {
                detail: "next_token is not a Kind::Ident".to_string(),
                position: self.lexer.get_pos(),
            });
        }

        self.next();

        let mut stm = LetStatement {
            token: cur_token,
            identifier: Identifier {
                token: self.cur_token.clone(),
                value: self.cur_token.literal.clone(),
            },
            value: None,
        };
        if self.peek_next().kind == Kind::Assign {
            self.next(); // cur_token will be = (assign)
            self.next(); // cur_token will be rightside of =
            let value = self.parse_expression(Precedence::Lowest).ok();
            if value.is_none() {
                return Err(errors::ParseError {
                    detail: "error occured while parsing let statement's value".to_string(),
                    position: self.lexer.get_pos(),
                });
            }
            if let Some(Expression::FunctionLiteral(mut lit)) = value {
                lit.ident = Some(stm.identifier.clone());
                lit.is_let_bind = true;
                stm.value = Some(Expression::FunctionLiteral(lit));
            } else {
                stm.value = value
            }
        }

        if !self.expect_next_is(&Kind::Semicolon) {
            return Err(errors::ParseError {
                detail: "next_token is not a Kind::Semicolon".to_string(),
                position: self.lexer.get_pos(),
            });
        }

        Ok(stm)
    }

    fn parse_return_statement(&mut self) -> Result<ReturnStatement, Vec<Box<dyn ParserError>>> {
        let mut stm = ReturnStatement {
            token: self.cur_token.clone(), // cur token is return
            value: None,
        };

        if self.peek_next().kind != Kind::Semicolon {
            // if not null return,
            self.next();
            let res = self.parse_expression(Precedence::Lowest);
            if res.is_err() {
                let mut errs = res.unwrap_err();
                errs.push(Box::new(errors::ParseError {
                    detail: "faild on parsing value (on return statement)".to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            }
            stm.value = res.ok();
            if self.peek_next_is(&Kind::Semicolon) {
                // consume Semicolon
                self.next();
            }
        } else {
            // next token is Semicolon
            self.next(); // consume Semicolon
        }

        Ok(stm)
    }

    fn parse_expression_statement(
        &mut self,
    ) -> Result<ExpressionStatement, Vec<Box<dyn ParserError>>> {
        let token = self.cur_token.clone();
        let expression = self.parse_expression(Precedence::Lowest);
        if expression.is_err() {
            return Err(expression.err().unwrap());
        }
        if self.peek_next_is(&Kind::Semicolon) {
            // consume Semicolon
            self.next();
        }
        Ok(ExpressionStatement {
            token,
            expression: Some(expression.unwrap()),
        })
    }

    fn parse_identifier(&mut self) -> Identifier {
        Identifier {
            token: self.cur_token.clone(),
            value: self.cur_token.literal.clone(),
        }
    }

    fn parse_integer_literal(&mut self) -> Result<IntegerLiteral, errors::PrefixFunctionError> {
        let value = self.cur_token.literal.parse();
        if value.is_err() {
            return Err(errors::PrefixFunctionError {
                detail: format!(
                    "faild to parse IntegerLiteral, error detail: {:?}",
                    value.err().unwrap()
                ),
                position: self.lexer.get_pos(),
                kind: errors::PrefixFunctionErrorKind::IntegerParseError,
            });
        }
        let value = value.unwrap();
        Ok(IntegerLiteral {
            token: self.cur_token.clone(),
            value,
        })
    }

    fn parse_bool_literal(&mut self) -> BooleanLiteral {
        let value = self.cur_token.literal.parse().unwrap();

        BooleanLiteral {
            token: self.cur_token.clone(),
            value,
        }
    }

    fn parse_string_literal(&mut self) -> StringLiteral {
        let value = self.cur_token.literal.clone();

        StringLiteral {
            token: self.cur_token.clone(),
            value,
        }
    }

    fn parse_block_statement(&mut self) -> Result<BlockStatement, Vec<Box<dyn ParserError>>> {
        let token = self.cur_token.clone();

        if token.kind != Kind::LBRACE {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "LBRACE not found (on block Statement)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        self.next(); // consume LBRACE

        let mut statements = Vec::new();

        while self.cur_token.kind != Kind::RBRACE && self.cur_token.kind != Kind::EOF {
            let stm = self.parse_statement();
            if stm.is_err() {
                let mut errs = stm.err().unwrap();
                errs.push(Box::new(errors::ParseError {
                    detail: "Parse faild in block statements".to_string(),
                    position: self.lexer.get_pos(),
                }));

                return Err(errs);
            }
            statements.push(stm.ok().unwrap());
            self.next();
        }

        if self.cur_token.kind != Kind::RBRACE {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "matching RBRACE not found (on block Statement)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        Ok(BlockStatement { token, statements })
    }

    fn parse_prefix_expression(&mut self) -> Result<Expression, Vec<Box<dyn ParserError>>> {
        let token = self.cur_token.clone();
        self.next();
        let exp = self.parse_expression(Precedence::Prefix);
        if exp.is_err() {
            let errs = exp.err().unwrap();
            return Err(errs);
        }
        let right = Box::new(exp.unwrap());
        Ok(Expression::PrefixExpression(PrefixExpression {
            token,
            right,
        }))
    }

    fn parse_group_expression(&mut self) -> Result<Expression, Vec<Box<dyn ParserError>>> {
        self.next(); // consume LPAREN
        let exp = self.parse_expression(Precedence::Lowest);
        if exp.is_err() {
            let errs = exp.err().unwrap();
            return Err(errs);
        }
        if !self.peek_next_is(&Kind::RPAREN) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "RPAREN not found".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        self.next(); // consume RPAREN
        exp
    }

    fn parse_if_expression(&mut self) -> Result<IfExpression, Vec<Box<dyn ParserError>>> {
        let if_token = self.cur_token.clone();
        if !self.expect_next_is(&Kind::LPAREN) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "LPAREN not found (next of IF token)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        } // LPAREN had consumed

        self.next();

        let condition = self.parse_expression(Precedence::Lowest);
        if condition.is_err() {
            let mut errs: Vec<Box<dyn ParserError>> = condition.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "Faild to parse inner condition expression (on If expression)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        let condition = condition.ok().unwrap();

        if !self.expect_next_is(&Kind::RPAREN) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "Cannot found RPAREN in IfExpression".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        } // RPAREN had consumed

        if !self.expect_next_is(&Kind::LBRACE) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "Cannot found LBRACE on next side of condition (on IfExpression)"
                    .to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        } // LBRACE had consumed (cur_token == LBRACE)

        let consequence = self.parse_block_statement();

        if consequence.is_err() {
            let mut errs: Vec<Box<dyn ParserError>> = consequence.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "Faild to parse consequence expression (on If expression)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        let consequence = consequence.ok().unwrap();

        let mut alternative = None;

        if self.expect_next_is(&Kind::Else) {
            if !self.expect_next_is(&Kind::LBRACE) {
                let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
                errs.push(Box::new(errors::ParseError {
                    detail: "Cannot found LBRACE on right side of else (on IfExpression)"
                        .to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            } // LBRACE had consumed (cur_token == LBRACE)

            let res = self.parse_block_statement();

            if res.is_err() {
                let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                errs.push(Box::new(errors::ParseError {
                    detail: "Faild to parse alternative block expression (on If expression)"
                        .to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            }
            alternative = Some(res.ok().unwrap());
        }

        Ok(IfExpression {
            token: if_token,
            condition: Box::new(condition),
            consequence,
            alternative,
        })
    }

    fn parse_function_literal(&mut self) -> Result<FunctionLiteral, Vec<Box<dyn ParserError>>> {
        let token = self.cur_token.clone();
        let mut ident = None;

        if self.expect_next_is(&Kind::Ident) {
            ident = Some(Identifier {
                token: self.cur_token.clone(),
                value: self.cur_token.literal.clone(),
            })
        }

        if !self.expect_next_is(&Kind::LPAREN) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "LPAREN not found (on FunctionLiteral)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        let params = self.parse_function_parameters();
        if params.is_err() {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(params.err().unwrap()));
            errs.push(Box::new(errors::ParseError {
                detail: "Error while parsing parmams (on Function literal)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        let parameters = params.unwrap();

        if !self.expect_next_is(&Kind::LBRACE) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "LBRACE not found (on Function literal)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        } // self.cur_token == Kind::LBRACE

        let body = self.parse_block_statement();

        if body.is_err() {
            let mut errs: Vec<Box<dyn ParserError>> = body.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "Error occurs in innerblock (on Function literal)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }
        let body = body.unwrap();

        Ok(FunctionLiteral {
            token,
            ident,
            parameters,
            body,
            is_let_bind: false,
        })
    }

    fn parse_function_parameters(&mut self) -> Result<Vec<Identifier>, errors::ParseError> {
        let mut identifiers = Vec::new();

        if self.peek_next_is(&Kind::RPAREN) {
            self.next(); // consume RPAREN
            return Ok(identifiers);
        }

        if !self.expect_next_is(&Kind::Ident) {
            return Err(errors::ParseError {
                detail: "Ident token not found (on parsing function params)".to_string(),
                position: self.lexer.get_pos(),
            });
        } // somethings wrong.

        identifiers.push(Identifier {
            token: self.cur_token.clone(),
            value: self.cur_token.literal.clone(),
        });

        while self.peek_next_is(&Kind::Comma) {
            self.next(); // consume comma
            if !self.expect_next_is(&Kind::Ident) {
                return Err(errors::ParseError {
                    detail: "Ident token not found (on parsing function params)".to_string(),
                    position: self.lexer.get_pos(),
                });
            } // got next id

            identifiers.push(Identifier {
                token: self.cur_token.clone(),
                value: self.cur_token.literal.clone(),
            });
        }
        if !self.expect_next_is(&Kind::RPAREN) {
            return Err(errors::ParseError {
                detail: "matching RPAREN not found (on parsing function params)".to_string(),
                position: self.lexer.get_pos(),
            });
        }

        Ok(identifiers)
    }

    fn parse_call_expression(
        &mut self,
        function: Expression,
    ) -> Result<CallExpression, Vec<Box<dyn ParserError>>> {
        let token = self.cur_token.clone();
        let arguments = self.parse_call_args();
        if arguments.is_err() {
            let mut errs: Vec<Box<dyn ParserError>> = arguments.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "Error occurs in innerblock (on Function literal)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        let arguments = arguments.unwrap();
        Ok(CallExpression {
            token,
            function: Box::new(function),
            arguments,
        })
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expression>, Vec<Box<dyn ParserError>>> {
        let mut args = Vec::new();
        if self.peek_next_is(&Kind::RPAREN) {
            self.next(); // consume RPAREN
            return Ok(args);
        }
        self.next();

        let res = self.parse_expression(Precedence::Lowest);
        if res.is_err() {
            let mut errs = res.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "faild to parse inner expression (on parse call args)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        args.push(res.unwrap());

        while self.peek_next_is(&Kind::Comma) {
            self.next(); // consume Comma
            self.next();

            let res = self.parse_expression(Precedence::Lowest);
            if res.is_err() {
                let mut errs = res.err().unwrap();
                errs.push(Box::new(errors::ParseError {
                    detail: "faild to parse inner expression (on parse call args)".to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            }

            args.push(res.unwrap());
        }

        if !self.expect_next_is(&Kind::RPAREN) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "matching RPAREN not found (on parse call args)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        Ok(args)
    }

    fn parse_array_elements(&mut self) -> Result<Vec<Expression>, Vec<Box<dyn ParserError>>> {
        let mut elements = Vec::new();
        if self.peek_next_is(&Kind::RBRACKET) {
            self.next(); // consume RBRACKET
            return Ok(elements);
        }
        self.next();

        let res = self.parse_expression(Precedence::Lowest);
        if res.is_err() {
            let mut errs = res.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "faild to parse inner expression (on parse call args)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        elements.push(res.unwrap());

        while self.peek_next_is(&Kind::Comma) {
            self.next(); // consume Comma
            self.next();

            let res = self.parse_expression(Precedence::Lowest);
            if res.is_err() {
                let mut errs = res.err().unwrap();
                errs.push(Box::new(errors::ParseError {
                    detail: "faild to parse inner expression (on parse call args)".to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            }

            elements.push(res.unwrap());
        }

        if !self.expect_next_is(&Kind::RBRACKET) {
            let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
            errs.push(Box::new(errors::ParseError {
                detail: "matching RBRACKET not found (on parse call args)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        Ok(elements)
    }

    fn parse_prefix(&mut self, kind: &Kind) -> Result<Expression, Vec<Box<dyn ParserError>>> {
        match kind {
            Kind::Ident => Ok(Expression::Identifier(self.parse_identifier())),
            Kind::Int => {
                let res = self.parse_integer_literal();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
                    errs.push(Box::new(res.err().unwrap()));
                    return Err(errs);
                }
                Ok(Expression::IntegerLiteral(res.ok().unwrap()))
            }
            Kind::String => {
                let res = self.parse_string_literal();
                Ok(Expression::StringLiteral(res))
            }
            Kind::LPAREN => {
                let res = self.parse_group_expression();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                    errs.push(Box::new(PrefixFunctionError {
                        detail: "failed to parse group expression".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::PrefixFunctionErrorKind::GroupExpressionError,
                    }));
                    return Err(errs);
                }
                Ok(res.ok().unwrap())
            }
            Kind::Bang | Kind::Minus => {
                let res = self.parse_prefix_expression();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                    errs.push(Box::new(PrefixFunctionError {
                        detail: "failed to prefix expression".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::PrefixFunctionErrorKind::PrefixExpressionError,
                    }));
                    return Err(errs);
                }
                Ok(res.ok().unwrap())
            }
            Kind::True | Kind::False => Ok(Expression::BooleanLiteral(self.parse_bool_literal())),
            Kind::If => {
                let res = self.parse_if_expression();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                    errs.push(Box::new(PrefixFunctionError {
                        detail: "failed to parse if expression".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::PrefixFunctionErrorKind::IfExpressionError,
                    }));
                    return Err(errs);
                }
                Ok(Expression::IfExpression(res.ok().unwrap()))
            }
            Kind::Function => {
                let res = self.parse_function_literal();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                    errs.push(Box::new(PrefixFunctionError {
                        detail: "failed to parse function literal".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::PrefixFunctionErrorKind::FunctionLiteralError,
                    }));
                    return Err(errs);
                }
                Ok(Expression::FunctionLiteral(res.ok().unwrap()))
            }
            Kind::LBRACKET => {
                let res = self.parse_array_elements();
                if res.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = res.err().unwrap();
                    errs.push(Box::new(PrefixFunctionError {
                        detail: "failed to parse array elements".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::PrefixFunctionErrorKind::FunctionLiteralError,
                    }));
                    return Err(errs);
                }
                Ok(Expression::ArrayLiteral(ArrayLiteral {
                    elements: res.unwrap(),
                }))
            }
            not_matched => {
                let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
                errs.push(Box::new(PrefixFunctionError {
                    detail: format!(
                        "failed to find matching parse function on {:?}",
                        not_matched
                    ),
                    position: self.lexer.get_pos(),
                    kind: errors::PrefixFunctionErrorKind::NoPrefixFunction,
                }));
                Err(errs)
            }
        }
    }

    fn parse_infix(&mut self, left: Expression) -> Result<Expression, Vec<Box<dyn ParserError>>> {
        let cur_token = self.cur_token.clone();

        match cur_token.kind {
            Kind::LPAREN => {
                let call_expression = self.parse_call_expression(left);
                if call_expression.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = call_expression.err().unwrap();
                    errs.push(Box::new(InfixFunctionError {
                        detail: "failed to parse on call expression (on parse infix)".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::InfixFunctionErrorKind::ParseError,
                    }));
                    return Err(errs);
                }
                return Ok(Expression::CallExpression(call_expression.unwrap()));
            }
            Kind::LBRACKET => {
                let token = self.cur_token.clone();

                self.next();

                let index = self.parse_expression(Precedence::Lowest);
                if index.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = index.err().unwrap();
                    errs.push(Box::new(InfixFunctionError {
                        detail: "failed to parse on index expression (on parse index)".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::InfixFunctionErrorKind::ParseError,
                    }));
                    return Err(errs);
                }
                let index = index.ok().unwrap();
                if self.expect_next_is(&Kind::RBRACKET) {
                } else {
                    let mut errs: Vec<Box<dyn ParserError>> = Vec::new();
                    errs.push(Box::new(InfixFunctionError {
                        detail: "failed to find closing RBRACKET (\"]\") (on parse index)"
                            .to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::InfixFunctionErrorKind::ParseError,
                    }));
                    return Err(errs);
                }
                Ok(Expression::IndexExpression(IndexExpression {
                    token,
                    left: Box::new(left),
                    index: Box::new(index),
                }))
            }
            __not_matched => {
                let operator = self.cur_token.clone();
                let cur_precedence = self.cur_precedence();

                self.next();

                let right = self.parse_expression(cur_precedence);
                if right.is_err() {
                    let mut errs: Vec<Box<dyn ParserError>> = right.err().unwrap();
                    errs.push(Box::new(InfixFunctionError {
                        detail: "failed to parse on right expression (on parse infix)".to_string(),
                        position: self.lexer.get_pos(),
                        kind: errors::InfixFunctionErrorKind::ParseError,
                    }));
                    return Err(errs);
                }
                let right = right.ok().unwrap();
                return Ok(Expression::InfixExpression(InfixExpression {
                    token: cur_token,
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                }));
            }
        }
    }

    fn parse_expression(
        &mut self,
        precedence: Precedence,
    ) -> Result<Expression, Vec<Box<dyn ParserError>>> {
        let exp = self.parse_prefix(&self.cur_token.kind.clone());

        if exp.is_err() {
            let mut errs = exp.err().unwrap();
            errs.push(Box::new(errors::ParseError {
                detail: "error on parsing prefix expression (on parse expression)".to_string(),
                position: self.lexer.get_pos(),
            }));
            return Err(errs);
        }

        let mut exp = exp.ok().unwrap();

        while !self.peek_next_is(&Kind::Semicolon) && precedence < self.peek_precedence() {
            if !is_infix(&self.peek_next().kind) {
                return Ok(exp);
            }

            self.next();

            let infix = self.parse_infix(exp);

            if infix.is_err() {
                let mut errs = infix.err().unwrap();
                errs.push(Box::new(errors::ParseError {
                    detail: "error on parsing infix expression (on parse expression)".to_string(),
                    position: self.lexer.get_pos(),
                }));
                return Err(errs);
            }
            exp = infix.ok().unwrap();
        }
        Ok(exp)
    }
}
