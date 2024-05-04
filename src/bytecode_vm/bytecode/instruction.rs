use std::io::Write;

use super::opcode::OpCode;

/// ARG_SIZE
const ARG_NONE: [usize; 0] = [];
const ARG_CONST: [usize; 1] = [8];
const ARG_OFFSET: [usize; 1] = [8];
const ARG_CLOSURE: [usize; 2] = [8, 8];

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

pub const CLOSURE: Definition = Definition {
    length: 1 + 8 + 8,
    arg_size: &ARG_CLOSURE,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PUSH,
    POP,
    CONST {
        idx: usize,
    },
    DEFGLB {
        idx: usize,
    },
    GETGLB {
        idx: usize,
    },
    DEFLCL {
        idx: usize,
    },
    GETLCL {
        idx: usize,
    },
    GETFREE {
        idx: usize,
    },
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
    JMP {
        idx: usize,
    },
    JIS {
        idx: usize,
    },
    JNS {
        idx: usize,
    },
    JEQ {
        idx: usize,
    },
    JNEQ {
        idx: usize,
    },
    ARRAY {
        count: usize,
    },
    INDEX,
    CALL {
        arg_len: usize,
    },
    /// Return
    RETN,
    /// Return with value
    RETV,
    ///Create a closure
    CLOSURE {
        /// index of constant function
        idx: usize,
        /// free variable length
        free: usize,
    },
}

impl Instruction {
    pub fn opcode(&self) -> OpCode {
        match self {
            Instruction::PUSH => OpCode::PUSH,
            Instruction::POP => OpCode::POP,
            Instruction::CONST { idx: _ } => OpCode::CONST,
            Instruction::DEFGLB { idx: _ } => OpCode::DEFGLB,
            Instruction::GETGLB { idx: _ } => OpCode::GETGLB,
            Instruction::DEFLCL { idx: _ } => OpCode::DEFLCL,
            Instruction::GETLCL { idx: _ } => OpCode::GETLCL,
            Instruction::GETFREE { idx: _ } => OpCode::GETFREE,
            Instruction::ADD => OpCode::ADD,
            Instruction::SUB => OpCode::SUB,
            Instruction::PRODUCT => OpCode::PRODUCT,
            Instruction::DIVIDE => OpCode::DIVIDE,
            Instruction::MOD => OpCode::MOD,
            Instruction::BANG => OpCode::BANG,
            Instruction::NEG => OpCode::NEG,
            Instruction::CGT => OpCode::CGT,
            Instruction::CGTE => OpCode::CGTE,
            Instruction::CLT => OpCode::CLT,
            Instruction::CLTE => OpCode::CLTE,
            Instruction::CEQ => OpCode::CEQ,
            Instruction::CNEQ => OpCode::CNEQ,
            Instruction::AND => OpCode::AND,
            Instruction::OR => OpCode::OR,
            Instruction::BAND => OpCode::BAND,
            Instruction::BOR => OpCode::BOR,
            Instruction::JMP { idx: _ } => OpCode::JMP,
            Instruction::JIS { idx: _ } => OpCode::JIS,
            Instruction::JNS { idx: _ } => OpCode::JNS,
            Instruction::JEQ { idx: _ } => OpCode::JEQ,
            Instruction::JNEQ { idx: _ } => OpCode::JNEQ,
            Instruction::ARRAY { count: _ } => OpCode::ARRAY,
            Instruction::INDEX => OpCode::INDEX,
            Instruction::CALL { arg_len: _ } => OpCode::CALL,
            Instruction::RETN => OpCode::RETN,
            Instruction::RETV => OpCode::RETV,
            Instruction::CLOSURE { idx: _, free: _ } => OpCode::CLOSURE,
        }
    }

    pub fn as_byte(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Instruction::PUSH
            | Instruction::POP
            | Instruction::ADD
            | Instruction::SUB
            | Instruction::PRODUCT
            | Instruction::DIVIDE
            | Instruction::MOD
            | Instruction::BANG
            | Instruction::NEG
            | Instruction::CGT
            | Instruction::CGTE
            | Instruction::CLT
            | Instruction::CLTE
            | Instruction::CEQ
            | Instruction::CNEQ
            | Instruction::AND
            | Instruction::OR
            | Instruction::BAND
            | Instruction::BOR
            | Instruction::INDEX
            | Instruction::RETN
            | Instruction::RETV => buf.push(self.opcode() as u8),
            Instruction::CONST { idx }
            | Instruction::DEFGLB { idx }
            | Instruction::GETGLB { idx }
            | Instruction::DEFLCL { idx }
            | Instruction::GETLCL { idx }
            | Instruction::GETFREE { idx } => {
                buf.push(self.opcode() as u8);
                buf.write_all(&idx.to_ne_bytes()).unwrap()
            }
            Instruction::ARRAY { count } => {
                buf.push(self.opcode() as u8);
                buf.write_all(&count.to_ne_bytes()).unwrap()
            }

            Instruction::JMP { idx }
            | Instruction::JIS { idx }
            | Instruction::JNS { idx }
            | Instruction::JEQ { idx }
            | Instruction::JNEQ { idx } => {
                buf.push(self.opcode() as u8);
                buf.write_all(&idx.to_ne_bytes()).unwrap()
            }

            Instruction::CALL { arg_len } => {
                buf.push(self.opcode() as u8);
                buf.write_all(&arg_len.to_ne_bytes()).unwrap()
            }

            Instruction::CLOSURE { idx, free } => {
                buf.push(self.opcode() as u8);
                buf.write_all(&idx.to_ne_bytes()).unwrap();
                buf.write_all(&free.to_ne_bytes()).unwrap();
            }
        }

        buf
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();

        match self {
            Instruction::PUSH => buf += "PUSH",
            Instruction::POP => buf += "POP",
            Instruction::CONST { idx } => buf += &format!("CONST\t\t{idx}"),
            Instruction::DEFGLB { idx } => buf += &format!("DEFGLB\t\t{idx}"),
            Instruction::GETGLB { idx } => buf += &format!("GETGLB\t\t{idx}"),
            Instruction::DEFLCL { idx } => buf += &format!("DEFLCL\t\t{idx}"),
            Instruction::GETLCL { idx } => buf += &format!("GETLCL\t\t{idx}"),
            Instruction::GETFREE { idx } => buf += &format!("GETFREE\t\t{idx}"),

            Instruction::ADD => buf += "ADD",
            Instruction::SUB => buf += "SUB",
            Instruction::PRODUCT => buf += "PRODUCT",
            Instruction::DIVIDE => buf += "DIVIDE",
            Instruction::MOD => buf += "MOD",
            Instruction::BANG => buf += "BANG",
            Instruction::NEG => buf += "NEG",
            Instruction::CGT => buf += "CGT",
            Instruction::CGTE => buf += "CGTE",
            Instruction::CLT => buf += "CLT",
            Instruction::CLTE => buf += "CLTE",
            Instruction::CEQ => buf += "CEQ",
            Instruction::CNEQ => buf += "CNEQ",
            Instruction::AND => buf += "AND",
            Instruction::OR => buf += "OR",
            Instruction::BAND => buf += "BAND",
            Instruction::BOR => buf += "BOR",
            Instruction::JMP { idx } => buf += &format!("JMP\t\t{idx}"),
            Instruction::JIS { idx } => buf += &format!("JIS\t\t{idx}"),
            Instruction::JNS { idx } => buf += &format!("JNS\t\t{idx}"),
            Instruction::JEQ { idx } => buf += &format!("JEQ\t\t{idx}"),
            Instruction::JNEQ { idx } => buf += &format!("JNEQ\t\t{idx}"),
            Instruction::ARRAY { count } => buf += &format!("ARRAY\t\t{count}"),
            Instruction::INDEX => buf += &format!("INDEX"),
            Instruction::CALL { arg_len } => buf += &format!("CALL\t\t{arg_len}"),
            Instruction::RETN => buf += "RETN",
            Instruction::RETV => buf += "RETV",
            Instruction::CLOSURE { idx, free } => buf += &format!("CLOSURE\t\t{idx}\t{free}"),
        }

        buf
    }
}
