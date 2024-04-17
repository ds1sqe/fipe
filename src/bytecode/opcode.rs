use super::instruction::{CONST, JUMP, NO_ARG};

/// Raw opcode
#[derive(Debug)]
pub enum OpCode {
    PUSH,
    POP,
    CONST,
    ADD,
    SUB,
    PRODUCT,
    DIVIDE,
    MOD,
    BANG,
    NEG,
    CGT,
    CGTE,
    CLT,
    CLTE,
    CEQ,
    CNEQ,
    AND,
    OR,
    BAND,
    BOR,
    JMP,
    JEQ,
    JNEQ,
}

impl OpCode {
    pub fn arg_len(&self) -> &'static [usize] {
        match self {
            OpCode::PUSH
            | OpCode::POP
            | OpCode::ADD
            | OpCode::SUB
            | OpCode::PRODUCT
            | OpCode::DIVIDE
            | OpCode::MOD
            | OpCode::BANG
            | OpCode::NEG
            | OpCode::CGT
            | OpCode::CGTE
            | OpCode::CLT
            | OpCode::CLTE
            | OpCode::CEQ
            | OpCode::CNEQ
            | OpCode::AND
            | OpCode::OR
            | OpCode::BAND
            | OpCode::BOR => return &NO_ARG.arg_size,

            OpCode::CONST => return &CONST.arg_size,

            OpCode::JMP | OpCode::JEQ | OpCode::JNEQ => return &JUMP.arg_size,
        }
    }
    pub fn length(&self) -> usize {
        match self {
            OpCode::PUSH
            | OpCode::POP
            | OpCode::ADD
            | OpCode::SUB
            | OpCode::PRODUCT
            | OpCode::DIVIDE
            | OpCode::MOD
            | OpCode::BANG
            | OpCode::NEG
            | OpCode::CGT
            | OpCode::CGTE
            | OpCode::CLT
            | OpCode::CLTE
            | OpCode::CEQ
            | OpCode::CNEQ
            | OpCode::AND
            | OpCode::OR
            | OpCode::BAND
            | OpCode::BOR => return NO_ARG.length,

            OpCode::CONST => return CONST.length,

            OpCode::JMP | OpCode::JEQ | OpCode::JNEQ => return JUMP.length,
        }
    }
}
