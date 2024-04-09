use std::mem::size_of;

pub mod blocks;
pub mod errors;
pub mod mark;
pub mod rawptr;
pub mod size;
/// Align up to double word boundary
pub fn alloc_size_of(object_size: usize) -> usize {
    // align will be 4 in 32bit machine
    //   and will be 8 in 64bit machine
    let align = size_of::<usize>();
    // align to double word boundary
    (object_size + (align - 1)) & !(align - 1)
}
