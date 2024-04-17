use super::opcode::OpCode;

/// ARG_SIZE
const ARG_NONE: [usize; 0] = [];
const ARG_CONST: [usize; 1] = [8];
const ARG_OFFSET: [usize; 1] = [8];

/// Definition of instruction's length and arg
pub struct Definition {
    pub length: usize,
    pub arg_size: &'static [usize],
}

pub const NO_ARG: Definition = Definition {
    length: 1,
    arg_size: &ARG_NONE,
};

pub const CONST: Definition = Definition {
    length: 1 + 8,
    arg_size: &ARG_CONST,
};

pub const JUMP: Definition = Definition {
    length: 1 + 8,
    arg_size: &ARG_OFFSET,
};

pub enum Instruction {
    PUSH,
    POP,
    CONST { idx: usize },
    ADD,
    SUB,
    PRODUCT,
    DIVIDE,
    MOD,
    BANG,
    NEG,
    CGT,
    CLT,
    CEQ,
    CNEQ,
    JMP { idx: usize },
    JEQ { idx: usize },
    JNEQ { idx: usize },
}

impl Instruction {
    pub fn opcode(&self) -> OpCode {
        match self {
            Instruction::PUSH => OpCode::PUSH,
            Instruction::POP => OpCode::POP,
            Instruction::CONST { idx: _ } => OpCode::CONST,
            Instruction::ADD => OpCode::ADD,
            Instruction::SUB => OpCode::SUB,
            Instruction::PRODUCT => OpCode::PRODUCT,
            Instruction::DIVIDE => OpCode::DIVIDE,
            Instruction::MOD => OpCode::MOD,
            Instruction::BANG => OpCode::BANG,
            Instruction::NEG => OpCode::NEG,
            Instruction::CGT => OpCode::CGT,
            Instruction::CLT => OpCode::CLT,
            Instruction::CEQ => OpCode::CEQ,
            Instruction::CNEQ => OpCode::CNEQ,
            Instruction::JMP { idx: _ } => OpCode::JMP,
            Instruction::JEQ { idx: _ } => OpCode::JEQ,
            Instruction::JNEQ { idx: _ } => OpCode::JNEQ,
        }
    }
}
