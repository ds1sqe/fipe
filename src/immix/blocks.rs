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
    collections::{HashMap, VecDeque},
    fmt,
    mem::replace,
    ptr::{write, NonNull},
};

use super::{
    errors::{AllocError, BlockError},
    mark::Mark,
    ptr::{MetaPtr, PairPtr, RawPtr, OR},
    size::SizeClass,
};

/// 0x10000  65536 (Byte)
pub const BLOCK_SIZE: usize = 1 << 16;
/// 0x100    256   (Byte)
pub const LINE_SIZE: usize = 1 << 8;

/// 0x100    256
pub const LINE_COUNT: usize = BLOCK_SIZE / LINE_SIZE;

//  0xFF00   65280 (Byte)
pub const BLOCK_CAPACITY: usize = BLOCK_SIZE - LINE_COUNT;
pub const LINE_MARK_START: usize = BLOCK_CAPACITY;

//  0xFF     255
pub const ALIGN_WORD: usize = (1 << 8) - 1;

//  0xFFFF..00
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
                Err(BlockError::OutOfMemory)
            } else {
                Ok(NonNull::new_unchecked(ptr))
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

pub struct BlockMeta {
    lines: *mut Mark,
}

#[derive(Debug)]
pub struct Hole {
    pub start: usize,
    pub end: usize,
}

impl BlockMeta {
    /// Creates a new [`BlockMeta`].
    ///
    /// # Safety
    ///
    /// from `block_ptr` to `block_ptr` + `LINE_MARK_START`
    /// have to be available
    pub unsafe fn new(block_ptr: *const u8) -> BlockMeta {
        let mut meta = BlockMeta {
            lines: unsafe { block_ptr.add(LINE_MARK_START) as *mut Mark },
        };
        meta.reset();
        meta
    }

    /// search hole upward
    pub fn find_hole(
        &self,
        start_byte: usize,
        alloc_size: usize,
    ) -> Option<Hole> {
        // The count of consecutive available holes.
        let mut count = 0;

        let starting_line = start_byte / LINE_SIZE;

        // celi up to LINE_SIZE
        let lines_required = (alloc_size + LINE_SIZE - 1) / LINE_SIZE;

        let end = LINE_COUNT;

        for index in starting_line..end {
            let marked = unsafe { *self.lines.add(index) };

            if marked == Mark::Unmarked {
                // Count unmarked lines
                count += 1;
            } else {
                if count >= lines_required {
                    // we have found space
                    let start = index - count;
                    let end = index;
                    return Some(Hole { start, end });
                }
                // There was no consecutive space,
                // so reset the hole search state.
                count = 0;
            }
        }
        None
    }
    /// Reset all mark flags to unmarked.
    pub fn reset(&mut self) {
        unsafe {
            for idx in 0..LINE_COUNT {
                *self.lines.add(idx) = Mark::Unmarked;
            }
        }
    }

    /// Mark the indexed line
    pub fn mark_line(&mut self, idx: usize) {
        unsafe { *self.as_line_mark(idx) = Mark::Marked };
    }
    /// Mark the range, caller must check low and high is safe
    pub fn mark_range(&mut self, low: usize, high: usize) {
        unsafe {
            for idx in low..=high {
                *self.lines.add(idx) = Mark::Marked;
            }
        }
    }
    /// Unmark the indexed line
    pub fn unmark_line(&mut self, idx: usize) {
        unsafe { *self.as_line_mark(idx) = Mark::Unmarked };
    }
    /// Unmark the range, caller must check low and high is safe
    pub fn unmark_range(&mut self, low: usize, high: usize) {
        unsafe {
            for idx in low..=high {
                *self.lines.add(idx) = Mark::Unmarked;
            }
        }
    }
    /// Set mark the indexed line
    pub fn set_mark_line(&mut self, mark: &Mark, idx: usize) {
        unsafe { *self.as_line_mark(idx) = *mark };
    }
    /// Set mark the range, caller must check low and high is safe
    pub fn set_mark_range(&mut self, mark: &Mark, low: usize, high: usize) {
        unsafe {
            for idx in low..=high {
                *self.lines.add(idx) = *mark;
            }
        }
    }

    /// Returns the mark status str of this [`BlockMeta`].
    pub fn mark_status_str(&self) -> String {
        unsafe {
            let mut buf = String::new();
            for idx in 0..LINE_COUNT {
                let mark = self.lines.add(idx);
                let flag = match *mark {
                    Mark::Unmarked => '-',
                    Mark::Allocated => 'A',
                    Mark::Marked => 'M',
                };
                buf += &format!("{flag}");
                if (idx + 1) % 8 == 0 {
                    buf += " ";
                }
                if (idx + 1) % 64 == 0 {
                    buf += &format!(" {idx}\n");
                }
            }
            buf
        }
    }
    pub fn mark_status_vec_str(&self) -> Vec<String> {
        unsafe {
            let mut vec = Vec::new();
            for line in 0..4 {
                let mut buf = String::new();

                for idx in 0..64 {
                    let mark = self.lines.add(line * 64 + idx);
                    let flag = match *mark {
                        Mark::Unmarked => '-',
                        Mark::Allocated => 'A',
                        Mark::Marked => 'M',
                    };
                    buf += &format!("{flag}");
                    if (idx + 1) % 8 == 0 {
                        buf += " ";
                    }
                    if (idx + 1) % 64 == 0 {}
                }
                buf += &format!("{}", (line + 1) * 64 - 1);
                vec.push(buf);
            }

            vec
        }
    }
    pub fn is_marked(&self, idx: usize) -> bool {
        unsafe { *self.as_ref_line_mark(idx) == Mark::Marked }
    }
    pub fn is_unmarked(&self, idx: usize) -> bool {
        unsafe { *self.as_ref_line_mark(idx) == Mark::Unmarked }
    }

    unsafe fn as_line_mark(&mut self, line: usize) -> &mut Mark {
        &mut *self.lines.add(line)
    }
    unsafe fn as_ref_line_mark(&self, line: usize) -> &Mark {
        &*self.lines.add(line)
    }

    pub fn is_fresh(&self) -> bool {
        unsafe {
            for idx in 0..LINE_COUNT {
                if !(*self.lines.add(idx) == Mark::Unmarked) {
                    return false;
                }
            }
        }
        true
    }
}

impl fmt::Debug for BlockMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let vec_str = self.mark_status_vec_str();
        f.debug_struct("BlockMeta")
            .field("  0", &vec_str[0])
            .field(" 64", &vec_str[1])
            .field("128", &vec_str[2])
            .field("192", &vec_str[3])
            .finish()
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
    pub meta: BlockMeta,
}

impl BumpBlock {
    pub fn new() -> Result<Self, BlockError> {
        let inner_block = Block::new(BLOCK_SIZE)?;
        let block_ptr = inner_block.as_ptr();

        let block = BumpBlock {
            cursor: block_ptr,
            limit: unsafe { block_ptr.add(BLOCK_CAPACITY) },
            block: inner_block,
            meta: unsafe { BlockMeta::new(block_ptr) },
        };
        Ok(block)
    }

    pub fn inner_alloc(
        &mut self,
        alloc_size: usize,
    ) -> Result<(MetaPtr, *const u8), BlockError> {
        let cursor_ptr = self.cursor as usize;
        let limit = self.limit as usize;

        let cursor_pos_opt = cursor_ptr.checked_add(alloc_size);
        if cursor_pos_opt.is_none() {
            return Err(BlockError::AddressOverflow);
        }
        let next_pos = cursor_pos_opt.unwrap();
        // align up to word boundary
        let next_ptr = (next_pos + 0xFF) & ALIGN_MASK;

        // if next_ptr == limit, we have to find hole next time,
        // and then, next_ptr > limit.
        if next_ptr <= limit {
            let mark_low = (self.cursor as usize
                - self.block.as_ptr() as usize)
                / LINE_SIZE;
            let mark_high =
                (next_pos - self.block.as_ptr() as usize) / LINE_SIZE;
            self.meta
                .set_mark_range(&Mark::Allocated, mark_low, mark_high);
            let cursor = self.cursor;
            self.cursor = next_ptr as *const u8;

            let self_ptr = NonNull::new(self);
            if self_ptr.is_none() {
                return Err(BlockError::OutOfMemory);
            }
            let ptr = MetaPtr {
                low: mark_low,
                high: mark_high,
                block: self_ptr.unwrap(),
            };
            Ok((ptr, cursor))
        } else {
            // try find hole.
            if let Some(Hole { start, end }) =
                self.meta.find_hole(0, alloc_size)
            {
                let cursor = start * LINE_SIZE;
                let limit = end * LINE_SIZE;
                self.cursor = unsafe { self.block.as_ptr().add(cursor) };
                self.limit = unsafe { self.block.as_ptr().add(limit) };
                return self.inner_alloc(alloc_size);
            }
            // There is no space in block for this allocation
            Err(BlockError::NoSpaceForAllocation)
        }
    }
    /// write object to `self.block`
    ///
    /// # Safety
    ///
    /// this function will access a raw ptr of self.block
    unsafe fn write<T>(&mut self, object: T, offset: usize) -> *const T {
        #![allow(dead_code)]
        let ptr = self.block.as_ptr().add(offset) as *mut T;
        write(ptr, object);
        ptr
    }

    pub fn current_hole_size(&self) -> usize {
        self.cursor as usize - self.limit as usize
    }

    pub fn is_fresh(&self) -> bool {
        self.meta.is_fresh()
    }

    pub fn unmark(&mut self) {
        self.meta.reset();
    }
}

#[derive(Debug)]
pub struct LargeBlock {
    pub ptr: BlockPointer,
    pub size: BlockSize,
    pub mark: Mark,
}
impl LargeBlock {
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
        Ok(LargeBlock {
            size,
            ptr: self::LargeBlock::alloc_block(size)?,
            mark: Mark::Allocated,
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
                Err(BlockError::OutOfMemory)
            } else {
                Ok(NonNull::new_unchecked(ptr))
            }
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub fn dealloc_block(self) {
        unsafe {
            let layout =
                Layout::from_size_align_unchecked(self.size, self.size);

            dealloc(self.ptr.as_ptr(), layout);
        }
    }

    pub fn set_mark(&mut self, mark: &Mark) {
        self.mark = *mark;
    }

    pub fn unmark(&mut self) {
        self.mark = Mark::Unmarked;
    }

    pub fn is_marked(&self) -> bool {
        self.mark == Mark::Marked
    }
    pub fn is_unmarked(&self) -> bool {
        self.mark == Mark::Unmarked
    }
}

#[derive(Debug)]
pub struct BlockList {
    pub head: VecDeque<BumpBlock>,
    pub large: HashMap<usize, LargeBlock>,
    pub rest: HashMap<usize, BumpBlock>,
    count: usize,
}

impl BlockList {
    pub fn new() -> BlockList {
        BlockList {
            head: VecDeque::new(),
            large: HashMap::new(),
            rest: HashMap::new(),
            count: 0,
        }
    }

    pub fn alloc(&mut self, alloc_size: usize) -> Result<PairPtr, AllocError> {
        let size = SizeClass::from(alloc_size)?;
        match size {
            SizeClass::Small | SizeClass::Medium => self.head_alloc(alloc_size),
            SizeClass::Large => self.large_alloc(alloc_size),
        }
    }

    fn head_alloc(&mut self, alloc_size: usize) -> Result<PairPtr, AllocError> {
        match self.head.is_empty() {
            false => match self.head[0].inner_alloc(alloc_size) {
                Ok((meta_ptr, cursor)) => Ok(PairPtr {
                    meta: OR::L(meta_ptr),
                    data: cursor,
                }),
                Err(__failed) => {
                    // head[0] bump block is full
                    if self.head.len() != 1 {
                        self.rest
                            .insert(self.count, self.head.pop_front().unwrap());
                        self.count += 1;
                        // recursion
                        self.head_alloc(alloc_size)
                    } else {
                        let new_bump = BumpBlock::new();
                        if new_bump.is_err() {
                            return Err(AllocError::OutOfMemory);
                        }
                        let new_bump = new_bump.unwrap();

                        let previous = replace(&mut self.head[0], new_bump);

                        self.rest.insert(self.count, previous);
                        self.count += 1;

                        let space = self.head[0].inner_alloc(alloc_size);

                        if space.is_err() {
                            return Err(AllocError::OutOfMemory);
                        }
                        let space = space.unwrap();
                        Ok(PairPtr {
                            meta: OR::L(space.0),
                            data: space.1,
                        })
                    }
                }
            },
            true => {
                let new_block = BumpBlock::new();
                if new_block.is_err() {
                    return Err(AllocError::OutOfMemory);
                }
                let new_block = new_block.unwrap();

                self.head.push_back(new_block);

                // recursion
                self.head_alloc(alloc_size)
            }
        }
    }

    fn large_alloc(
        &mut self,
        alloc_size: usize,
    ) -> Result<PairPtr, AllocError> {
        let size = alloc_size.next_power_of_two();
        let new_large_block = LargeBlock::new(size)?;
        self.large.insert(self.count, new_large_block);

        let ptr = self.large.get(&self.count).unwrap().as_ptr();
        self.count += 1;
        let rst = PairPtr {
            meta: OR::R(RawPtr::new(ptr as *const LargeBlock)),
            data: ptr,
        };
        Ok(rst)
    }

    fn dealloc_large(&mut self) {
        let mut drop_list = Vec::new();
        for (idx, lblk) in self.large.iter() {
            if lblk.is_unmarked() {
                drop_list.push(*idx);
            }
        }

        for idx in drop_list {
            let removed = self.large.remove(&idx);
            removed.unwrap().dealloc_block();
        }
    }

    fn recycle(&mut self) {
        let mut fresh_list = Vec::new();

        for (idx, blk) in self.rest.iter() {
            if blk.is_fresh() {
                fresh_list.push(*idx);
            }
        }

        for idx in fresh_list {
            self.head.push_back(self.rest.remove(&idx).unwrap());
        }
    }

    pub fn sweep(&mut self) {
        self.dealloc_large();
        self.recycle();
    }

    pub fn unmark_all(&mut self) {
        for (_, lblk) in self.large.iter_mut() {
            lblk.unmark();
        }
        if !self.head.is_empty() {
            self.head[0].meta.reset();
        }
        for (_, blk) in self.rest.iter_mut() {
            blk.unmark();
        }
    }
}

impl Default for BlockList {
    fn default() -> Self {
        Self::new()
    }
}
