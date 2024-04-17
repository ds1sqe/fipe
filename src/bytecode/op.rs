pub enum OP {
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

impl OP {
    pub fn to_string(&self) -> String {
        let mut buf = String::new();

        match self {
            OP::PUSH => buf += "PUSH",
            OP::POP => buf += "POP",
            OP::CONST { idx } => buf += &format!("CONST\t\t{idx}"),
            OP::ADD => buf += "ADD",
            OP::SUB => buf += "SUB",
            OP::CGT => buf += "CGT",
            OP::CLT => buf += "CLT",
            OP::CEQ => buf += "CEQ",
            OP::CNEQ => buf += "CNEQ",
            OP::JMP => buf += "JMP",
            OP::JEQ { idx } => buf += &format!("JEQ\t\t{idx}"),
            OP::JNEQ { idx } => buf += &format!("JNEQ\t\t{idx}"),
        }

        buf
    }
}
