use crate::object::Object;

pub enum Opcode {
    PUSH,
    POP,
    CONST { idx: usize },
    ADD,
    SUB,
    CGT,
    CLT,
    CEQ,
    CNEQ,
    JMP,
    JEQ { idx: usize },
    JNEQ { idx: usize },
}

impl Opcode {
    pub fn to_string(&self) -> String {
        let mut buf = String::new();

        match self {
            Opcode::PUSH => buf += "PUSH",
            Opcode::POP => buf += "POP",
            Opcode::CONST { idx } => buf += &format!("CONST\t\t{idx}"),
            Opcode::ADD => buf += "ADD",
            Opcode::SUB => buf += "SUB",
            Opcode::CGT => buf += "CGT",
            Opcode::CLT => buf += "CLT",
            Opcode::CEQ => buf += "CEQ",
            Opcode::CNEQ => buf += "CNEQ",
            Opcode::JMP => buf += "JMP",
            Opcode::JEQ { idx } => buf += &format!("JEQ\t\t{idx}"),
            Opcode::JNEQ { idx } => buf += &format!("JNEQ\t\t{idx}"),
        }

        buf
    }
}

pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Vec<Opcode>,
}

impl Bytecode {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),
            instructions: Vec::new(),
        }
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();

        for (idx, ins) in self.instructions.iter().enumerate() {
            buf += &format!("{}");
        }

        buf
    }
}
