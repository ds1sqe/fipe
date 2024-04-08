#[derive(Debug)]
pub enum BlockError {
    /// block size is not a power of two
    BadSize(usize),
    /// Cannot allocate memory
    OutOfMemory,
    /// Cannot allocate in block
    NoSpaceForAllocation,
    /// Address have overflow
    AddressOverflow,
}

pub enum AllocError {
    /// Cannot allocate memory
    OutOfMemory,
    /// Size is too big to allocate
    SizeTooBig,
    /// Wrapper for block error
    InternalError(BlockError),
}
