pub const BLOCK_SIZE_BITS: usize = 15;
// 32k   0x8000
pub const BLOCK_SIZE: usize = 1 << BLOCK_SIZE_BITS;

pub const LINE_SIZE_BITS: usize = 7;
// 128   0x80
pub const LINE_SIZE: usize = 1 << LINE_SIZE_BITS;

// 256   0x100
pub const LINE_COUNT: usize = BLOCK_SIZE / LINE_SIZE;

// 32512 0x7F00
pub const BLOCK_CAPACITY: usize = BLOCK_SIZE - LINE_COUNT;
pub const LINE_MARK_START: usize = BLOCK_CAPACITY;

pub const ALIGN_WORD: usize = 16;
pub const ALIGN_MASK: usize = !(ALIGN_WORD - 1);
