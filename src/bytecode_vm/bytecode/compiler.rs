use crate::{
    ast::{
        ArrayLiteral, BlockStatement, BooleanLiteral, CallExpression,
        Expression, ExpressionStatement, FunctionLiteral, Identifier,
        IfExpression, IndexExpression, InfixExpression, IntegerLiteral,
        LetStatement, PrefixExpression, Program, ReturnStatement, Statement,
        StringLiteral,
    },
    object::{Bool, CompiledFunction, Int, Object, StringObject},
    token::Kind,
};

use super::{
    errors::CompileError,
    instruction::Instruction,
    instructions::Instructions,
    symbol::{self, Symbol, SymbolTable},
    Bytecode,
};

const SUCCESS: Result<bool, CompileError> = Ok(true);

/// Hold current function's instructions
#[derive(Debug, Clone)]
struct Scope {
    /// current function's instructions
    instructions: Instructions,
}

impl Scope {
    pub fn new(instructions: Instructions) -> Self {
        Self { instructions }
    }
}

#[derive(Debug)]
struct InstructionInfo {
    instruction: Instruction,
    pos: usize,
}

#[derive(Debug)]
pub struct Compiler {
    /// constants of Program
    constants: Vec<Object>,

    /// stack memory of function's instruction aka [`Scope`]
    scopes: Vec<Scope>,

    /// index of current_function's scope
    scope_idx: usize,

    /// symbol_table which holds symbol.
    /// never be None
    symbol_table: Option<SymbolTable>,

    /// last written instruction's information
    last_instruction: Option<InstructionInfo>,
}

impl Compiler {
    /// Creates a new [`Compiler`].
    ///
    /// # Panics
    ///
    /// May Panics if OOM
    ///
    /// # Errors [`CompileError::CreationFailed`]
    ///
    /// This function will return an error if inner instructions creation
    /// have failed
    pub fn create() -> Result<Self, CompileError> {
        let mut scopes = Vec::new();
        let inst_rst = Instructions::create();
        if inst_rst.is_err() {
            return Err(CompileError::CreationFailed(inst_rst.unwrap_err()));
        }
        scopes.push(
            // this Scope is will be main function
            Scope {
                instructions: inst_rst.unwrap(),
            },
        );

        Ok(Self {
            constants: Vec::new(),

            scopes,
            scope_idx: 0,

            symbol_table: Some(SymbolTable::new()),

            last_instruction: None,
        })
    }

    /// Generate bytecode as compile result
    ///
    /// # Errors [`CompileError::NotFinishedInMain`]
    ///
    /// This function will return an error if
    /// scope_idx is not 0 ( it means that compiling had not ended in main )
    pub fn bytecode(self) -> Result<Bytecode, CompileError> {
        if self.scope_idx != 0 {
            return Err(CompileError::NotFinishedInMain);
        }
        Ok(Bytecode {
            constants: self.constants.clone(),
            instructions: self.current_scope().instructions.clone(),
        })
    }

    /// compile given src: Program
    ///
    /// # Errors [`CompileError`]
    ///
    /// This function will return an error if inner function failed.
    ///
    /// if have successfully compiled, returns Ok(True)
    pub fn compile(&mut self, src: Program) -> Result<bool, CompileError> {
        for stm in src.statements {
            match self.compile_stm(&stm) {
                Err(e) => return Err(e),
                Ok(_) => (),
            }
        }

        SUCCESS
    }

    /// compile statement
    fn compile_stm(&mut self, stm: &Statement) -> Result<bool, CompileError> {
        match stm {
            Statement::ExpressionStatement(stm) => {
                self.compile_expression_stm(stm)
            }
            Statement::LetStatement(stm) => self.compile_let_stm(stm),
            Statement::ReturnStatement(stm) => self.compile_return_stm(stm),
            Statement::BlockStatement(stm) => self.compile_block_stm(stm),
        }
    }

    /// compile expression
    fn compile_exp(&mut self, exp: &Expression) -> Result<bool, CompileError> {
        match exp {
            Expression::Identifier(exp) => self.compile_identifier_exp(exp),
            Expression::IntegerLiteral(lit) => {
                self.compile_integer_literal(lit)
            }
            Expression::BooleanLiteral(lit) => self.compile_bool_literal(lit),
            Expression::StringLiteral(lit) => self.compile_string_literal(lit),
            Expression::FunctionLiteral(lit) => {
                self.compile_function_literal(lit)
            }
            Expression::ArrayLiteral(lit) => self.compile_array_literal(lit),
            Expression::InfixExpression(exp) => self.compile_infix_exp(exp),
            Expression::PrefixExpression(exp) => self.compile_prefix_exp(exp),
            Expression::IfExpression(exp) => self.compile_if_exp(exp),
            Expression::CallExpression(exp) => self.compile_call_exp(exp),
            Expression::IndexExpression(exp) => self.compile_index_exp(exp),
        }
    }

    /// compile expression statement
    fn compile_expression_stm(
        &mut self,
        stm: &ExpressionStatement,
    ) -> Result<bool, CompileError> {
        self.compile_exp(&stm.expression.as_ref().unwrap())?;
        self.emit(Instruction::POP);
        SUCCESS
    }

    /// compile let statement
    fn compile_let_stm(
        &mut self,
        stm: &LetStatement,
    ) -> Result<bool, CompileError> {
        let idx = self
            .symbol_table
            .as_mut()
            .unwrap()
            .define(&stm.identifier.value);

        self.compile_exp(&stm.value.clone().unwrap())?;

        if self.symbol_table.as_ref().unwrap().is_global() {
            self.emit(Instruction::DEFGLB { idx });
        } else {
            self.emit(Instruction::DEFLCL { idx });
        }

        SUCCESS
    }

    /// compile return statement
    fn compile_return_stm(
        &mut self,
        stm: &ReturnStatement,
    ) -> Result<bool, CompileError> {
        match &stm.value {
            Some(exp) => {
                self.compile_exp(exp)?;
                self.emit(Instruction::RETV);
            }
            None => {
                self.emit(Instruction::RETN);
            }
        }
        SUCCESS
    }

    /// compile block statement
    fn compile_block_stm(
        &mut self,
        stm: &BlockStatement,
    ) -> Result<bool, CompileError> {
        for statement in &stm.statements {
            self.compile_stm(statement)?;
        }
        SUCCESS
    }

    /// Compile given identifier expression.
    /// try to find a symbol with given identifier,
    /// and if have found, emit load symbol instruction.
    ///
    /// # Errors [CompileError::IdentifierNotFound]
    /// This function will return an error if identifier not found on [`self.symbol_table`]
    fn compile_identifier_exp(
        &mut self,
        exp: &Identifier,
    ) -> Result<bool, CompileError> {
        let rst = self.symbol_table.as_mut().unwrap().resolve(&exp.value);
        if rst.is_some() {
            let sym = rst.unwrap();
            self.load_symbol(&sym);

            SUCCESS
        } else {
            Err(CompileError::IdentifierNotFound(exp.clone()))
        }
    }

    /// Compile given IntegerLiteral
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_integer_literal(
        &mut self,
        lit: &IntegerLiteral,
    ) -> Result<bool, CompileError> {
        let int = Object::Int(Int { value: lit.value });
        self.constants.push(int); // enlist Int to constant pool
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })?;

        SUCCESS
    }

    /// Compile given bool literal
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_bool_literal(
        &mut self,
        lit: &BooleanLiteral,
    ) -> Result<bool, CompileError> {
        let boolean = Object::Bool(Bool { value: lit.value });
        self.constants.push(boolean); // enlist boolean to constant pool
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })?;

        SUCCESS
    }
    /// Compile given string literal
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_string_literal(
        &mut self,
        lit: &StringLiteral,
    ) -> Result<bool, CompileError> {
        let str = Object::String(StringObject {
            value: lit.value.clone(),
        });
        self.constants.push(str);
        self.emit(Instruction::CONST {
            idx: self.constants.len() - 1,
        })?;

        SUCCESS
    }

    /// Compile given function literal
    ///
    /// # Errors
    ///
    /// This function will return an error if failed to compile
    fn compile_function_literal(
        &mut self,
        lit: &FunctionLiteral,
    ) -> Result<bool, CompileError> {
        self.enter_scope();

        if lit.ident.is_some() {
            unsafe {
                self.symbol_table
                    .as_mut()
                    // this is safe because it's never be None
                    .unwrap_unchecked()
                    .define_function_name(&lit.ident.as_ref().unwrap().value);
            }
        }

        // Instruction example

        // parameters
        // 001 GETLCL 0 (param1)
        // 002 GETLCL 1 (param2)

        for param in &lit.parameters {
            self.symbol_table.as_mut().unwrap().define(&param.value);
        }

        // parameters
        // 003 DEFLCL 2 (local1)
        // 004 DEFLCL 3 (local2)
        // 005 GETLCL 3 (local2)
        // 006 GETLCL 2 (local1)

        self.compile_block_stm(&lit.body)?;

        // if the last instuction is POP then
        //  replace it with RETV
        //  which means it's have ended with expression statement.
        //  it's implicitly
        //
        if !self.update_last_instruction_if(Instruction::POP, Instruction::RETV)
        {
            self.emit(Instruction::RETN);
        }

        let free_syms = self.symbol_table.as_ref().unwrap().get_free();

        let local_len = self.symbol_table.as_ref().unwrap().len;
        let body_scope = self.leave_scope();
        // free variable (local variable of outer function)
        // 007 GETFREE 0 (free1)
        // 008 GETFREE 1 (free2)

        for free in free_syms.iter() {
            self.load_symbol(free);
        }

        let compiled_function = CompiledFunction {
            arg_len: lit.parameters.len(),
            local_len,
            instructions: body_scope.instructions,
        };

        self.constants
            .push(Object::CompiledFunction(compiled_function));

        self.emit(Instruction::CLOSURE {
            idx: self.constants.len() - 1,
            free: free_syms.len(),
        });

        // if function has identifier and have't let bind
        if lit.ident.is_some() && !lit.is_let_bind {
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

        SUCCESS
    }

    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_array_literal(
        &mut self,
        lit: &ArrayLiteral,
    ) -> Result<bool, CompileError> {
        for el in lit.elements.iter() {
            self.compile_exp(el);
        }
        self.emit(Instruction::ARRAY {
            count: lit.elements.len(),
        });

        SUCCESS
    }

    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_prefix_exp(
        &mut self,
        exp: &PrefixExpression,
    ) -> Result<bool, CompileError> {
        self.compile_exp(&exp.right);

        match exp.token.kind {
            Kind::Bang => self.emit(Instruction::BANG),
            Kind::Minus => self.emit(Instruction::NEG),
            __not_matched => {
                // emit error
                todo!()
            }
        };

        SUCCESS
    }
    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_infix_exp(
        &mut self,
        exp: &InfixExpression,
    ) -> Result<bool, CompileError> {
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

        SUCCESS
    }
    /// .
    ///
    /// # Panics
    ///
    /// Panics if .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_if_exp(
        &mut self,
        exp: &IfExpression,
    ) -> Result<bool, CompileError> {
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

            self.compile_stm(&Statement::BlockStatement(
                exp.alternative.clone().unwrap(),
            ));
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

        SUCCESS
    }
    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_call_exp(
        &mut self,
        exp: &CallExpression,
    ) -> Result<bool, CompileError> {
        self.compile_exp(&exp.function);

        // example of instruction
        // 000 Function
        // 001 GETLCC arg 1
        // 002 GETLCC arg 2
        // 003 GETLCC local 1
        // 004 GETLCC local 2
        // 005 GETFREE free1
        // 006 GETFREE free2
        // 007 CALL

        // expected stack
        // 000 Function (bp)
        // 001 arg 1
        // 002 arg 2
        // 003 local 1
        // 004 local 2
        // 005 free1
        // 006 free2
        // 007 LOCAL STACK

        for arg in &exp.arguments {
            self.compile_exp(arg)
        }

        self.emit(Instruction::CALL {
            arg_len: exp.arguments.len(),
        });

        SUCCESS
    }

    /// .
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    fn compile_index_exp(
        &mut self,
        exp: &IndexExpression,
    ) -> Result<bool, CompileError> {
        self.compile_exp(&exp.left);
        self.compile_exp(&exp.index);

        self.emit(Instruction::INDEX);

        SUCCESS
    }

    fn emit(&mut self, ins: Instruction) -> Result<usize, CompileError> {
        let res = self
            .current_scope_mut()
            .instructions
            .add_instruction(ins.clone());

        if res.is_err() {
            return Err(CompileError::InstructionWriteError(res.unwrap_err()));
        }

        let pos = res.unwrap();

        self.last_instruction = Some(InstructionInfo {
            instruction: ins,
            pos,
        });

        Ok(pos)
    }
    /// Update instruction at `offset` with given `ins`
    ///
    /// # Safety
    ///
    /// this function is unsafe because it call [`Instructions::update_instruction`]
    ///
    unsafe fn update(&mut self, ins: Instruction, offset: usize) {
        self.current_scope_mut()
            .instructions
            .update_instruction(ins, offset)
    }

    /// Returns the remove last instruction of this [`Compiler`].
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
    fn update_last_instruction_if(
        &mut self,
        ins: Instruction,
        with: Instruction,
    ) -> bool {
        if self.last_instruction.as_ref().is_some_and(|ins_info| {
            (ins_info.instruction == ins) || (ins_info.instruction == with)
        }) {
            let pos = self.last_instruction.as_ref().unwrap().pos;

            unsafe {
                self.update(with.clone(), pos);
            }

            self.last_instruction = Some(InstructionInfo {
                instruction: with,
                pos,
            });

            return true;
        }
        false
    }

    /// Remove last instruction if `self.last_instruction == ins`
    fn remove_last_instruction_if(&mut self, ins: Instruction) {
        if self
            .last_instruction
            .as_ref()
            .is_some_and(|ins_info| ins_info.instruction == ins)
        {
            self.remove_last_instruction();
        }
    }

    /// Returns the next offset of this [`Compiler`].
    fn next_offset(&self) -> usize {
        self.current_scope().instructions.length()
    }

    /// create new scope and enclose current `self.symbol_table`
    fn enter_scope(&mut self) -> Result<bool, CompileError> {
        let inst_rst = Instructions::create();
        if inst_rst.is_err() {
            // this is safe
            unsafe {
                return Err(CompileError::ScopeCreateFaild(
                    // this will prohibit redundant checking
                    inst_rst.unwrap_err_unchecked(),
                ));
            }
        }
        let new_scope = Scope::new(unsafe { inst_rst.unwrap_unchecked() });

        self.scopes.push(new_scope);
        self.scope_idx += 1;

        self.symbol_table =
            Some(SymbolTable::enclose(self.symbol_table.take().unwrap()));
    }

    /// Leave [`Scope`] of this [`Compiler`]
    /// and return previous [`Scope`]
    ///
    /// # Panics
    ///
    /// Panics if scopes length below 1 ( len < 1 )
    fn leave_scope(&mut self) -> Scope {
        self.symbol_table =
            Some(self.symbol_table.as_mut().unwrap().get_outer());
        self.scope_idx -= 1;
        self.scopes.pop().unwrap()
    }
    /// Returns a reference to the current scope of this [`Compiler`].
    fn current_scope(&self) -> &Scope {
        &self.scopes[self.scope_idx]
    }

    /// Returns a mutable reference to the current scope of this [`Compiler`].
    fn current_scope_mut(&mut self) -> &mut Scope {
        &mut self.scopes[self.scope_idx]
    }

    /// .
    fn load_symbol(&mut self, sym: &Symbol) {
        let inst = match sym.scope() {
            symbol::Scope::Global => Instruction::GETGLB { idx: sym.index },
            symbol::Scope::Local => Instruction::GETLCL { idx: sym.index },
            symbol::Scope::Free => Instruction::GETFREE { idx: sym.index },
            symbol::Scope::Function => Instruction::GETCUR,
        };

        self.emit(inst);
    }
}
