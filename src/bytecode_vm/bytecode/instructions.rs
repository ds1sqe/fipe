use std::{alloc, alloc::Layout, ptr::NonNull};

use super::{instruction::Instruction, opcode::OpCode};

#[derive(Debug, Clone, PartialEq)]
pub struct Instructions {
    byte: NonNull<u8>,
    cursor: *mut u8,
    cap: usize,
    len: usize,
}

const SIZE: usize = 1 << 11;

impl Instructions {
    pub fn new() -> Self {
        let layout = Layout::array::<u8>(SIZE).unwrap();

        let new_ptr = unsafe { alloc::alloc(layout) };

        let ptr = match NonNull::new(new_ptr as *mut u8) {
            Some(p) => p,
            None => alloc::handle_alloc_error(layout),
        };

        Self {
            byte: ptr,
            cursor: ptr.as_ptr(),
            cap: SIZE,
            len: 0,
        }
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        let mut idx = 0;

        while idx < self.len {
            let ins = self.read_instruction(idx);
            buf += &format!("{:0>5}\t\t", idx);
            buf += &ins.to_string();
            buf += "\n";
            idx += ins.opcode().length();
        }

        buf
    }

    pub fn to_string_with_highlight(&self, hidx: usize) -> String {
        let mut buf = String::new();
        let mut idx = 0;

        while idx < self.len {
            let ins = self.read_instruction(idx);
            if idx == hidx {
                buf += &format!(">>{:0>5}\t\t", idx);
            } else {
                buf += &format!("{:0>5}\t\t", idx);
            }
            buf += &ins.to_string();
            buf += "\n";
            idx += ins.opcode().length();
        }

        buf
    }

    /// add new instruction at the end.
    /// return new Instruction's offset
    pub fn add_instruction(&mut self, ins: Instruction) -> usize {
        if self.len + ins.opcode().length() > self.cap {
            self.grow();
        }
        unsafe {
            std::ptr::copy(ins.as_byte().as_ptr(), self.cursor, ins.opcode().length());
            self.cursor = self.cursor.add(ins.opcode().length());
        }
        let offset = self.len;
        self.len += ins.opcode().length();
        offset
    }

    pub fn update_instruction(&mut self, ins: Instruction, offset: usize) {
        unsafe {
            std::ptr::copy(
                ins.as_byte().as_ptr(),
                self.byte.as_ptr().add(offset),
                ins.opcode().length(),
            );
        }
    }

    pub fn remove_instruction(&mut self, new_len: usize) {
        unsafe {
            self.cursor = self.byte.as_ptr().add(new_len);
        }
        self.len = new_len;
    }

    pub fn read_instruction(&self, offset: usize) -> Instruction {
        unsafe {
            let opcode = std::ptr::read(self.byte.as_ptr().add(offset) as *const OpCode);

            match opcode {
                OpCode::PUSH => Instruction::PUSH,
                OpCode::POP => Instruction::POP,
                OpCode::ADD => Instruction::ADD,
                OpCode::SUB => Instruction::SUB,
                OpCode::PRODUCT => Instruction::PRODUCT,
                OpCode::DIVIDE => Instruction::DIVIDE,
                OpCode::MOD => Instruction::MOD,
                OpCode::BANG => Instruction::BANG,
                OpCode::NEG => Instruction::NEG,
                OpCode::CGT => Instruction::CGT,
                OpCode::CGTE => Instruction::CGTE,
                OpCode::CLT => Instruction::CLT,
                OpCode::CLTE => Instruction::CLTE,
                OpCode::CEQ => Instruction::CEQ,
                OpCode::CNEQ => Instruction::CNEQ,
                OpCode::AND => Instruction::AND,
                OpCode::OR => Instruction::OR,
                OpCode::BAND => Instruction::BAND,
                OpCode::BOR => Instruction::BOR,
                OpCode::INDEX => Instruction::INDEX,
                OpCode::RETN => Instruction::RETN,
                OpCode::RETV => Instruction::RETV,

                has_argument => {
                    let arg_1 = std::ptr::read(self.byte.as_ptr().add(offset + 1) as *const usize);

                    match has_argument {
                        OpCode::CONST => Instruction::CONST { idx: arg_1 },
                        OpCode::DEFGLB => Instruction::DEFGLB { idx: arg_1 },
                        OpCode::GETGLB => Instruction::GETGLB { idx: arg_1 },
                        OpCode::DEFLCL => Instruction::DEFLCL { idx: arg_1 },
                        OpCode::GETLCL => Instruction::GETLCL { idx: arg_1 },
                        OpCode::GETFREE => Instruction::GETFREE { idx: arg_1 },
                        OpCode::JMP => Instruction::JMP { idx: arg_1 },
                        OpCode::JIS => Instruction::JIS { idx: arg_1 },
                        OpCode::JNS => Instruction::JNS { idx: arg_1 },
                        OpCode::JEQ => Instruction::JEQ { idx: arg_1 },
                        OpCode::JNEQ => Instruction::JNEQ { idx: arg_1 },
                        OpCode::ARRAY => Instruction::ARRAY { count: arg_1 },
                        OpCode::CALL => Instruction::CALL { arg_len: arg_1 },
                        OpCode::CLOSURE => {
                            let arg_2 = std::ptr::read(
                                self.byte.as_ptr().add(offset + 1 + 8) as *const usize
                            );

                            Instruction::CLOSURE {
                                idx: arg_1,
                                free: arg_2,
                            }
                        }
                        not_matched => {
                            panic!("Has to be unreachable {:?}", not_matched);
                        }
                    }
                }
            }
        }
    }
    pub fn length(&self) -> usize {
        self.len
    }

    fn grow(&mut self) {
        let new_cap = 2 * self.cap;
        let new_layout = Layout::array::<u8>(new_cap).unwrap();

        assert!(
            new_layout.size() <= isize::MAX as usize,
            "Too large to allocate"
        );

        let old_layout = Layout::array::<u8>(self.cap).unwrap();
        let old_ptr = self.byte.as_ptr() as *mut u8;
        let new_ptr = unsafe { alloc::realloc(old_ptr, old_layout, new_layout.size()) };

        self.byte = match NonNull::new(new_ptr as *mut u8) {
            Some(p) => p,
            None => alloc::handle_alloc_error(new_layout),
        };
        self.cap = new_cap
    }
}

// impl Drop for Instructions {
//     fn drop(&mut self) {
//         dbg!("drop...", &self);
//         unsafe {
//             alloc::dealloc(
//                 self.byte.as_ptr() as *mut u8,
//                 Layout::array::<u8>(self.cap).unwrap(),
//             );
//         }
//     }
// }
