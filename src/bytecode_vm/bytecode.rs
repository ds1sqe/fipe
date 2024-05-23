pub mod compiler;
pub mod errors;
pub mod instruction;
pub mod instructions;
pub mod opcode;
mod symbol;

use std::fmt::Display;

use crate::object::{Object, ObjectTrait};

use self::{errors::BytecodeError, instructions::Instructions};

#[derive(Debug)]
pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Instructions,
}

impl Bytecode {
    pub fn create() -> Result<Self, BytecodeError> {
        let new_instruction = Instructions::create()?;

        Ok(Self {
            constants: Vec::new(),
            instructions: new_instruction,
        })
    }
}

impl Display for Bytecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = String::new();
        buf += "\nCONSTS\n";
        for (idx, cons) in self.constants.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &cons.to_str();
            buf += "\n";
        }

        buf += "\nINSTRUCTIONS\n";

        buf += &self.instructions.to_string();

        f.write_str(&buf)
    }
}
