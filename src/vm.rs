use crate::{
    bytecode::{instruction::Instruction, instructions::Instructions, Bytecode},
    object::{Int, Object, ObjectTrait, ObjectType},
};

pub struct VM {
    stack: Vec<Object>,
    constants: Vec<Object>,
    instructions: Instructions,

    ic: usize,
}

impl VM {
    pub fn new(code: Bytecode) -> Self {
        Self {
            stack: Vec::new(),
            constants: code.constants,
            instructions: code.instructions,
            ic: 0,
        }
    }

    pub fn run_single(&mut self) {
        let ins = self.instructions.read_instruction(self.ic);
        match &ins {
            Instruction::PUSH => todo!(),
            Instruction::POP => todo!(),
            Instruction::CONST { idx } => {
                // load constants into stack
                self.stack.push(self.constants[*idx].clone())
            }
            Instruction::ADD => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        let rst = Object::Int(Int {
                            value: left.value + right.value,
                        });
                        self.stack.push(rst);
                    } else if left.get_type() == ObjectType::String {
                        todo!()
                    }
                }
            }
            Instruction::SUB => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        let rst = Object::Int(Int {
                            value: left.value - right.value,
                        });
                        self.stack.push(rst);
                    }
                }
            }
            Instruction::PRODUCT => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        let rst = Object::Int(Int {
                            value: left.value * right.value,
                        });
                        self.stack.push(rst)
                    }
                }
            }
            Instruction::DIVIDE => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        let rst = Object::Int(Int {
                            value: left.value / right.value,
                        });
                        self.stack.push(rst)
                    }
                }
            }
            Instruction::MOD => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        let rst = Object::Int(Int {
                            value: left.value % right.value,
                        });
                        self.stack.push(rst)
                    } else if left.get_type() == ObjectType::String {
                        todo!()
                    }
                }
            }
            Instruction::BANG => {
                let right = self.stack.pop().unwrap();
                if right.get_type() == ObjectType::Bool {
                    let Object::Bool(mut right) = right else {unreachable!()};
                    right.value = !right.value;
                    self.stack.push(Object::Bool(right))
                } else {
                    // emit error
                }
            }
            Instruction::NEG => {
                let right = self.stack.pop().unwrap();
                if right.get_type() == ObjectType::Int {
                    let Object::Int(mut right) = right else {unreachable!()};
                    right.value = -right.value;
                    self.stack.push(Object::Int(right))
                } else {
                    // emit error
                }
            }
            Instruction::CGT => todo!(),
            Instruction::CLT => todo!(),
            Instruction::CEQ => todo!(),
            Instruction::CNEQ => todo!(),
            Instruction::JMP { idx } => todo!(),
            Instruction::JEQ { idx } => todo!(),
            Instruction::JNEQ { idx } => todo!(),
        }
        self.ic += ins.opcode().length();
    }

    pub fn is_runable(&self) -> bool {
        self.ic < self.instructions.length()
    }

    pub fn top(&self) -> Option<&Object> {
        self.stack.last()
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

        buf += "\nSTACK\n";
        for (idx, ins) in self.stack.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &ins.to_str();
            buf += "\n";
        }

        buf
    }
}
