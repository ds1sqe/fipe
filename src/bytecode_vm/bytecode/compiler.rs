use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression,
        IndexExpression, InfixExpression, IntegerLiteral, LetStatement,
        PrefixExpression, Program, ReturnStatement, Statement, StringLiteral,
    },
    object::{Bool, Int, Object, StringObject},
    token::Kind,
};

use super::{
    instruction::Instruction, instructions::Instructions, symbol::SymbolTable,
    Bytecode,
};

struct Scope {
    instructions: Instructions,
}

pub struct Compiler {
    constants: Vec<Object>,

    scopes: Vec<Scope>,
    scope_idx: usize,
    instructions: Instructions,
    symbol_table: SymbolTable,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),

            scopes: Vec::new(),
            scope_idx: 0,
            instructions: Instructions::new(),
            symbol_table: SymbolTable::new(),
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
            Expression::ArrayLiteral(lit) => self.compile_array_literal(lit),
            Expression::InfixExpression(exp) => self.compile_infix_exp(exp),
            Expression::PrefixExpression(exp) => self.compile_prefix_exp(exp),
            Expression::IfExpression(exp) => self.compile_if_exp(exp),
            Expression::CallExpression(_) => todo!(),
            Expression::IndexExpression(exp) => self.compile_index_exp(exp),
        }
    }

    fn compile_expression_stm(&mut self, stm: &ExpressionStatement) {
        self.compile_exp(&stm.expression.as_ref().unwrap())
    }
    fn compile_let_stm(&mut self, stm: &LetStatement) {
        self.compile_exp(&stm.value.clone().unwrap());

        let idx = self.symbol_table.define_global(&stm.identifier.value);

        self.emit(Instruction::DEFGLB { idx });
    }
    fn compile_return_stm(&mut self, stm: &ReturnStatement) {}
    fn compile_block_stm(&mut self, stm: &BlockStatement) {
        for statement in &stm.statements {
            self.compile_stm(statement);
        }
    }

    fn compile_identifier_exp(&mut self, exp: &Identifier) {
        let rst = self.symbol_table.get_global(&exp.value);
        if rst.is_some() {
            let idx = rst.unwrap().index;
            self.emit(Instruction::GETGLB { idx });
        } else {
            // emit error
            todo!();
        }
    }
    fn compile_integer_literal(&mut self, lit: &IntegerLiteral) {
        let int = Object::Int(Int { value: lit.value });
        self.constants.push(int);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        });
    }
    fn compile_bool_literal(&mut self, lit: &BooleanLiteral) {
        let boolean = Object::Bool(Bool { value: lit.value });
        self.constants.push(boolean);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        });
    }
    fn compile_string_literal(&mut self, lit: &StringLiteral) {
        let str = Object::String(StringObject {
            value: lit.value.clone(),
        });
        self.constants.push(str);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        });
    }
    fn compile_function_literal(&mut self, lit: &FunctionLiteral) {}

    fn compile_array_literal(&mut self, lit: &ArrayLiteral) {
        for el in lit.elements.iter() {
            self.compile_exp(el);
        }
        self.emit(Instruction::ARRAY {
            count: lit.elements.len(),
        });
    }

    fn compile_prefix_exp(&mut self, exp: &PrefixExpression) {
        self.compile_exp(&exp.right);

        match exp.token.kind {
            Kind::Bang => self.emit(Instruction::BANG),
            Kind::Minus => self.emit(Instruction::NEG),
            __not_matched => {
                // emit error
                todo!()
            }
        };
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
                todo!()
            }
        };
    }
    fn compile_if_exp(&mut self, exp: &IfExpression) {
        self.compile_exp(&exp.condition);
        let jump_consequence = self.emit(Instruction::JNS { idx: 0 });
        self.compile_stm(&Statement::BlockStatement(exp.consequence.clone()));

        if exp.alternative.is_some() {
            let jump_alternative = self.emit(Instruction::JMP { idx: 0 });

            self.update(
                Instruction::JNS {
                    idx: self.next_offset(),
                },
                jump_consequence,
            );

            self.compile_stm(&Statement::BlockStatement(
                exp.alternative.clone().unwrap(),
            ));

            self.update(
                Instruction::JMP {
                    idx: self.next_offset(),
                },
                jump_alternative,
            );
        } else {
            self.update(
                Instruction::JNS {
                    idx: self.next_offset(),
                },
                jump_consequence,
            );
        }
    }
    fn compile_call_exp(&mut self, exp: &CallExpression) {}
    fn compile_index_exp(&mut self, exp: &IndexExpression) {
        self.compile_exp(&exp.left);
        self.compile_exp(&exp.index);

        self.emit(Instruction::INDEX);
    }

    fn emit(&mut self, ins: Instruction) -> usize {
        self.instructions.add_instruction(ins)
    }
    fn update(&mut self, ins: Instruction, offset: usize) {
        self.instructions.update_instruction(ins, offset)
    }

    fn next_offset(&self) -> usize {
        self.instructions.length()
    }
}
