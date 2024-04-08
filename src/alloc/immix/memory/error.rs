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
