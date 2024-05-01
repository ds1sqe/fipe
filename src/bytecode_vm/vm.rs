use self::frame::Frame;

use super::bytecode::{
    instruction::Instruction, instructions::Instructions, Bytecode,
};
use crate::object::{
    Array, Bool, CompiledFunction, Int, Object, ObjectTrait, ObjectType,
    StringObject,
};

mod frame;

const GLOBAL_SIZE: usize = 1 << 8;

const FRAME_SIZE: usize = 1 << 10;

pub struct VM {
    stack: Vec<Object>,
    /// stack pointer
    sp: usize,

    constants: Vec<Object>,
    global: Vec<Object>,

    frames: Vec<Frame>,
    /// frame pointer
    fp: usize,
}

impl VM {
    pub fn new(code: Bytecode) -> Self {
        let main_func = CompiledFunction {
            instructions: code.instructions,
            local_len: 0,
            arg_len: 0,
        };
        let main_frame = Frame::new(main_func, 0);

        let frames = vec![main_frame];

        Self {
            stack: Vec::new(),
            sp: 0,

            constants: code.constants,
            global: Vec::with_capacity(GLOBAL_SIZE),

            frames,
            fp: 0,
        }
    }

    pub fn run_single(&mut self) {
        let ins = self.current_frame().rext_instruction();
        match &ins {
            Instruction::PUSH => todo!(),
            Instruction::POP => todo!(),
            Instruction::CONST { idx } => {
                // load constants into stack
                self.stack.push(self.constants[*idx].clone());
                self.sp += 1;
            }
            Instruction::DEFGLB { idx } => {
                // define global variable
                if *idx < self.global.len() {
                    self.global[*idx] = self.stack.pop().unwrap();
                } else {
                    self.global.push(self.stack.pop().unwrap());
                }
                self.sp -= 1;
            }
            Instruction::GETGLB { idx } => {
                // get global variable
                self.stack.push(self.global[*idx].clone());
                self.sp += 1;
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
                self.current_frame_mut().set_ic(*idx);
                return;
            }
            Instruction::JIS { idx } => {
                let sign = self.stack.pop().unwrap();
                if sign.get_type() == ObjectType::Bool {
                    let Object::Bool(sign) = sign else {unreachable!()};
                    if sign.value {
                        self.current_frame_mut().set_ic(*idx);
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
                        self.current_frame_mut().set_ic(*idx);
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
            Instruction::CALL { arg_len } => {
                self.current_frame_mut().add_ic(ins.opcode().length());
                //
                return;
            }
            not_implemented => {
                panic!("not implemented instruction {:?}", not_implemented);
            }
        }
        self.current_frame_mut().add_ic(ins.opcode().length());
    }

    pub fn is_runable(&self) -> bool {
        self.current_frame().is_runnable()
    }

    pub fn top(&self) -> Option<&Object> {
        self.stack.last()
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        buf += "CONSTS\n";
        for (idx, cons) in self.constants.iter().enumerate() {
            buf += &format!("{:0>6}\t\t", idx);
            buf += &cons.to_str();
            buf += "\n";
        }

        buf += "\nFrames\n";
        for (idx, frame) in self.frames.iter().enumerate() {
            if idx == self.fp {
                buf += "Current Frame:\n";
            }
            buf += &frame.to_string();
        }

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

    fn current_frame(&self) -> &Frame {
        &self.frames[self.fp]
    }
    fn current_frame_mut(&mut self) -> &mut Frame {
        &mut self.frames[self.fp]
    }

    fn push_frame(&mut self, frame: Frame) {
        self.fp += 1;
        self.frames.push(frame)
    }
    fn pop_frame(&mut self) -> Frame {
        self.fp -= 1;
        self.frames.pop().unwrap()
    }

    fn call(&mut self, arg_len: usize) {
        let Object::CompiledFunction(fun) = self.stack[self.sp - arg_len].to_owned() else {
            unreachable!()
        };

        if arg_len != fun.arg_len {
            // emit error
        }
        let new_bp = self.sp - arg_len;
        let local_len = fun.local_len;

        let new_frame = Frame::new(fun, self.sp - arg_len);

        self.sp = new_bp + local_len
    }
}
