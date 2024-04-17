use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression,
        IndexExpression, InfixExpression, IntegerLiteral, LetStatement,
        PrefixExpression, Program, ReturnStatement, Statement, StringLiteral,
    },
    object::{Int, Object},
    token::Kind,
};

use super::{op::OP, Bytecode};

pub struct Compiler {
    constants: Vec<Object>,
    instructions: Vec<OP>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),
            instructions: Vec::new(),
        }
    }

    pub fn bytecode(self) -> Bytecode {
        Bytecode {
            constants: self.constants,
            instructions: self.instructions,
        }
    }

    pub fn compile(&mut self, src: Program) -> Option<()> {
        for stm in src.statements {
            self.compile_stm(&stm)
        }

        None
    }

    fn compile_stm(&mut self, stm: &Statement) {
        match stm {
            Statement::ExpressionStatement(stm) => self.compile_expression_stm(stm),
            Statement::LetStatement(stm) => self.compile_let_stm(stm),
            Statement::ReturnStatement(stm) => self.compile_return_stm(stm),
            Statement::BlockStatement(stm) => self.compile_block_stm(stm),
        }
    }

    fn compile_exp(&mut self, exp: &Expression) {
        match exp {
            Expression::Identifier(exp) => self.compile_identifier_exp(exp),
            Expression::IntegerLiteral(lit) => self.compile_integer_literal(lit),
            Expression::BooleanLiteral(_) => todo!(),
            Expression::StringLiteral(_) => todo!(),
            Expression::FunctionLiteral(_) => todo!(),
            Expression::ArrayLiteral(_) => todo!(),
            Expression::InfixExpression(exp) => self.compile_infix_exp(exp),
            Expression::PrefixExpression(exp) => self.compile_prefix_exp(exp),
            Expression::IfExpression(_) => todo!(),
            Expression::CallExpression(_) => todo!(),
            Expression::IndexExpression(_) => todo!(),
        }
    }

    fn compile_expression_stm(&mut self, stm: &ExpressionStatement) {
        self.compile_exp(&stm.expression.as_ref().unwrap())
    }
    fn compile_let_stm(&mut self, stm: &LetStatement) {}
    fn compile_return_stm(&mut self, stm: &ReturnStatement) {}
    fn compile_block_stm(&mut self, stm: &BlockStatement) {}

    fn compile_identifier_exp(&mut self, exp: &Identifier) {}
    fn compile_integer_literal(&mut self, lit: &IntegerLiteral) {
        let int = Object::Int(Int { value: lit.value });
        self.constants.push(int);
        self.emit(OP::CONST {
            idx: self.constants.len() - 1,
        })
    }
    fn compile_bool_literal(&mut self, lit: &BooleanLiteral) {}
    fn compile_string_literal(&mut self, lit: &StringLiteral) {}
    fn compile_function_literal(&mut self, lit: &FunctionLiteral) {}
    fn compile_array_literal(&mut self, lit: &ArrayLiteral) {}
    fn compile_prefix_exp(&mut self, exp: &PrefixExpression) {
        self.compile_exp(&exp.right);

        match exp.token.kind {
            Kind::Bang => self.emit(OP::BANG),
            Kind::Minus => self.emit(OP::NEG),
            __not_matched => {
                // emit error
            }
        }
    }
    fn compile_infix_exp(&mut self, exp: &InfixExpression) {
        self.compile_exp(&exp.right);
        self.compile_exp(&exp.left);

        match &exp.operator.kind {
            Kind::Assign => todo!(),
            Kind::Plus => self.emit(OP::ADD),
            Kind::Minus => self.emit(OP::SUB),
            Kind::Product => self.emit(OP::PRODUCT),
            Kind::Divide => self.emit(OP::DIVIDE),
            Kind::Mod => self.emit(OP::MOD),
            Kind::LT => todo!(),
            Kind::LT_OR_EQ => todo!(),
            Kind::GT => todo!(),
            Kind::GT_OR_EQ => todo!(),
            Kind::EQ => todo!(),
            Kind::NOT_EQ => todo!(),
            Kind::And => todo!(),
            Kind::Or => todo!(),
            Kind::Bit_And => todo!(),
            Kind::Bit_Or => todo!(),
            __not_matched => {
                // emit error
            }
        }
    }
    fn compile_if_exp(&mut self, exp: &IfExpression) {}
    fn compile_call_exp(&mut self, exp: &CallExpression) {}
    fn compile_index_exp(&mut self, exp: &IndexExpression) {}

    fn emit(&mut self, op: OP) {
        self.instructions.push(op)
    }
}
