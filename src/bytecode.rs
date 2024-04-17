pub mod compiler;
pub mod op;

use crate::object::Object;

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

        for (idx, ins) in self.instructions.iter().enumerate() {
            buf += &format!("{:0<6}", idx);
            buf += &ins.to_string();
            buf += "\n";
        }

        buf
    }
}
