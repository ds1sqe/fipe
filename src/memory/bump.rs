use std::mem::size_of;

use super::block::Block;

pub const BLOCK_SIZE_BITS: usize = 15;
// 32k 0x8000
pub const BLOCK_SIZE: usize = 1 << BLOCK_SIZE_BITS;

pub type RawPtr = *const u8;

pub struct BumpBlock {
    cursor: RawPtr,
    limit: RawPtr,
    block: Block,
}

impl BumpBlock {
    pub fn inner_alloc(&mut self, alloc_size: usize) -> Option<RawPtr> {
        let block_start_ptr = self.block.as_ptr() as usize;
        let cursor_ptr = self.cursor as usize;

        let align_mask: usize = !(size_of::<usize>() - 1);

        let next_ptr = cursor_ptr.checked_sub(alloc_size)? & align_mask;

        if next_ptr < block_start_ptr {
            None
        } else {
            self.cursor = next_ptr as RawPtr;
            Some(next_ptr as RawPtr)
        }
    }
}
