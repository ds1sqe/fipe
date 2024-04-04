use std::mem::size_of;

use super::{block::Block, errors::BumpError};

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

pub type RawPtr = *const u8;

pub struct BumpBlock {
    /// index of last written object
    cursor: RawPtr,
    ///
    limit: RawPtr,
    /// memory block
    block: Block,

    meta: BlockMeta,
}

pub struct BlockMeta {
    lines: *mut u8,
}

pub struct Hole {
    cursor: usize,
    limit: usize,
}

impl BumpBlock {
    pub fn inner_alloc(&mut self, alloc_size: usize) -> Result<RawPtr, BumpError> {
        let block_start_ptr = self.block.as_ptr() as usize;
        let cursor_ptr = self.cursor as usize;

        // align down to word boundary
        let align_mask: usize = !(size_of::<usize>() - 1);

        let cursor_pos_opt = cursor_ptr.checked_sub(alloc_size);
        if cursor_pos_opt.is_none() {
            return Err(BumpError::AddressOverflow);
        }
        let next_pos = cursor_pos_opt.unwrap();

        let next_ptr = next_pos & align_mask;

        if next_ptr < block_start_ptr {
            // if allocation start lower than block beginning,
            // which stands for that there is no space in block
            // for this allocation
            Err(BumpError::NoSpaceForAllocation)
        } else {
            self.cursor = next_ptr as RawPtr;
            Ok(next_ptr as RawPtr)
        }
    }
}

impl BlockMeta {
    pub fn find_next_hole(
        &self,
        starting_at: usize,
        alloc_size: usize,
    ) -> Option<Hole> {
        // The count of consecutive available holes.
        let mut count = 0;

        let starting_line = starting_at / LINE_SIZE;
        let lines_required = (alloc_size + LINE_SIZE - 1) / LINE_SIZE;

        let mut end = starting_line;

        for index in (0..starting_line).rev() {
            let marked = unsafe { *self.lines.add(index) };

            if marked == 0 {
                // Count unmarked lines
                count += 1;

                if index == 0 && count >= lines_required {
                    // We have extra space
                    let cursor = end * LINE_SIZE;
                    let limit = index * LINE_SIZE;
                    return Some(Hole { cursor, limit });
                }
            } else {
                // This block is marked
                if count > lines_required {
                    // but at least 2 previous blocks were not marked.
                    // returning the hole, considering immediately
                    // preceding block as conservatively marked
                    let cursor = end * LINE_SIZE;
                    let limit = (index + 2) * LINE_SIZE;
                    return Some(Hole { cursor, limit });
                }

                // There was no consecutive space,
                // so reset the hole search state.
                count = 0;
                end = index;
            }
        }
        None
    }
}
