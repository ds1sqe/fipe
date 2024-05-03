use super::instruction::{CONST, JUMP, NO_ARG};

/// Raw opcode
#[derive(Debug)]
pub enum OpCode {
    PUSH,
    POP,
    CONST,
    /// Define global
    DEFGLB,
    /// Get global
    GETGLB,
    /// Define local
    DEFLCL,
    /// Get local
    GETLCL,
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
    /// Logical AND
    AND,
    /// Logical OR
    OR,
    /// Bit AND
    BAND,
    /// Bit OR
    BOR,
    JMP,
    /// jump if state
    JIS,
    /// jump if not state
    JNS,
    JEQ,
    JNEQ,
    ARRAY,
    INDEX,
    CALL,
    /// Return
    RETN,
    /// Return with value
    RETV,
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
            | OpCode::BOR
            | OpCode::RETN
            | OpCode::RETV => return &NO_ARG.arg_size,

            OpCode::CONST
            | OpCode::DEFGLB
            | OpCode::GETGLB
            | OpCode::DEFLCL
            | OpCode::GETLCL
            | OpCode::CALL
            | OpCode::ARRAY
            | OpCode::INDEX => return &CONST.arg_size,

            OpCode::JMP
            | OpCode::JIS
            | OpCode::JNS
            | OpCode::JEQ
            | OpCode::JNEQ => return &JUMP.arg_size,
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
            | OpCode::BOR
            | OpCode::RETN
            | OpCode::RETV => return NO_ARG.length,

            OpCode::CONST
            | OpCode::DEFGLB
            | OpCode::GETGLB
            | OpCode::DEFLCL
            | OpCode::GETLCL
            | OpCode::CALL
            | OpCode::ARRAY
            | OpCode::INDEX => return CONST.length,

            OpCode::JMP
            | OpCode::JIS
            | OpCode::JNS
            | OpCode::JEQ
            | OpCode::JNEQ => return JUMP.length,
        }
    }
}
