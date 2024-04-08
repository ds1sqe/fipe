use std::{
    alloc::{alloc, dealloc, Layout},
    ptr::NonNull,
};

use super::error::BlockError;
// Block Layout
//
//
// Addr   Line  |-------128 byte--------|
// 0x7F00 7F    | MARK BITS             |
// 0x7E00 7E    |                       |
// --------------------------------------
// 0x7D00 7D    | DATA                  |
// 0x7C00 7C    |                       |
// ......................................
//
// 0x0100 1     |                       |
// 0x0000 0     |_______________________|

/// 0x8000  32768 (Byte)
pub const BLOCK_SIZE: usize = 1 << 15;
/// 0x80    128   (Byte)
pub const LINE_SIZE: usize = 1 << 7;

/// 0x100   256
pub const LINE_COUNT: usize = BLOCK_SIZE / LINE_SIZE;

//  0x7F00  32512  (Byte)
pub const BLOCK_CAPACITY: usize = BLOCK_SIZE - LINE_COUNT;
pub const LINE_MARK_START: usize = BLOCK_CAPACITY;

//  0xF    15
pub const ALIGN_WORD: usize = 15;

//  0xFFFF...0
pub const ALIGN_MASK: usize = !(ALIGN_WORD);

pub type CstPtr = *const u8;

#[derive(Debug)]
pub struct Block {
    pub ptr: BlockPointer,
    pub size: BlockSize,
}

pub type BlockPointer = NonNull<u8>;
pub type BlockSize = usize;

impl Block {
    /// create new memory block.
    ///
    /// # Errors
    /// This function will return an error if [size] is not a power of two
    /// or cannot allocate memory
    ///
    pub fn new(size: BlockSize) -> Result<Self, BlockError> {
        if !size.is_power_of_two() {
            return Err(BlockError::BadSize(size));
        }
        Ok(Block {
            size,
            ptr: self::Block::alloc_block(size)?,
        })
    }
    /// allocate block pointer.
    ///
    /// # Errors
    ///
    /// This function will return an error if [size] is not a power of two
    /// or cannot allocate memory
    ///
    /// # Safety
    /// caller must check [size] is safe
    ///
    fn alloc_block(size: BlockSize) -> Result<BlockPointer, BlockError> {
        unsafe {
            let layout = Layout::from_size_align_unchecked(size, size);
            let ptr = alloc(layout);
            if ptr.is_null() {
                return Err(BlockError::OutOfMemory);
            } else {
                return Ok(NonNull::new_unchecked(ptr));
            }
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub fn dealloc_block(ptr: BlockPointer, size: BlockSize) {
        unsafe {
            let layout = Layout::from_size_align_unchecked(size, size);

            dealloc(ptr.as_ptr(), layout);
        }
    }
}
