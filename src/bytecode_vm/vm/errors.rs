use crate::{bytecode_vm::bytecode::errors::FrameError, object::Object};

use super::super::bytecode::instruction::Instruction;

#[derive(Debug)]
pub enum VmError {
    InstructionReadFailure(FrameError),
    InvalidIntegerInstruction(Instruction),
    InvalidBoolInstruction(Instruction),
    InvalidStringInstruction(Instruction),
    TypeNotSame { left: Object, right: Object },
    NotABolean { obj: Object },
    NotAInt { obj: Object },

    JumpConditionNotABolean { obj: Object },
    IndexTargetNotAArray { obj: Object },
    IndexNotAInt { obj: Object },
    IndexOutOfBounds { index: i64, length: usize },

    NotImplentedInstruction { ins: Instruction },
}
