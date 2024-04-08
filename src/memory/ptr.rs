use std::ptr::NonNull;

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
