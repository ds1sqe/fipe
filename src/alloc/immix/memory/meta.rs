use super::block::{LINE_COUNT, LINE_MARK_START, LINE_SIZE};

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
    /// Mark the entire block
    pub fn mark_block(&mut self, idx: usize) {
        unsafe { *self.as_block_mark() = 1 };
    }

    unsafe fn as_line_mark(&mut self, line: usize) -> &mut u8 {
        &mut *self.lines.add(line)
    }

    unsafe fn as_block_mark(&mut self) -> &mut u8 {
        &mut *self.lines.add(LINE_COUNT - 1)
    }
}
