use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression,
        IndexExpression, InfixExpression, IntegerLiteral, LetStatement,
        PrefixExpression, Program, ReturnStatement, Statement, StringLiteral,
    },
    object::Object,
};

use super::{op::OP, Bytecode};

pub struct Compiler {
    constants: Vec<Object>,
    instructions: Vec<OP>,
}

impl Compiler {
    pub fn compile(&mut self, src: Program) -> Option<()> {
        for stm in src.statements {
            self.compile_stm(&stm)
        }

        None
    }

    pub fn compile_stm(&mut self, stm: &Statement) {
        match stm {
            Statement::ExpressionStatement(stm) => self.compile_expression_stm(stm),
            Statement::LetStatement(stm) => self.compile_let_stm(stm),
            Statement::ReturnStatement(stm) => self.compile_return_stm(stm),
            Statement::BlockStatement(stm) => self.compile_block_stm(stm),
        }
    }

    pub fn compile_exp(&mut self, exp: &Expression) {
        match exp {
            Expression::Identifier(_) => todo!(),
            Expression::IntegerLiteral(_) => todo!(),
            Expression::BooleanLiteral(_) => todo!(),
            Expression::StringLiteral(_) => todo!(),
            Expression::FunctionLiteral(_) => todo!(),
            Expression::ArrayLiteral(_) => todo!(),
            Expression::InfixExpression(_) => todo!(),
            Expression::PrefixExpression(_) => todo!(),
            Expression::IfExpression(_) => todo!(),
            Expression::CallExpression(_) => todo!(),
            Expression::IndexExpression(_) => todo!(),
        }
    }

    pub fn compile_expression_stm(&mut self, stm: &ExpressionStatement) {
        self.compile_exp(&stm.expression.as_ref().unwrap())
    }
    pub fn compile_let_stm(&mut self, stm: &LetStatement) {}
    pub fn compile_return_stm(&mut self, stm: &ReturnStatement) {}
    pub fn compile_block_stm(&mut self, stm: &BlockStatement) {}

    pub fn compile_identifier_exp(&mut self, exp: &Identifier) {}
    pub fn compile_integer_literal(&mut self, lit: &IntegerLiteral) {}
    pub fn compile_bool_literal(&mut self, lit: &BooleanLiteral) {}
    pub fn compile_string_literal(&mut self, lit: &StringLiteral) {}
    pub fn compile_function_literal(&mut self, lit: &FunctionLiteral) {}
    pub fn compile_array_literal(&mut self, lit: &ArrayLiteral) {}
    pub fn compile_infix_exp(&mut self, exp: &InfixExpression) {}
    pub fn compile_prefix_exp(&mut self, exp: &PrefixExpression) {}
    pub fn compile_if_exp(&mut self, exp: &IfExpression) {}
    pub fn compile_call_exp(&mut self, exp: &CallExpression) {}
    pub fn compile_index_exp(&mut self, exp: &IndexExpression) {}

    fn emit(&mut self, op: OP) {}

    pub fn bytecode(&mut self) -> Bytecode {
        todo!()
    }
}
