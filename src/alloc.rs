mod errors;
mod immix;
mod ptr;
mod size;

use std::{mem::size_of, ptr::NonNull};

use self::{errors::AllocError, ptr::RawPtr, size::SizeClass};

pub type ArraySize = usize;

pub enum Mark {
    Allocated,
    Unmarked,
    Marked,
}

pub trait AllocRaw {
    type Header: AllocHeader;

    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, AllocError>
    where
        T: AllocObject<<Self::Header as AllocHeader>::TypeId>;

    fn get_header(object: NonNull<()>) -> NonNull<Self::Header>;

    fn get_object(header: NonNull<Self::Header>) -> NonNull<()>;

    fn alloc_array(&self, size_bytes: ArraySize) -> Result<RawPtr<u8>, AllocError>;
}

pub trait AllocHeader: Sized {
    /// Allocated object type
    type TypeId: AllocTypeId;

    /// create new header for object type O
    fn new<O: AllocObject<Self::TypeId>>(size: usize, size_class: SizeClass, mark: Mark) -> Self;

    /// create new header for array
    fn new_array(size: ArraySize, size_class: SizeClass, mark: Mark) -> Self;

    /// Set the mark to marked
    fn mark(&mut self);

    /// get bool for mark value
    fn is_marked(&self) -> bool;

    /// get size class of the object
    fn size_class(&self) -> SizeClass;

    /// get the size of the object
    fn size(&self) -> usize;

    /// get the type of the object
    fn type_id(&self) -> Self::TypeId;
}

pub trait AllocTypeId: Copy + Clone {}

pub trait AllocObject<T: AllocTypeId> {
    const TYPE_ID: T;
}

/// Align up to double word boundary
pub fn alloc_size_of(object_size: usize) -> usize {
    // align will be 4 in 32bit machine
    //   and will be 8 in 64bit machine
    let align = size_of::<usize>();
    // align to double word boundary
    (object_size + (align - 1)) & !(align - 1)
}
