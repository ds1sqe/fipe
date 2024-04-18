pub mod compiler;
pub mod instruction;
pub mod instructions;
pub mod opcode;
mod symbol;

use crate::object::{Object, ObjectTrait};

use self::instructions::Instructions;

pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Instructions,
}

impl Bytecode {
    pub fn new() -> Self {
        Self {
            constants: Vec::new(),
            instructions: Instructions::new(),
        }
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        buf += "\nCONSTS\n";
        for (idx, cons) in self.constants.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &cons.to_str();
            buf += "\n";
        }

        buf += "\nINSTRUCTIONS\n";

        buf += &self.instructions.to_string();

        buf
    }
}
