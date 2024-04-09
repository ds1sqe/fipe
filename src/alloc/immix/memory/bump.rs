use std::ptr::write;

use super::block::{Block, CstPtr, ALIGN_MASK, BLOCK_CAPACITY, BLOCK_SIZE};
use super::error::BlockError;
use super::meta::{BlockMeta, Hole};

#[derive(Debug)]
pub struct BumpBlock {
    /// index of last written object
    cursor: CstPtr,
    /// index of limit
    limit: CstPtr,
    /// memory block
    block: Block,
    /// line mark data
    meta: BlockMeta,
}

impl BumpBlock {
    pub fn new() -> Result<Self, BlockError> {
        let inner_block = Block::new(BLOCK_SIZE)?;
        let block_ptr = inner_block.as_ptr();

        let block = BumpBlock {
            cursor: unsafe { block_ptr.add(BLOCK_CAPACITY) },
            limit: block_ptr,
            block: inner_block,
            meta: BlockMeta::new(block_ptr),
        };
        Ok(block)
    }

    pub fn inner_alloc(&mut self, alloc_size: usize) -> Result<CstPtr, BlockError> {
        let limit = self.limit as usize;
        let cursor_ptr = self.cursor as usize;

        let cursor_pos_opt = cursor_ptr.checked_sub(alloc_size);
        if cursor_pos_opt.is_none() {
            return Err(BlockError::AddressOverflow);
        }
        let next_pos = cursor_pos_opt.unwrap();

        // align down to word boundary
        let next_ptr = next_pos & ALIGN_MASK;

        if next_ptr < limit {
            let block_relative_limit =
                unsafe { self.limit.sub(self.block.as_ptr() as usize) } as usize;
            if block_relative_limit > 0 {
                if let Some(Hole { cursor, limit }) =
                    self.meta.find_next_hole(block_relative_limit, alloc_size)
                {
                    self.cursor = unsafe { self.block.as_ptr().add(cursor) };
                    self.limit = unsafe { self.block.as_ptr().add(limit) };
                    return self.inner_alloc(alloc_size);
                }
            }
            // if block_relative_limit <= 0, it means that
            // There is no space in block for this allocation
            Err(BlockError::NoSpaceForAllocation)
        } else {
            self.cursor = next_ptr as CstPtr;
            Ok(next_ptr as CstPtr)
        }
    }

    unsafe fn write<T>(&mut self, object: T, offset: usize) -> *const T {
        let ptr = self.block.as_ptr().add(offset) as *mut T;
        write(ptr, object);
        ptr
    }

    pub fn current_hole_size(&self) -> usize {
        self.cursor as usize - self.limit as usize
    }
}
