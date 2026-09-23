use super::instruction::{CLOSURE, CONST, JUMP, NO_ARG};

/// Raw opcode
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
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
    GETFREE,
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
    /// Create a closure
    CLOSURE,
    /// Get current function
    GETCUR,
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
            | OpCode::RETV
            | OpCode::INDEX
            | OpCode::GETCUR => NO_ARG.arg_size,

            OpCode::CONST
            | OpCode::DEFGLB
            | OpCode::GETGLB
            | OpCode::DEFLCL
            | OpCode::GETLCL
            | OpCode::GETFREE
            | OpCode::CALL
            | OpCode::ARRAY => CONST.arg_size,

            OpCode::JMP
            | OpCode::JIS
            | OpCode::JNS
            | OpCode::JEQ
            | OpCode::JNEQ => JUMP.arg_size,

            OpCode::CLOSURE => CLOSURE.arg_size,
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
            | OpCode::RETV
            | OpCode::INDEX
            | OpCode::GETCUR => NO_ARG.length,

            OpCode::CONST
            | OpCode::DEFGLB
            | OpCode::GETGLB
            | OpCode::DEFLCL
            | OpCode::GETLCL
            | OpCode::GETFREE
            | OpCode::CALL
            | OpCode::ARRAY => CONST.length,

            OpCode::JMP
            | OpCode::JIS
            | OpCode::JNS
            | OpCode::JEQ
            | OpCode::JNEQ => JUMP.length,

            OpCode::CLOSURE => CLOSURE.length,
        }
    }
}

impl TryFrom<u8> for OpCode {
    type Error = ();

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        match byte {
            value if value == Self::PUSH as u8 => Ok(Self::PUSH),
            value if value == Self::POP as u8 => Ok(Self::POP),
            value if value == Self::CONST as u8 => Ok(Self::CONST),
            value if value == Self::DEFGLB as u8 => Ok(Self::DEFGLB),
            value if value == Self::GETGLB as u8 => Ok(Self::GETGLB),
            value if value == Self::DEFLCL as u8 => Ok(Self::DEFLCL),
            value if value == Self::GETLCL as u8 => Ok(Self::GETLCL),
            value if value == Self::GETFREE as u8 => Ok(Self::GETFREE),
            value if value == Self::ADD as u8 => Ok(Self::ADD),
            value if value == Self::SUB as u8 => Ok(Self::SUB),
            value if value == Self::PRODUCT as u8 => Ok(Self::PRODUCT),
            value if value == Self::DIVIDE as u8 => Ok(Self::DIVIDE),
            value if value == Self::MOD as u8 => Ok(Self::MOD),
            value if value == Self::BANG as u8 => Ok(Self::BANG),
            value if value == Self::NEG as u8 => Ok(Self::NEG),
            value if value == Self::CGT as u8 => Ok(Self::CGT),
            value if value == Self::CGTE as u8 => Ok(Self::CGTE),
            value if value == Self::CLT as u8 => Ok(Self::CLT),
            value if value == Self::CLTE as u8 => Ok(Self::CLTE),
            value if value == Self::CEQ as u8 => Ok(Self::CEQ),
            value if value == Self::CNEQ as u8 => Ok(Self::CNEQ),
            value if value == Self::AND as u8 => Ok(Self::AND),
            value if value == Self::OR as u8 => Ok(Self::OR),
            value if value == Self::BAND as u8 => Ok(Self::BAND),
            value if value == Self::BOR as u8 => Ok(Self::BOR),
            value if value == Self::JMP as u8 => Ok(Self::JMP),
            value if value == Self::JIS as u8 => Ok(Self::JIS),
            value if value == Self::JNS as u8 => Ok(Self::JNS),
            value if value == Self::JEQ as u8 => Ok(Self::JEQ),
            value if value == Self::JNEQ as u8 => Ok(Self::JNEQ),
            value if value == Self::ARRAY as u8 => Ok(Self::ARRAY),
            value if value == Self::INDEX as u8 => Ok(Self::INDEX),
            value if value == Self::CALL as u8 => Ok(Self::CALL),
            value if value == Self::RETN as u8 => Ok(Self::RETN),
            value if value == Self::RETV as u8 => Ok(Self::RETV),
            value if value == Self::CLOSURE as u8 => Ok(Self::CLOSURE),
            value if value == Self::GETCUR as u8 => Ok(Self::GETCUR),
            _ => Err(()),
        }
    }
}
