use std::{alloc::Layout, fmt::Display};

use super::{
    errors::InstructionsError, instruction::Instruction, opcode::OpCode,
};

/// Owned encoded bytecode. Clones have independent storage.
#[derive(Debug, Clone, PartialEq)]
pub struct Instructions {
    byte: Vec<u8>,
}

const SIZE: usize = 1 << 11;

impl Instructions {
    /// Create an empty instruction buffer.
    pub fn create() -> Result<Self, InstructionsError> {
        let mut code = Self { byte: Vec::new() };
        code.reserve(SIZE)?;
        Ok(code)
    }

    fn reserve(&mut self, additional: usize) -> Result<(), InstructionsError> {
        let size = self
            .byte
            .len()
            .checked_add(additional)
            .ok_or(InstructionsError::TooLargeToAllocate)?;
        let layout = Layout::array::<u8>(size)
            .map_err(|_| InstructionsError::TooLargeToAllocate)?;
        self.byte
            .try_reserve(additional)
            .map_err(|_| InstructionsError::AllocationFailed(layout))
    }

    /// Append an instruction and return its byte offset.
    pub fn add_instruction(
        &mut self,
        ins: Instruction,
    ) -> Result<usize, InstructionsError> {
        let bytes = ins.as_byte();
        self.reserve(bytes.len())?;
        let offset = self.byte.len();
        self.byte.extend_from_slice(&bytes);
        Ok(offset)
    }

    /// Replace an instruction at its byte offset.
    ///
    /// # Safety
    /// The offset must name an instruction of the same encoded length.
    pub unsafe fn update_instruction(
        &mut self,
        ins: Instruction,
        offset: usize,
    ) {
        let bytes = ins.as_byte();
        let end = offset
            .checked_add(bytes.len())
            .expect("instruction offset overflow");
        self.byte[offset..end].copy_from_slice(&bytes);
    }

    /// Truncate the bytecode at an instruction boundary.
    pub fn remove_instruction(&mut self, new_len: usize) {
        assert!(new_len <= self.byte.len(), "cannot extend by truncating");
        self.byte.truncate(new_len);
    }

    fn read_operand(&self, offset: usize) -> Option<usize> {
        let end = offset.checked_add(std::mem::size_of::<usize>())?;
        let bytes = self.byte.get(offset..end)?.try_into().ok()?;
        Some(usize::from_ne_bytes(bytes))
    }

    /// Decode an instruction without assuming alignment of its operands.
    pub fn read_instruction(
        &self,
        offset: usize,
    ) -> Result<Instruction, InstructionsError> {
        let decode = || -> Option<Instruction> {
            let opcode = OpCode::try_from(*self.byte.get(offset)?).ok()?;
            let argument = || self.read_operand(offset.checked_add(1)?);
            Some(match opcode {
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
                OpCode::CONST => Instruction::CONST { idx: argument()? },
                OpCode::DEFGLB => Instruction::DEFGLB { idx: argument()? },
                OpCode::GETGLB => Instruction::GETGLB { idx: argument()? },
                OpCode::DEFLCL => Instruction::DEFLCL { idx: argument()? },
                OpCode::GETLCL => Instruction::GETLCL { idx: argument()? },
                OpCode::GETFREE => Instruction::GETFREE { idx: argument()? },
                OpCode::JMP => Instruction::JMP { idx: argument()? },
                OpCode::JIS => Instruction::JIS { idx: argument()? },
                OpCode::JNS => Instruction::JNS { idx: argument()? },
                OpCode::JEQ => Instruction::JEQ { idx: argument()? },
                OpCode::JNEQ => Instruction::JNEQ { idx: argument()? },
                OpCode::ARRAY => Instruction::ARRAY { count: argument()? },
                OpCode::CALL => Instruction::CALL {
                    arg_len: argument()?,
                },
                OpCode::CLOSURE => Instruction::CLOSURE {
                    idx: argument()?,
                    free: self.read_operand(
                        offset.checked_add(1 + std::mem::size_of::<usize>())?,
                    )?,
                },
            })
        };
        decode().ok_or(InstructionsError::CannotRead { offset })
    }

    /// Return the encoded length in bytes.
    pub fn length(&self) -> usize {
        self.byte.len()
    }

    pub fn to_string_with_highlight(&self, hidx: usize) -> String {
        self.format(Some(hidx))
    }

    fn format(&self, highlight: Option<usize>) -> String {
        let mut buf = String::new();
        let mut offset = 0;
        while offset < self.byte.len() {
            let instruction = match self.read_instruction(offset) {
                Ok(instruction) => instruction,
                Err(error) => {
                    return format!("Error reading instruction: {error:?}")
                }
            };
            if highlight == Some(offset) {
                buf += ">>";
            }
            buf += &format!("{offset:0>5}\t\t{instruction}\n");
            offset += instruction.opcode().length();
        }
        buf
    }
}

impl Display for Instructions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.format(None))
    }
}
