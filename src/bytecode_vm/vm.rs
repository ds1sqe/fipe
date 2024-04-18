use super::bytecode::{
    instruction::Instruction, instructions::Instructions, Bytecode,
};
use crate::object::{
    Array, Bool, Int, Object, ObjectTrait, ObjectType, StringObject,
};

const GLOBAL_SIZE: usize = 1 << 8;

pub struct VM {
    stack: Vec<Object>,
    constants: Vec<Object>,
    global: Vec<Object>,
    instructions: Instructions,

    ic: usize,
}

impl VM {
    pub fn new(code: Bytecode) -> Self {
        Self {
            stack: Vec::new(),
            constants: code.constants,
            instructions: code.instructions,
            global: Vec::with_capacity(GLOBAL_SIZE),
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
            Instruction::DEFGLB { idx } => {
                // define global variable
                if *idx < self.global.len() {
                    self.global[*idx] = self.stack.pop().unwrap();
                } else {
                    self.global.push(self.stack.pop().unwrap());
                }
            }
            Instruction::GETGLB { idx } => {
                // get global variable
                self.stack.push(self.global[*idx].clone())
            }

            Instruction::ADD
            | Instruction::SUB
            | Instruction::PRODUCT
            | Instruction::DIVIDE
            | Instruction::MOD
            | Instruction::CGT
            | Instruction::CGTE
            | Instruction::CLT
            | Instruction::CLTE
            | Instruction::CEQ
            | Instruction::CNEQ
            | Instruction::AND
            | Instruction::OR
            | Instruction::BAND
            | Instruction::BOR => {
                let left = self.stack.pop().unwrap();
                let right = self.stack.pop().unwrap();

                if left.get_type() == right.get_type() {
                    if left.get_type() == ObjectType::Int {
                        let Object::Int(left) = left else {unreachable!()};
                        let Object::Int(right) = right else {unreachable!()};

                        match &ins {
                            Instruction::ADD
                            | Instruction::SUB
                            | Instruction::PRODUCT
                            | Instruction::DIVIDE
                            | Instruction::MOD
                            | Instruction::BAND
                            | Instruction::BOR => {
                                let value = match &ins {
                                    Instruction::ADD => left.value + right.value,
                                    Instruction::SUB => left.value - right.value,
                                    Instruction::PRODUCT => {
                                        left.value * right.value
                                    }
                                    Instruction::DIVIDE => left.value / right.value,
                                    Instruction::MOD => left.value % right.value,
                                    Instruction::BAND => left.value & right.value,
                                    Instruction::BOR => left.value | right.value,
                                    __ => {
                                        unreachable!()
                                    }
                                };
                                let rst = Object::Int(Int { value });
                                self.stack.push(rst);
                            }
                            Instruction::CGT
                            | Instruction::CGTE
                            | Instruction::CLT
                            | Instruction::CLTE
                            | Instruction::CEQ
                            | Instruction::CNEQ => {
                                let value = match &ins {
                                    Instruction::CGT => left.value > right.value,
                                    Instruction::CGTE => left.value >= right.value,
                                    Instruction::CLT => left.value < right.value,
                                    Instruction::CLTE => left.value <= right.value,
                                    Instruction::CEQ => left.value == right.value,
                                    Instruction::CNEQ => left.value != right.value,
                                    __ => {
                                        unreachable!()
                                    }
                                };
                                let rst = Object::Bool(Bool { value });
                                self.stack.push(rst);
                            }
                            __ => {
                                unreachable!()
                            }
                        }
                    } else if left.get_type() == ObjectType::Bool {
                        let Object::Bool(left) = left else {unreachable!()};
                        let Object::Bool(right) = right else {unreachable!()};
                        let value = match &ins {
                            Instruction::AND => left.value && right.value,
                            Instruction::OR => left.value || right.value,
                            Instruction::CEQ => left.value == right.value,
                            Instruction::CNEQ => left.value != right.value,
                            __ => {
                                unreachable!()
                            }
                        };
                        let rst = Object::Bool(Bool { value });
                        self.stack.push(rst);
                    } else if left.get_type() == ObjectType::String {
                        let Object::String(left) = left else {unreachable!()};
                        let Object::String(right) = right else {unreachable!()};

                        match &ins {
                            Instruction::ADD => {
                                let rst = Object::String(StringObject {
                                    value: left.value + &right.value,
                                });
                                self.stack.push(rst);
                            }
                            Instruction::CEQ => {
                                let rst = Object::Bool(Bool {
                                    value: left.value == right.value,
                                });
                                self.stack.push(rst);
                            }
                            Instruction::CNEQ => {
                                let rst = Object::Bool(Bool {
                                    value: left.value != right.value,
                                });
                                self.stack.push(rst);
                            }
                            __ => {
                                unreachable!()
                            }
                        }
                    }
                } else {
                    // emit error
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

            Instruction::JMP { idx } => {
                self.ic = *idx;
                return;
            }
            Instruction::JIS { idx } => {
                let sign = self.stack.pop().unwrap();
                if sign.get_type() == ObjectType::Bool {
                    let Object::Bool(sign) = sign else {unreachable!()};
                    if sign.value {
                        self.ic = *idx;
                        return;
                    }
                } else {
                    // emit error
                }
            }
            Instruction::JNS { idx } => {
                let sign = self.stack.pop().unwrap();
                if sign.get_type() == ObjectType::Bool {
                    let Object::Bool(sign) = sign else {unreachable!()};
                    if !sign.value {
                        self.ic = *idx;
                        return;
                    }
                } else {
                    // emit error
                }
            }
            Instruction::JEQ { idx } => todo!(),
            Instruction::JNEQ { idx } => todo!(),
            Instruction::ARRAY { count } => {
                let mut elements = Vec::with_capacity(*count);
                let stack_len = self.stack.len();

                for offset in (1..=*count).rev() {
                    elements.push(self.stack[stack_len - offset].clone())
                }
                self.stack.truncate(stack_len - count);

                let array = Object::Array(Array { elements });

                self.stack.push(array);
            }
            Instruction::INDEX => {
                let idx = self.stack.pop().unwrap();

                if idx.get_type() == ObjectType::Int {
                    let Object::Int(int) = idx else {
                    unreachable!()
                    };

                    let tgt = self.stack.pop().unwrap();
                    if tgt.get_type() == ObjectType::Array {
                        let Object::Array(arr) = tgt else {
                    unreachable!()
                    };
                        self.stack.push(arr.elements[int.value as usize].clone());
                    } else {
                        // emit error
                    }
                } else {
                    // emit error
                }
            }
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
        buf += &format!("IC : {}\n", self.ic);
        buf += "CONSTS\n";
        for (idx, cons) in self.constants.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &cons.to_str();
            buf += "\n";
        }

        buf += "\nINSTRUCTIONS\n";
        buf += &self.instructions.to_string_with_highlight(self.ic);

        buf += "\nSTACK\n";
        for (idx, ins) in self.stack.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &ins.to_str();
            buf += "\n";
        }

        buf
    }

    pub fn stack_to_string(&self) -> String {
        let mut buf = String::new();
        buf += "\nSTACK\n";
        for (idx, ins) in self.stack.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &ins.to_str();
            buf += "\n";
        }

        buf
    }
}
