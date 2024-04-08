pub mod fat_ptr;
pub mod tagged_ptr;
mod value;

use std::cell::Cell;

use crate::alloc::ptr::RawPtr;

pub trait Mutatorscope {}

pub struct ScopedPtr<'guard, T: Sized> {
    value: &'guard T,
}

impl<'guard, T: Sized> ScopedPtr<'guard, T> {
    pub fn new(_guard: &'guard dyn Mutatorscope, value: &'guard T) -> ScopedPtr<'guard, T> {
        ScopedPtr { value }
    }
}

pub struct CellPtr<T: Sized> {
    inner: Cell<RawPtr<T>>,
}

pub trait ScopedRef<T> {
    fn scoped_ref<'scope>(&self, guard: &'scope dyn Mutatorscope) -> &'scope T;
}

impl<T> ScopedRef<T> for RawPtr<T> {
    fn scoped_ref<'scope>(&self, _guard: &'scope dyn Mutatorscope) -> &'scope T {
        unsafe { &*self.as_ptr() }
    }
}

impl<T: Sized> CellPtr<T> {
    pub fn get<'guard>(&self, guard: &'guard dyn Mutatorscope) -> ScopedPtr<'guard, T> {
        ScopedPtr::new(guard, self.inner.get().scoped_ref(guard))
    }
}
