use std::{
    alloc::{alloc, dealloc, Layout},
    ptr::NonNull,
};

use super::errors::BlockError;

pub trait AllocRaw {
    fn alloc<T>(&self, object: T) -> *const T;
}

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
