use super::size::SizeClassError;

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

#[derive(Debug)]
pub enum AllocError {
    /// Cannot allocate memory
    OutOfMemory,
    /// Size is too big to allocate
    SizeTooBig,
    /// Wrapper for block error
    InternalError(BlockError),
}

impl From<SizeClassError> for AllocError {
    fn from(value: SizeClassError) -> Self {
        match value {
            SizeClassError::TooBig => AllocError::SizeTooBig,
        }
    }
}

impl From<BlockError> for AllocError {
    fn from(value: BlockError) -> Self {
        AllocError::InternalError(value)
    }
}

#[derive(Debug)]
pub enum ImmixError {
    /// Key already exist
    DuplicatedKey,
    /// Wrapper for alloc error
    InternalError(AllocError),
}

impl From<AllocError> for ImmixError {
    fn from(value: AllocError) -> Self {
        ImmixError::InternalError(value)
    }
}
