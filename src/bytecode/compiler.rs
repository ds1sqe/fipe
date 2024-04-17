use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression, IndexExpression,
        InfixExpression, IntegerLiteral, LetStatement, PrefixExpression, Program, ReturnStatement,
        Statement, StringLiteral,
    },
    object::{Bool, Int, Object, StringObject},
    token::Kind,
};

use super::{instruction::Instruction, instructions::Instructions, Bytecode};

pub struct Compiler {
    constants: Vec<Object>,
    instructions: Instructions,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),
            instructions: Instructions::new(),
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
            Expression::BooleanLiteral(lit) => self.compile_bool_literal(lit),
            Expression::StringLiteral(lit) => self.compile_string_literal(lit),
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
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })
    }
    fn compile_bool_literal(&mut self, lit: &BooleanLiteral) {
        let boolean = Object::Bool(Bool { value: lit.value });
        self.constants.push(boolean);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })
    }
    fn compile_string_literal(&mut self, lit: &StringLiteral) {
        let str = Object::String(StringObject {
            value: lit.value.clone(),
        });
        self.constants.push(str);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })
    }
    fn compile_function_literal(&mut self, lit: &FunctionLiteral) {}
    fn compile_array_literal(&mut self, lit: &ArrayLiteral) {}
    fn compile_prefix_exp(&mut self, exp: &PrefixExpression) {
        self.compile_exp(&exp.right);

        match exp.token.kind {
            Kind::Bang => self.emit(Instruction::BANG),
            Kind::Minus => self.emit(Instruction::NEG),
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
            Kind::Plus => self.emit(Instruction::ADD),
            Kind::Minus => self.emit(Instruction::SUB),
            Kind::Product => self.emit(Instruction::PRODUCT),
            Kind::Divide => self.emit(Instruction::DIVIDE),
            Kind::Mod => self.emit(Instruction::MOD),
            Kind::LT => self.emit(Instruction::CLT),
            Kind::LT_OR_EQ => self.emit(Instruction::CLTE),
            Kind::GT => self.emit(Instruction::CGT),
            Kind::GT_OR_EQ => self.emit(Instruction::CGTE),
            Kind::EQ => self.emit(Instruction::CEQ),
            Kind::NOT_EQ => self.emit(Instruction::CNEQ),
            Kind::And => self.emit(Instruction::AND),
            Kind::Or => self.emit(Instruction::OR),
            Kind::Bit_And => self.emit(Instruction::BAND),
            Kind::Bit_Or => self.emit(Instruction::BOR),
            __not_matched => {
                // emit error
            }
        }
    }
    fn compile_if_exp(&mut self, exp: &IfExpression) {}
    fn compile_call_exp(&mut self, exp: &CallExpression) {}
    fn compile_index_exp(&mut self, exp: &IndexExpression) {}

    fn emit(&mut self, ins: Instruction) {
        self.instructions.add_instruction(ins)
    }
}
