use std::{
    alloc::{alloc, dealloc, Layout},
    ptr::NonNull,
};

pub trait AllocRaw {
    fn alloc<T>(&self, object: T) -> *const T;
}

#[derive(Debug)]
pub struct Block {
    ptr: BlockPointer,
    size: BlockSize,
}

pub type BlockPointer = NonNull<u8>;
pub type BlockSize = usize;

#[derive(Debug)]
pub enum BlockError {
    /// block size is not a power of two
    BadSize(usize),
    /// Cannot allocate memory
    OutOfMemory,
}

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

#[test]
fn test_block_alignment() {
    let size = 64;
    let block = Block::new(size).unwrap();
    // the block address bitwise AND the alignment bits (size - 1) should
    // be a mutually exclusive set of bits
    let mask = size - 1;
    assert!((block.ptr.as_ptr() as usize & mask) ^ mask == mask);
}
