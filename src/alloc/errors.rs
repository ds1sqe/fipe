use super::immix::memory::error::BlockError;

pub enum AllocError {
    /// Cannot allocate memory
    OutOfMemory,
    /// Size is too big to allocate
    SizeTooBig,
    /// Wrapper for block error
    InternalError(BlockError),
}
