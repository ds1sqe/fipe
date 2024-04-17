pub mod compiler;
pub mod op;

use crate::object::{Object, ObjectTrait};

use self::op::OP;

pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Vec<OP>,
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
        buf += "\nCONSTS\n";
        for (idx, cons) in self.constants.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &cons.to_str();
            buf += "\n";
        }

        buf += "\nINSTRUCTIONS\n";
        for (idx, ins) in self.instructions.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &ins.to_string();
            buf += "\n";
        }

        buf
    }
}
