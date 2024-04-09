use std::ptr::NonNull;

use crate::alloc::ptr::RawPtr;

use super::fat_ptr::FatPtr;

const TAG_MASK: usize = 0x3;
pub const TAG_SYMBOL: usize = 0x0;
pub const TAG_PAIR: usize = 0x1;
pub const TAG_OBJECT: usize = 0x2;
pub const TAG_NUMBER: usize = 0x3;
const PTR_MASK: usize = !0x3;

pub trait Tagged<T> {
    fn tag(self, tag: usize) -> NonNull<T>;
    fn untag(from: NonNull<T>) -> RawPtr<T>;
}

impl<T> Tagged<T> for RawPtr<T> {
    fn tag(self, tag: usize) -> NonNull<T> {
        unsafe { NonNull::new_unchecked((self.as_addr() | tag) as *mut T) }
    }
    fn untag(from: NonNull<T>) -> RawPtr<T> {
        RawPtr::new((from.as_ptr() as usize & PTR_MASK) as *const T)
    }
}

/// Type tagged pointer which carries type infomation in lowest 2 bits
pub union TaggedPtr {
    tag: usize,
    number: isize,
    // symbol:NonNull<Symbol>,
    object: NonNull<()>,
}

impl TaggedPtr {
    fn object<T>(ptr: RawPtr<T>) -> Self {
        Self {
            object: ptr.tag(TAG_OBJECT).cast::<()>(),
        }
    }
}

impl From<FatPtr> for TaggedPtr {
    fn from(value: FatPtr) -> Self {
        todo!()
    }
}
