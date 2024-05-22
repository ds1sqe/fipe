use std::alloc::Layout;

use crate::ast::Identifier;

#[derive(Debug)]
pub enum CompileError {
    /// compile had not ended in main scope
    NotFinishedInMain,

    /// cannot find identifier on symbol table
    IdentifierNotFound(Identifier),

    /// creation of [`super::compiler::Compiler`] have failed because of inner instruction failed
    CreationFailed(InstructionsError),

    /// failed to write (emit) instruction
    InstructionWriteError(InstructionsError),

    /// failed to create new scope bacause of
    /// [`InstructionsError`]
    ScopeCreateFaild(InstructionsError),
}

#[derive(Debug)]
pub enum InstructionsError {
    /// memory allocation have failed
    AllocationFailed(Layout),
    /// alloc size > [isize::MAX]
    TooLargeToAllocate,

    /// cannot read instruction at offset
    CannotRead { offset: usize },
}

#[derive(Debug)]
pub enum FrameError {
    /// wraper of InstructionsError
    InnerError(InstructionsError),
}

impl From<InstructionsError> for FrameError {
    fn from(err: InstructionsError) -> Self {
        FrameError::InnerError(err)
    }
}

#[derive(Debug)]
pub enum BytecodeError {
    /// wraper of InstructionsError
    InnerError(InstructionsError),
}

impl From<InstructionsError> for BytecodeError {
    fn from(err: InstructionsError) -> Self {
        Self::InnerError(err)
    }
}
