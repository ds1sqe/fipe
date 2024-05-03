use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression, Expression,
        ExpressionStatement, FunctionLiteral, Identifier, IfExpression, IndexExpression,
        InfixExpression, IntegerLiteral, LetStatement, PrefixExpression, Program, ReturnStatement,
        Statement, StringLiteral,
    },
    object::{Bool, CompiledFunction, Int, Object, StringObject},
    token::Kind,
};

use super::{instruction::Instruction, instructions::Instructions, symbol::SymbolTable, Bytecode};

#[derive(Debug, Clone)]
struct Scope {
    instructions: Instructions,
}

#[derive(Debug)]
struct InstructionInfo {
    instruction: Instruction,
    pos: usize,
}

#[derive(Debug)]
pub struct Compiler {
    constants: Vec<Object>,

    scopes: Vec<Scope>,
    scope_idx: usize,
    symbol_table: Option<SymbolTable>,

    last_instruction: Option<InstructionInfo>,
}

impl Compiler {
    pub fn new() -> Self {
        let mut scopes = Vec::new();
        scopes.push(Scope {
            instructions: Instructions::new(),
        });

        Self {
            constants: Vec::new(),

            scopes,
            scope_idx: 0,

            symbol_table: Some(SymbolTable::new()),

            last_instruction: None,
        }
    }

    pub fn bytecode(self) -> Bytecode {
        if self.scope_idx != 0 {
            // emit error (have to be 0 which means main-global )
            panic!("scope is not 0")
        }
        Bytecode {
            constants: self.constants.clone(),
            instructions: self.current_scope().instructions.clone(),
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
            Expression::FunctionLiteral(lit) => self.compile_function_literal(lit),
            Expression::ArrayLiteral(lit) => self.compile_array_literal(lit),
            Expression::InfixExpression(exp) => self.compile_infix_exp(exp),
            Expression::PrefixExpression(exp) => self.compile_prefix_exp(exp),
            Expression::IfExpression(exp) => self.compile_if_exp(exp),
            Expression::CallExpression(exp) => self.compile_call_exp(exp),
            Expression::IndexExpression(exp) => self.compile_index_exp(exp),
        }
    }

    fn compile_expression_stm(&mut self, stm: &ExpressionStatement) {
        self.compile_exp(&stm.expression.as_ref().unwrap());
        self.emit(Instruction::POP);
    }
    fn compile_let_stm(&mut self, stm: &LetStatement) {
        self.compile_exp(&stm.value.clone().unwrap());

        let idx = self
            .symbol_table
            .as_mut()
            .unwrap()
            .define(&stm.identifier.value);

        if self.symbol_table.as_ref().unwrap().is_global() {
            self.emit(Instruction::DEFGLB { idx });
        } else {
            self.emit(Instruction::DEFLCL { idx });
        }
    }
    fn compile_return_stm(&mut self, stm: &ReturnStatement) {
        match &stm.value {
            Some(exp) => {
                self.compile_exp(exp);
                self.emit(Instruction::RETV);
            }
            None => {
                self.emit(Instruction::RETN);
            }
        }
    }
    fn compile_block_stm(&mut self, stm: &BlockStatement) {
        for statement in &stm.statements {
            self.compile_stm(statement);
        }
    }

    fn compile_identifier_exp(&mut self, exp: &Identifier) {
        let rst = self.symbol_table.as_mut().unwrap().resolve(&exp.value);
        if rst.is_some() {
            let idx = rst.unwrap().index;
            if rst.unwrap().is_global() {
                self.emit(Instruction::GETGLB { idx });
            } else {
                self.emit(Instruction::GETLCL { idx });
            }
        } else {
            // emit error
            panic!("Identifier not found!!! {:?}", &exp);
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
    fn compile_function_literal(&mut self, lit: &FunctionLiteral) {
        self.enter_scope();

        for param in &lit.parameters {
            self.symbol_table.as_mut().unwrap().define(&param.value);
        }

        self.compile_block_stm(&lit.body);

        if !self.update_last_instruction_if(Instruction::POP, Instruction::RETV) {
            self.emit(Instruction::RETN);
        }

        let local_len = self.symbol_table.as_ref().unwrap().len;
        let body_scope = self.leave_scope();

        let compiled_function = CompiledFunction {
            arg_len: lit.parameters.len(),
            local_len,
            instructions: body_scope.instructions,
        };

        self.constants
            .push(Object::CompiledFunction(compiled_function));
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        });

        if lit.ident.is_some() {
            let idx = self
                .symbol_table
                .as_mut()
                .unwrap()
                .define(&lit.ident.as_ref().unwrap().value);

            if self.symbol_table.as_ref().unwrap().is_global() {
                self.emit(Instruction::DEFGLB { idx });
            } else {
                self.emit(Instruction::DEFLCL { idx });
            }
        }
    }

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

        self.remove_last_instruction_if(Instruction::POP);

        if exp.alternative.is_some() {
            let jump_alternative = self.emit(Instruction::JMP { idx: 0 });

            self.update(
                Instruction::JNS {
                    idx: self.next_offset(),
                },
                jump_consequence,
            );

            self.compile_stm(&Statement::BlockStatement(exp.alternative.clone().unwrap()));
            self.remove_last_instruction_if(Instruction::POP);

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
    fn compile_call_exp(&mut self, exp: &CallExpression) {
        self.compile_exp(&exp.function);

        // expected instruction
        // 000 Function
        // 001 DEFLCC local 1
        // 002 DEFLCC local 2
        // 003 DEFLCC arg 1
        // 004 DEFLCC arg 2
        // 005 CALL

        // expected stack
        // 000 Function (bp)
        // 001 local 1
        // 002 local 2
        // 003 arg 1
        // 004 arg 2
        // 005 LOCAL STACK

        for arg in &exp.arguments {
            self.compile_exp(arg)
        }

        self.emit(Instruction::CALL {
            arg_len: exp.arguments.len(),
        });
    }

    fn compile_index_exp(&mut self, exp: &IndexExpression) {
        self.compile_exp(&exp.left);
        self.compile_exp(&exp.index);

        self.emit(Instruction::INDEX);
    }

    fn emit(&mut self, ins: Instruction) -> usize {
        let pos = self
            .current_scope_mut()
            .instructions
            .add_instruction(ins.clone());

        self.last_instruction = Some(InstructionInfo {
            instruction: ins,
            pos,
        });

        pos
    }
    fn update(&mut self, ins: Instruction, offset: usize) {
        self.current_scope_mut()
            .instructions
            .update_instruction(ins, offset)
    }

    fn remove_last_instruction(&mut self) {
        let new_length = self.last_instruction.as_ref().unwrap().pos;
        self.current_scope_mut()
            .instructions
            .remove_instruction(new_length);
        self.last_instruction = None;
    }

    /// Update last_instruction if (ins_info.instruction == ins) || (ins_info.instruction == with)
    ///
    /// returns true if have changed instruction
    fn update_last_instruction_if(&mut self, ins: Instruction, with: Instruction) -> bool {
        if self
            .last_instruction
            .as_ref()
            .is_some_and(|ins_info| (ins_info.instruction == ins) || (ins_info.instruction == with))
        {
            let pos = self.last_instruction.as_ref().unwrap().pos;
            self.update(with.clone(), pos);
            self.last_instruction = Some(InstructionInfo {
                instruction: with,
                pos,
            });

            return true;
        }
        false
    }

    fn remove_last_instruction_if(&mut self, ins: Instruction) {
        if self
            .last_instruction
            .as_ref()
            .is_some_and(|ins_info| ins_info.instruction == ins)
        {
            self.remove_last_instruction();
            //self.update(,self.last_instruction.unwrap().pos)
        }
    }

    fn next_offset(&self) -> usize {
        self.current_scope().instructions.length()
    }

    /// create new scope and enclose current `self.symbol_table`
    fn enter_scope(&mut self) {
        let new_scope = Scope {
            instructions: Instructions::new(),
        };
        self.scopes.push(new_scope);
        self.scope_idx += 1;

        self.symbol_table = Some(SymbolTable::enclose(self.symbol_table.take().unwrap()));
    }

    /// Leave [`Scope`] of this [`Compiler`]
    /// and return previous [`Scope`]
    ///
    /// # Panics
    ///
    /// Panics if scopes length below 1 ( len < 1 )
    fn leave_scope(&mut self) -> Scope {
        self.symbol_table = Some(self.symbol_table.as_mut().unwrap().get_outer());
        self.scope_idx -= 1;
        self.scopes.pop().unwrap()
    }
    fn current_scope(&self) -> &Scope {
        &self.scopes[self.scope_idx]
    }

    fn current_scope_mut(&mut self) -> &mut Scope {
        &mut self.scopes[self.scope_idx]
    }
}
