use std::{
    alloc::{self, Layout},
    fmt::Display,
    ptr::NonNull,
};

use super::{
    errors::InstructionsError, instruction::Instruction, opcode::OpCode,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Instructions {
    /// actual bytes are stored in here
    byte: NonNull<u8>,
    /// position of next instruction will be written
    cursor: *mut u8,
    /// capacity of [`Self::byte`] ( bytes )
    cap: usize,
    /// actual length of whole instructions ( bytes )
    len: usize,
}

/// default size of instructions, 2048 (0x800) bytes
const SIZE: usize = 1 << 11;

const SUCCESS: Result<(), InstructionsError> = Ok(());

impl Instructions {
    /// create [`Instructions`]
    ///
    /// # Panics
    ///
    /// Panics if OOM (out of memory) and panic_flag is set to true
    ///
    /// # Errors [`InstructionsError::AllocationFailed`]
    ///
    /// This function will return an error if panic_flag is set to false
    /// and OOM occured
    pub fn create() -> Result<Self, InstructionsError> {
        let layout = Layout::array::<u8>(SIZE).unwrap();

        let new_ptr = unsafe { alloc::alloc(layout) };

        let ptr = match NonNull::new(new_ptr) {
            Some(p) => p,
            None => {
                // HACK: change this to configuration
                let panic_flag = false;

                if panic_flag {
                    // May panic here if OOM (Out of memory)
                    // depending on env_config and if panic_flag is true,
                    // global configuration, it may either panic (resulting in unwinding or aborting as per
                    // configuration for all panics), or abort the process (with no unwinding).
                    alloc::handle_alloc_error(layout);
                }
                return Err(InstructionsError::AllocationFailed(layout));
            }
        };

        Ok(Self {
            byte: ptr,
            cursor: ptr.as_ptr(),
            cap: SIZE,
            len: 0,
        })
    }
    /// Stringify this [`Instructions`]
    ///
    /// * hidx - index of highlight target
    pub fn to_string_with_highlight(&self, hidx: usize) -> String {
        let mut buf = String::new();
        let mut idx = 0;

        while idx < self.len {
            let res = self.read_instruction(idx);

            if res.is_err() {
                return format!(
                    "Error have occured on reading instruction. Error: {:?}",
                    res.unwrap_err()
                );
            }

            let ins = res.unwrap();

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

    /// Add new instruction at the end.
    /// return new Instruction's offset
    ///
    /// # Errors [`InstructionsError`]
    ///
    /// This function will return an error if this [`Instructions::grow()`] have failed.
    pub fn add_instruction(
        &mut self,
        ins: Instruction,
    ) -> Result<usize, InstructionsError> {
        if self.len + ins.opcode().length() > self.cap {
            self.grow()?;
        }
        unsafe {
            std::ptr::copy(
                ins.as_byte().as_ptr(),
                self.cursor,
                ins.opcode().length(),
            );
            self.cursor = self.cursor.add(ins.opcode().length());
        }
        let offset = self.len;
        self.len += ins.opcode().length();

        Ok(offset)
    }

    /// Update instruction at `offset` with given `ins`
    ///
    /// # Safety
    /// if lengths are different between `ins` and instruction at `offset`,
    /// There will be Corruption of this [`Instructions`]
    pub unsafe fn update_instruction(
        &mut self,
        ins: Instruction,
        offset: usize,
    ) {
        unsafe {
            std::ptr::copy(
                ins.as_byte().as_ptr(),
                self.byte.as_ptr().add(offset),
                ins.opcode().length(),
            );
        }
    }

    /// Remove instructions by set [`Self::len`] with `new_len`
    ///
    /// This will move [`Self::cursor`] with new position of next instruction's one
    pub fn remove_instruction(&mut self, new_len: usize) {
        unsafe {
            self.cursor = self.byte.as_ptr().add(new_len);
        }
        self.len = new_len;
    }

    /// Read Instruction at given `offset`
    ///
    /// # Errors [`InstructionsError::CannotRead`]
    ///
    /// This function will return an error if failed to read opcode
    /// from [`Self::byte`] at `offset` to convert it as [`Instruction`]
    pub fn read_instruction(
        &self,
        offset: usize,
    ) -> Result<Instruction, InstructionsError> {
        unsafe {
            let opcode =
                std::ptr::read(self.byte.as_ptr().add(offset) as *const OpCode);

            let instruction = match opcode {
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
                OpCode::GETCUR => Instruction::GETCUR,

                has_argument => {
                    let arg_1 = std::ptr::read(
                        self.byte.as_ptr().add(offset + 1) as *const usize,
                    );

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
                                self.byte.as_ptr().add(offset + 1 + 8)
                                    as *const usize,
                            );

                            Instruction::CLOSURE {
                                idx: arg_1,
                                free: arg_2,
                            }
                        }
                        _not_matched => {
                            return Err(InstructionsError::CannotRead {
                                offset,
                            });
                        }
                    }
                }
            };

            Ok(instruction)
        }
    }

    /// Returns the length of this [`Instructions`].
    pub fn length(&self) -> usize {
        self.len
    }

    /// grow size of [`Self::byte`] by twice
    ///
    /// # Panics
    ///
    /// Panics if OOM (out of memory) and panic_flag is set to true
    ///
    /// # Errors [`InstructionsError::AllocationFailed`]
    ///
    /// This function will return an error if panic_flag is set to false
    /// and OOM occured
    fn grow(&mut self) -> Result<(), InstructionsError> {
        let new_cap = 2 * self.cap;
        let new_layout = Layout::array::<u8>(new_cap).unwrap();

        if new_layout.size() <= isize::MAX as usize {
            return Err(InstructionsError::TooLargeToAllocate);
        }

        let old_layout = Layout::array::<u8>(self.cap).unwrap();
        let old_ptr = self.byte.as_ptr();
        let new_ptr =
            unsafe { alloc::realloc(old_ptr, old_layout, new_layout.size()) };

        self.byte = match NonNull::new(new_ptr) {
            Some(p) => p,
            None => {
                // HACK: change this to configuration
                let panic_flag = false;

                if panic_flag {
                    // May panic here if OOM (Out of memory)
                    // depending on env_config and if panic_flag is true,
                    // global configuration, it may either panic (resulting in unwinding or aborting as per
                    // configuration for all panics), or abort the process (with no unwinding).
                    alloc::handle_alloc_error(new_layout);
                }
                return Err(InstructionsError::AllocationFailed(new_layout));
            }
        };
        self.cap = new_cap;

        SUCCESS
    }

    /// Returns the manual drop of this [`Instructions`].
    ///
    /// # Safety
    /// this is unsafe because of possibility of multiple owner of this
    #[allow(dead_code)]
    unsafe fn manual_drop(&mut self) {
        dbg!("drop...", &self);
        unsafe {
            alloc::dealloc(
                self.byte.as_ptr(),
                Layout::array::<u8>(self.cap).unwrap(),
            );
        }
    }
}

impl Display for Instructions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = String::new();
        let mut idx = 0;

        while idx < self.len {
            let res = self.read_instruction(idx);

            if res.is_err() {
                f.write_fmt(format_args!(
                    "Error have occured on reading instruction. Error: {:?}",
                    res.unwrap_err()
                ))?;
                return Ok(());
            }

            let ins = res.unwrap();
            buf += &format!("{:0>5}\t\t", idx);
            buf += &ins.to_string();
            buf += "\n";
            idx += ins.opcode().length();
        }

        f.write_str(buf.as_str())
    }
}
