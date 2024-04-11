use std::{marker::PhantomData, ptr::NonNull};

use super::{
    blocks::{BumpBlock, LargeBlock},
    mark::Mark,
};

#[derive(Debug)]
pub struct RawPtr<T: Sized> {
    ptr: NonNull<T>,
}

impl<T: Sized> RawPtr<T> {
    /// create new RawPtr form given `*const` ptr
    pub fn new(ptr: *const T) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(ptr as *mut T) },
        }
    }

    /// cast self into raw pointer
    pub fn as_ptr(self) -> *const T {
        self.ptr.as_ptr()
    }

    /// get address
    pub fn as_addr(self) -> usize {
        self.ptr.as_ptr() as usize
    }

    /// get ref to the object
    /// [safety] Unsafe because there are no guarantees
    /// about internal `ptr`'s validity
    pub unsafe fn as_ref(&self) -> &T {
        self.ptr.as_ref()
    }
    /// get mut ref to the object
    /// [safety] Unsafe because there are no guarantees
    /// about internal `ptr`'s validity
    pub unsafe fn as_mut(&mut self) -> &mut T {
        self.ptr.as_mut()
    }
}

impl<T: Sized> Clone for RawPtr<T> {
    fn clone(&self) -> RawPtr<T> {
        RawPtr { ptr: self.ptr }
    }
}

impl<T: Sized> Copy for RawPtr<T> {}

impl<T: Sized> PartialEq for RawPtr<T> {
    fn eq(&self, other: &RawPtr<T>) -> bool {
        self.ptr == other.ptr
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OR<L, R> {
    L(L),
    R(R),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetaPtr {
    pub low: usize,
    pub high: usize,
    pub block: NonNull<BumpBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairPtr {
    pub meta: OR<MetaPtr, RawPtr<LargeBlock>>,
    pub data: *const u8,
}

impl PairPtr {
    pub fn set_mark(&mut self, mark: &Mark) {
        match &mut self.meta {
            OR::L(mptr) => {
                unsafe {
                    mptr.block
                        .as_mut()
                        .meta
                        .set_mark_range(mark, mptr.low, mptr.high)
                };
            }
            OR::R(lblk) => unsafe {
                lblk.as_mut().set_mark(mark);
            },
        }
    }
    pub fn is_unmarked(&self) -> bool {
        match &self.meta {
            OR::L(mptr) => unsafe {
                return mptr.block.as_ref().meta.is_unmarked(mptr.low);
            },
            OR::R(lblk) => unsafe {
                return lblk.as_ref().is_unmarked();
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedPtr<T> {
    pub ptr: PairPtr,
    pub tag: PhantomData<T>,
}

impl<T> TypedPtr<T> {
    pub fn as_ptr(&self) -> *mut T {
        self.ptr.data as *mut T
    }

    pub fn set_mark(&mut self, mark: &Mark) {
        self.ptr.set_mark(mark)
    }
}
