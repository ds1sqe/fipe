// Block Layout
//
//
// Addr   Line  |-------256 byte--------|
// 0xFF00 FF    | MARK BITS             |
// --------------------------------------
// 0xFE00 FE    | DATA                  |
// 0xFD00 7D    |                       |
// 0xFC00 7C    |                       |
// ......................................
//
// 0x0100 1     |                       |
// 0x0000 0     |_______________________|

use std::{
    alloc::{alloc, dealloc, Layout},
    mem::replace,
    ptr::{write, NonNull},
};

use super::errors::{AllocError, BlockError};

/// 0x10000  65536 (Byte)
pub const BLOCK_SIZE: usize = 1 << 16;
/// 0x100    256   (Byte)
pub const LINE_SIZE: usize = 1 << 8;

/// 0x100    256
pub const LINE_COUNT: usize = BLOCK_SIZE / LINE_SIZE;

//  0xFF00   65280 (Byte)
pub const BLOCK_CAPACITY: usize = BLOCK_SIZE - LINE_COUNT;
pub const LINE_MARK_START: usize = BLOCK_CAPACITY;

//  0xF    15
pub const ALIGN_WORD: usize = 15;

//  0xFFFF...0
pub const ALIGN_MASK: usize = !(ALIGN_WORD);

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

#[derive(Debug)]
pub struct BlockMeta {
    lines: *mut u8,
}

#[derive(Debug)]
pub struct Hole {
    pub cursor: usize,
    pub limit: usize,
}

impl BlockMeta {
    pub fn new(block_ptr: *const u8) -> BlockMeta {
        let mut meta = BlockMeta {
            lines: unsafe { block_ptr.add(LINE_MARK_START) as *mut u8 },
        };
        meta.reset();
        meta
    }
    /// search hole downward
    pub fn find_next_hole(&self, starting_at: usize, alloc_size: usize) -> Option<Hole> {
        // The count of consecutive available holes.
        let mut count = 0;

        let starting_line = starting_at / LINE_SIZE;

        // celi up to LINE_SIZE
        let lines_required = (alloc_size + LINE_SIZE - 1) / LINE_SIZE;

        let mut end = starting_line;

        for index in (0..starting_line).rev() {
            let marked = unsafe { *self.lines.add(index) };

            if marked == 0 {
                // Count unmarked lines
                count += 1;

                if index == 0 && count >= lines_required {
                    // We reached line zero and
                    // We have extra space
                    let cursor = end * LINE_SIZE;
                    let limit = index * LINE_SIZE;
                    // cursor >= limit
                    return Some(Hole { cursor, limit });
                }
            } else {
                // This line is marked
                if count > lines_required {
                    let cursor = end * LINE_SIZE;
                    // add 2 to index because,
                    // 1 for walking back from the current marked line,
                    // 1 for walking back from the previous conservatively marked line
                    let limit = (index + 2) * LINE_SIZE;
                    // cursor >= limit
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

    /// Reset all mark flags to unmarked.
    pub fn reset(&mut self) {
        unsafe {
            for idx in 0..LINE_COUNT {
                *self.lines.add(idx) = 0;
            }
        }
    }

    /// Mark the indexed line
    pub fn mark_line(&mut self, idx: usize) {
        unsafe { *self.as_line_mark(idx) = 1 };
    }
    /// Unmark the indexed line
    pub fn unmark_line(&mut self, idx: usize) {
        unsafe { *self.as_line_mark(idx) = 0 };
    }
    pub fn print_mark_status(&self) {
        unsafe {
            for idx in 0..LINE_COUNT {
                let mark = *self.lines.add(idx);
                print!("{mark}");
                if (idx + 1) % 64 == 0 {
                    println!();
                }
            }
        }
    }

    pub fn is_maked(&self, idx: usize) -> bool {
        unsafe { *self.as_ref_line_mark(idx) == 1 }
    }

    unsafe fn as_line_mark(&mut self, line: usize) -> &mut u8 {
        &mut *self.lines.add(line)
    }
    unsafe fn as_ref_line_mark(&self, line: usize) -> &u8 {
        &*self.lines.add(line)
    }
}

#[derive(Debug)]
pub struct BumpBlock {
    /// index of last written object
    cursor: *const u8,
    /// index of limit
    limit: *const u8,
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

    pub fn inner_alloc(&mut self, alloc_size: usize) -> Result<*const u8, BlockError> {
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
            self.meta.print_mark_status();
            // celi up to LINE_SIZE
            let mark_idx = (self.cursor as usize - self.block.as_ptr() as usize) / BLOCK_CAPACITY;
            dbg!(mark_idx);
            self.meta.mark_line(mark_idx - 1);
            self.meta.print_mark_status();
            self.cursor = next_ptr as *const u8;
            Ok(next_ptr as *const u8)
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

pub struct BlockList {
    pub head: Option<BumpBlock>,
    pub overflow: Option<BumpBlock>,
    pub rest: Vec<BumpBlock>,
}

impl BlockList {
    pub fn new() -> BlockList {
        BlockList {
            head: None,
            overflow: None,
            rest: Vec::new(),
        }
    }

    pub fn overflow_alloc(&mut self, alloc_size: usize) -> Result<*const u8, AllocError> {
        match self.overflow {
            Some(ref mut overflow) => match overflow.inner_alloc(alloc_size) {
                Ok(space) => Ok(space),
                Err(__) => {
                    // current bump block is full

                    let new_bump = BumpBlock::new();
                    if new_bump.is_err() {
                        return Err(AllocError::OutOfMemory);
                    }
                    let new_bump = new_bump.unwrap();

                    let previous = replace(overflow, new_bump);

                    self.rest.push(previous);

                    let space = overflow.inner_alloc(alloc_size);

                    if space.is_err() {
                        return Err(AllocError::OutOfMemory);
                    }
                    Ok(space.unwrap())
                }
            },
            None => {
                let overflow = BumpBlock::new();
                if overflow.is_err() {
                    return Err(AllocError::OutOfMemory);
                }
                let mut overflow = overflow.unwrap();

                let space = overflow
                    .inner_alloc(alloc_size)
                    .expect("We expect this object to fit!");

                self.overflow = Some(overflow);

                Ok(space)
            }
        }
    }
}
