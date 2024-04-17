use crate::{
    bytecode::{op::OP, Bytecode},
    object::{Int, Object, ObjectTrait, ObjectType},
};

pub struct VM {
    stack: Vec<Object>,
    constants: Vec<Object>,
    instructions: Vec<OP>,

    pc: usize,
}

impl VM {
    pub fn new(code: Bytecode) -> Self {
        Self {
            stack: Vec::new(),
            constants: code.constants,
            instructions: code.instructions,
            pc: 0,
        }
    }

    pub fn run_single(&mut self) {
        match self.instructions[self.pc] {
            OP::PUSH => todo!(),
            OP::POP => todo!(),
            OP::CONST { idx } => {
                // load constants into stack
                self.stack.push(self.constants[idx].clone())
            }
            OP::ADD => {
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
            OP::SUB => {
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
            OP::PRODUCT => {
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
            OP::DIVIDE => {
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
            OP::MOD => {
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
            OP::BANG => {
                let right = self.stack.pop().unwrap();
                if right.get_type() == ObjectType::Bool {
                    let Object::Bool(mut right) = right else {unreachable!()};
                    right.value = !right.value;
                    self.stack.push(Object::Bool(right))
                } else {
                    // emit error
                }
            }
            OP::NEG => {
                let right = self.stack.pop().unwrap();
                if right.get_type() == ObjectType::Int {
                    let Object::Int(mut right) = right else {unreachable!()};
                    right.value = -right.value;
                    self.stack.push(Object::Int(right))
                } else {
                    // emit error
                }
            }
            OP::CGT => todo!(),
            OP::CLT => todo!(),
            OP::CEQ => todo!(),
            OP::CNEQ => todo!(),
            OP::JMP => todo!(),
            OP::JEQ { idx } => todo!(),
            OP::JNEQ { idx } => todo!(),
        }
        self.pc += 1;
    }

    pub fn is_runable(&self) -> bool {
        self.pc < self.instructions.len()
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
        for (idx, ins) in self.instructions.iter().enumerate() {
            if idx == self.pc {
                buf += &format!("{:*>6}\t\t", idx);
            } else {
                buf += &format!("{:0>6}\t\t", idx);
            }
            buf += &ins.to_string();
            buf += "\n";
        }

        buf += "\nSTACK\n";
        for (idx, ins) in self.stack.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &ins.to_str();
            buf += "\n";
        }

        buf
    }
}
