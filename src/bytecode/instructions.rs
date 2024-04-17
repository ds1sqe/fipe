use std::{alloc, alloc::Layout, ptr::NonNull};

use super::instruction::Instruction;

pub struct Instructions {
    byte: NonNull<u8>,
    cap: usize,
    len: usize,
}

const SIZE: usize = 1 << 16;

impl Instructions {
    pub fn new() -> Self {
        let layout = Layout::array::<u8>(SIZE).unwrap();

        let new_ptr = unsafe { alloc::alloc(layout) };

        let ptr = match NonNull::new(new_ptr as *mut u8) {
            Some(p) => p,
            None => alloc::handle_alloc_error(layout),
        };

        Self {
            byte: ptr,
            cap: 0,
            len: 0,
        }
    }

    pub fn add_instruction(&mut self, ins: Instruction) {
        if self.len + ins.opcode().length() >= self.cap {
            self.grow();
        }
    }

    fn grow(&mut self) {
        let new_cap = 2 * self.cap;
        let new_layout = Layout::array::<u8>(new_cap).unwrap();

        assert!(
            new_layout.size() <= isize::MAX as usize,
            "Too large to allocate"
        );

        let old_layout = Layout::array::<u8>(self.cap).unwrap();
        let old_ptr = self.byte.as_ptr() as *mut u8;
        let new_ptr =
            unsafe { alloc::realloc(old_ptr, old_layout, new_layout.size()) };

        self.byte = match NonNull::new(new_ptr as *mut u8) {
            Some(p) => p,
            None => alloc::handle_alloc_error(new_layout),
        };
        self.cap = new_cap
    }
}

impl Drop for Instructions {
    fn drop(&mut self) {
        unsafe {
            alloc::dealloc(
                self.byte.as_ptr() as *mut u8,
                Layout::array::<u8>(self.cap).unwrap(),
            );
        }
    }
}
