use std::fmt::Display;

use self::{errors::VmError, frame::Frame};

use super::bytecode::{instruction::Instruction, Bytecode};
use crate::{
    object::{
        Array, Bool, ClosureFunction, CompiledFunction, Int, Object,
        ObjectTrait, ObjectType, StringObject,
    },
    utils::add_pad,
};

mod frame;

pub mod errors;

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
        let main_closure = ClosureFunction {
            fun: main_func,
            free: Vec::new(),
        };
        let main_frame = Frame::new(main_closure, 0);

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

    pub fn run_single(&mut self) -> Result<(), VmError> {
        let ins_rst = self.current_frame().rext_instruction();
        match ins_rst {
            Err(err) => Err(VmError::InstructionReadFailure(err)),
            Ok(instruction) => {
                match instruction {
                    Instruction::PUSH => todo!(),
                    Instruction::POP => {
                        self.pop_stack();
                    }
                    Instruction::CONST { idx } => {
                        // load constants into stack
                        self.push_stack(self.constants[idx].clone());
                    }
                    Instruction::DEFGLB { idx } => {
                        // define global variable
                        if idx < self.global.len() {
                            self.global[idx] = self.pop_stack().unwrap();
                        } else {
                            let obj = self.pop_stack().unwrap();
                            self.global.push(obj);
                        }
                    }
                    Instruction::GETGLB { idx } => {
                        // get global variable
                        self.push_stack(self.global[idx].clone());
                    }
                    Instruction::DEFLCL { idx } => {
                        // define local variable
                        let offset = &self.current_frame().bp() + 1 + idx;
                        self.stack[offset] = Some(self.pop_stack().unwrap());
                    }
                    Instruction::GETLCL { idx } => {
                        // get local variable
                        let offset = &self.current_frame().bp() + 1 + idx;

                        self.push_stack(self.stack[offset].clone().unwrap());
                    }
                    Instruction::GETFREE { idx } => {
                        // get local variable
                        self.push_stack(
                            self.current_frame().get_closure_ref().free[idx]
                                .clone(),
                        );
                    }
                    Instruction::GETCUR => {
                        // push current function to stack
                        self.push_stack(Object::Closure(
                            self.current_frame().get_closure_ref().clone(),
                        ));
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
                                let Object::Int(left) = left else {
                                    unreachable!()
                                };
                                let Object::Int(right) = right else {
                                    unreachable!()
                                };

                                match instruction {
                                    Instruction::ADD
                                    | Instruction::SUB
                                    | Instruction::PRODUCT
                                    | Instruction::DIVIDE
                                    | Instruction::MOD
                                    | Instruction::BAND
                                    | Instruction::BOR => {
                                        let value = match &instruction {
                                            Instruction::ADD => {
                                                left.value + right.value
                                            }
                                            Instruction::SUB => {
                                                left.value - right.value
                                            }
                                            Instruction::PRODUCT => {
                                                left.value * right.value
                                            }
                                            Instruction::DIVIDE => {
                                                left.value / right.value
                                            }
                                            Instruction::MOD => {
                                                left.value % right.value
                                            }
                                            Instruction::BAND => {
                                                left.value & right.value
                                            }
                                            Instruction::BOR => {
                                                left.value | right.value
                                            }
                                            _unreachable => {
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
                                        let value = match &instruction {
                                            Instruction::CGT => {
                                                left.value > right.value
                                            }
                                            Instruction::CGTE => {
                                                left.value >= right.value
                                            }
                                            Instruction::CLT => {
                                                left.value < right.value
                                            }
                                            Instruction::CLTE => {
                                                left.value <= right.value
                                            }
                                            Instruction::CEQ => {
                                                left.value == right.value
                                            }
                                            Instruction::CNEQ => {
                                                left.value != right.value
                                            }
                                            _unreachable => {
                                                unreachable!()
                                            }
                                        };
                                        let rst = Object::Bool(Bool { value });
                                        self.push_stack(rst);
                                    }
                                    invalid => {
                                        return Err(
                                            VmError::InvalidIntegerInstruction(
                                                invalid,
                                            ),
                                        );
                                    }
                                }
                            } else if left.get_type() == ObjectType::Bool {
                                let Object::Bool(left) = left else {
                                    unreachable!()
                                };
                                let Object::Bool(right) = right else {
                                    unreachable!()
                                };
                                let value = match instruction {
                                    Instruction::AND => {
                                        left.value && right.value
                                    }
                                    Instruction::OR => {
                                        left.value || right.value
                                    }
                                    Instruction::CEQ => {
                                        left.value == right.value
                                    }
                                    Instruction::CNEQ => {
                                        left.value != right.value
                                    }
                                    invaild_instruction => {
                                        return Err(
                                            VmError::InvalidBoolInstruction(
                                                invaild_instruction,
                                            ),
                                        );
                                    }
                                };
                                let rst = Object::Bool(Bool { value });
                                self.push_stack(rst);
                            } else if left.get_type() == ObjectType::String {
                                let Object::String(left) = left else {
                                    unreachable!()
                                };
                                let Object::String(right) = right else {
                                    unreachable!()
                                };

                                match instruction {
                                    Instruction::ADD => {
                                        let rst =
                                            Object::String(StringObject {
                                                value: left.value
                                                    + &right.value,
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
                                    invalid => {
                                        return Err(
                                            VmError::InvalidStringInstruction(
                                                invalid,
                                            ),
                                        );
                                    }
                                }
                            }
                        } else {
                            return Err(VmError::TypeNotSame { left, right });
                        }
                    }

                    Instruction::BANG => {
                        let right = self.pop_stack().unwrap();
                        if right.get_type() == ObjectType::Bool {
                            let Object::Bool(mut right) = right else {
                                unreachable!()
                            };
                            right.value = !right.value;
                            self.push_stack(Object::Bool(right))
                        } else {
                            return Err(VmError::NotABolean { obj: right });
                        }
                    }
                    Instruction::NEG => {
                        let right = self.pop_stack().unwrap();
                        if right.get_type() == ObjectType::Int {
                            let Object::Int(mut right) = right else {
                                unreachable!()
                            };
                            right.value = -right.value;
                            self.push_stack(Object::Int(right))
                        } else {
                            return Err(VmError::NotAInt { obj: right });
                        }
                    }

                    Instruction::JMP { idx } => {
                        self.current_frame_mut().set_ic(idx);
                        return Ok(());
                    }
                    Instruction::JIS { idx } => {
                        let sign = self.pop_stack().unwrap();
                        if sign.get_type() == ObjectType::Bool {
                            let Object::Bool(sign) = sign else {
                                unreachable!()
                            };
                            if sign.value {
                                self.current_frame_mut().set_ic(idx);
                                return Ok(());
                            }
                        } else {
                            return Err(VmError::JumpConditionNotABolean {
                                obj: sign,
                            });
                        }
                    }
                    Instruction::JNS { idx } => {
                        let sign = self.pop_stack().unwrap();
                        if sign.get_type() == ObjectType::Bool {
                            let Object::Bool(sign) = sign else {
                                unreachable!()
                            };
                            if !sign.value {
                                self.current_frame_mut().set_ic(idx);
                                return Ok(());
                            }
                        } else {
                            return Err(VmError::JumpConditionNotABolean {
                                obj: sign,
                            });
                        }
                    }

                    Instruction::JEQ { idx: _ } => {
                        todo!()
                    }
                    Instruction::JNEQ { idx: _ } => {
                        todo!()
                    }

                    Instruction::ARRAY { count } => {
                        let mut elements = Vec::with_capacity(count);

                        for offset in (1..=count).rev() {
                            elements.push(
                                self.stack[self.sp + 1 - offset]
                                    .clone()
                                    .unwrap(),
                            )
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
                                self.push_stack(
                                    arr.elements[int.value as usize].clone(),
                                );
                            } else {
                                return Err(VmError::IndexTargetNotAArray {
                                    obj: tgt,
                                });
                            }
                        } else {
                            return Err(VmError::IndexNotAInt { obj: idx });
                        }
                    }
                    Instruction::CALL { arg_len } => {
                        self.current_frame_mut()
                            .add_ic(instruction.opcode().length());
                        self.call(arg_len);
                    }
                    Instruction::RETN => {
                        let popped_frame = self.pop_frame();
                        self.sp = &popped_frame.bp() - 1;
                    }
                    Instruction::RETV => {
                        let value = self.pop_stack().unwrap();
                        let popped_frame = self.pop_frame();
                        self.sp = &popped_frame.bp() - 1;
                        self.push_stack(value);
                    }
                    Instruction::CLOSURE { idx, free } => {
                        self.make_closure(idx, free);
                    }
                }

                self.current_frame_mut()
                    .add_ic(instruction.opcode().length());

                Ok(())
            }
        }
    }

    pub fn stack_to_string(&self) -> String {
        let mut buf = String::new();
        buf += "\nSTACK\n";
        for idx in 0..self.sp + 5 {
            let obj = &self.stack[idx];

            match idx.cmp(&self.sp) {
                std::cmp::Ordering::Less => {
                    buf += &format!("{:->6}\t\t", idx);
                }
                std::cmp::Ordering::Equal => {
                    buf += &format!("{:0>6}\t\t", idx);
                }
                std::cmp::Ordering::Greater => {
                    buf += &format!("{:+>6}\t\t", idx);
                }
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
        if self.sp > 0 {
            let result = self.stack[self.sp].clone();
            self.sp -= 1;
            result
        } else {
            // TODO: Find better way to representation of None
            self.stack[self.sp + 1] = None;
            None
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

    fn make_closure(&mut self, fn_idx: usize, free_len: usize) {
        let Object::CompiledFunction(compiled_func) =
            self.constants[fn_idx].clone()
        else {
            unreachable!()
        };

        let mut closure = ClosureFunction {
            fun: compiled_func,
            free: Vec::with_capacity(free_len),
        };

        for idx in 0..free_len {
            closure
                .free
                .push(self.stack[self.sp + 1 - free_len + idx].clone().unwrap())
        }

        self.push_stack(Object::Closure(closure))
    }

    fn call(&mut self, arg_len: usize) {
        let Object::Closure(cl) =
            self.stack[self.sp - arg_len].clone().unwrap()
        else {
            unreachable!()
        };

        if arg_len != cl.fun.arg_len {
            // emit error
        }
        // postion of function
        let new_bp = self.sp - arg_len;
        let local_len = cl.fun.local_len;

        let new_frame = Frame::new(cl, new_bp);

        self.push_frame(new_frame);

        self.sp = new_bp + local_len;
    }
}

impl Display for VM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

        f.write_str(buf.as_str())
    }
}
