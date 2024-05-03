use self::frame::Frame;

use super::bytecode::{instruction::Instruction, instructions::Instructions, Bytecode};
use crate::{
    object::{Array, Bool, CompiledFunction, Int, Object, ObjectTrait, ObjectType, StringObject},
    utils::add_pad,
};

mod frame;

const GLOBAL_SIZE: usize = 1 << 8;

const FRAME_SIZE: usize = 1 << 10;
const STACK_SIZE: usize = 1 << 11;

pub struct VM {
    /// stack[0] have reserved for inner representation of "NONE"
    stack: [Option<Object>; STACK_SIZE],
    /// stack pointer, postion of last written object.
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

        let mut frames = Vec::with_capacity(FRAME_SIZE);
        frames.push(main_frame);

        const STACK_INIT: Option<Object> = None;

        Self {
            stack: [STACK_INIT; STACK_SIZE],
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
            Instruction::POP => {
                self.pop_stack();
            }
            Instruction::CONST { idx } => {
                // load constants into stack
                self.push_stack(self.constants[*idx].clone());
            }
            Instruction::DEFGLB { idx } => {
                // define global variable
                if *idx < self.global.len() {
                    self.global[*idx] = self.pop_stack().unwrap();
                } else {
                    let obj = self.pop_stack().unwrap();
                    self.global.push(obj);
                }
            }
            Instruction::GETGLB { idx } => {
                // get global variable
                self.push_stack(self.global[*idx].clone());
            }
            Instruction::DEFLCL { idx } => {
                // define local variable
                let offset = self.current_frame().bp() + idx;
                self.stack[offset] = Some(self.pop_stack().unwrap());
            }
            Instruction::GETLCL { idx } => {
                // get local variable
                let offset = self.current_frame().bp() + idx;

                self.push_stack(self.stack[offset].clone().unwrap());
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
                let left = self.pop_stack().unwrap();
                let right = self.pop_stack().unwrap();

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
                                    Instruction::PRODUCT => left.value * right.value,
                                    Instruction::DIVIDE => left.value / right.value,
                                    Instruction::MOD => left.value % right.value,
                                    Instruction::BAND => left.value & right.value,
                                    Instruction::BOR => left.value | right.value,
                                    __ => {
                                        unreachable!()
                                    }
                                };
                                let rst = Object::Int(Int { value });
                                self.push_stack(rst);
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
                                self.push_stack(rst);
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
                        self.push_stack(rst);
                    } else if left.get_type() == ObjectType::String {
                        let Object::String(left) = left else {unreachable!()};
                        let Object::String(right) = right else {unreachable!()};

                        match &ins {
                            Instruction::ADD => {
                                let rst = Object::String(StringObject {
                                    value: left.value + &right.value,
                                });
                                self.push_stack(rst);
                            }
                            Instruction::CEQ => {
                                let rst = Object::Bool(Bool {
                                    value: left.value == right.value,
                                });
                                self.push_stack(rst);
                            }
                            Instruction::CNEQ => {
                                let rst = Object::Bool(Bool {
                                    value: left.value != right.value,
                                });
                                self.push_stack(rst);
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
                let right = self.pop_stack().unwrap();
                if right.get_type() == ObjectType::Bool {
                    let Object::Bool(mut right) = right else {unreachable!()};
                    right.value = !right.value;
                    self.push_stack(Object::Bool(right))
                } else {
                    // emit error
                }
            }
            Instruction::NEG => {
                let right = self.pop_stack().unwrap();
                if right.get_type() == ObjectType::Int {
                    let Object::Int(mut right) = right else {unreachable!()};
                    right.value = -right.value;
                    self.push_stack(Object::Int(right))
                } else {
                    // emit error
                }
            }

            Instruction::JMP { idx } => {
                self.current_frame_mut().set_ic(*idx);
                return;
            }
            Instruction::JIS { idx } => {
                let sign = self.pop_stack().unwrap();
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
                let sign = self.pop_stack().unwrap();
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

                for offset in (1..=*count).rev() {
                    elements.push(self.stack[self.sp + 1 - offset].clone().unwrap())
                }
                self.sp -= count;

                let array = Object::Array(Array { elements });

                self.push_stack(array);
            }
            Instruction::INDEX => {
                let idx = self.pop_stack().unwrap();

                if idx.get_type() == ObjectType::Int {
                    let Object::Int(int) = idx else {
                    unreachable!()
                    };

                    let tgt = self.pop_stack().unwrap();
                    if tgt.get_type() == ObjectType::Array {
                        let Object::Array(arr) = tgt else {
                    unreachable!()
                    };
                        self.push_stack(arr.elements[int.value as usize].clone());
                    } else {
                        // emit error
                    }
                } else {
                    // emit error
                }
            }
            Instruction::CALL { arg_len } => {
                self.current_frame_mut().add_ic(ins.opcode().length());
                self.call(*arg_len);
                return;
            }
            Instruction::RETN => {
                let popped_frame = self.pop_frame();
                self.sp = popped_frame.bp() - 1;
                return;
            }
            Instruction::RETV => {
                let value = self.pop_stack().unwrap();
                let popped_frame = self.pop_frame();
                self.sp = popped_frame.bp() - 1;
                self.push_stack(value);
                return;
            }
            not_implemented => {
                panic!("not implemented instruction {:?}", not_implemented);
            }
        }
        self.current_frame_mut().add_ic(ins.opcode().length());
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

        buf += &self.stack_to_string();

        buf
    }

    pub fn stack_to_string(&self) -> String {
        let mut buf = String::new();
        buf += "\nSTACK\n";
        for idx in 0..self.sp + 5 {
            let obj = &self.stack[idx];
            if idx < self.sp {
                buf += &format!("{:->6}\t\t", idx);
            } else if idx == self.sp {
                buf += &format!("{:0>6}\t\t", idx);
            } else {
                buf += &format!("{:+>6}\t\t", idx);
            }
            if obj.is_none() {
                buf += "NONE"
            } else {
                let obj = obj.as_ref().unwrap();
                if obj.get_type() == ObjectType::CompiledFunction {
                    buf += &add_pad(&obj.to_str(), "\t")
                } else {
                    buf += &obj.to_str();
                }
            }
            buf += "\n";
        }
        buf
    }

    pub fn is_runable(&self) -> bool {
        self.current_frame().is_runnable()
    }

    pub fn top(&self) -> &Option<Object> {
        &self.stack[self.sp]
    }
    pub fn last_pop(&self) -> &Option<Object> {
        &self.stack[self.sp + 1]
    }
    fn push_stack(&mut self, obj: Object) {
        if self.sp >= STACK_SIZE {
            panic!("STACK OVERFLOW: {}", self.stack_to_string())
        }
        self.stack[self.sp + 1] = Some(obj);
        self.sp += 1;
    }

    fn pop_stack(&mut self) -> Option<Object> {
        // TODO: Find better way to representation of None
        if self.sp > 0 {
            let result = self.stack[self.sp].clone();
            self.sp -= 1;
            return result;
        } else {
            self.stack[self.sp + 1] = None;
            return None;
        }
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
        let Object::CompiledFunction(fun) = self.stack[self.sp - arg_len].take().unwrap() else {
            unreachable!()
        };

        if arg_len != fun.arg_len {
            // emit error
        }
        let new_bp = self.sp - arg_len;
        let local_len = fun.local_len;

        let new_frame = Frame::new(fun, self.sp - arg_len);

        self.push_frame(new_frame);

        self.sp = new_bp + local_len
    }
}
