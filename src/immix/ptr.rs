use std::{cell::Cell, marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{blocks::BlockMeta, mark::Mark};

#[derive(Debug)]
pub struct RawPtr<T: Sized> {
    ptr: NonNull<T>,
}

impl<T: Sized> RawPtr<T> {
    /// create new RawPtr form given `*const` ptr
    pub fn new(ptr: *const T) -> Self {
        Self {
            ptr: NonNull::new(ptr as *mut T)
                .expect("RawPtr requires a non-null pointer"),
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
    /// # Safety
    /// Unsafe because there are no guarantees
    /// about internal `ptr`'s validity
    pub unsafe fn as_ref(&self) -> &T {
        self.ptr.as_ref()
    }
    /// get mut ref to the object
    /// # Safety
    /// Unsafe because there are no guarantees
    /// about internal `ptr`'s validity
    pub unsafe fn as_mut(&mut self) -> &mut T {
        self.ptr.as_mut()
    }
}

impl<T: Sized> Clone for RawPtr<T> {
    fn clone(&self) -> RawPtr<T> {
        *self
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

#[derive(Debug, Clone, PartialEq)]
pub struct MetaPtr {
    pub low: usize,
    pub high: usize,
    pub marks: BlockMeta,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairPtr {
    pub meta: OR<MetaPtr, Rc<Cell<Mark>>>,
    pub data: *const u8,
}

impl PairPtr {
    pub fn set_mark(&mut self, mark: &Mark) {
        match &mut self.meta {
            OR::L(mptr) => {
                mptr.marks.set_mark_range(mark, mptr.low, mptr.high);
            }
            OR::R(mark_cell) => {
                mark_cell.set(*mark);
            }
        }
    }
    pub fn is_unmarked(&self) -> bool {
        match &self.meta {
            OR::L(mptr) => mptr.marks.is_unmarked(mptr.low),
            OR::R(mark_cell) => mark_cell.get() == Mark::Unmarked,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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
