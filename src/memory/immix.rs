use std::ptr::{write, NonNull};
use std::slice::from_raw_parts_mut;
use std::{cell::UnsafeCell, marker::PhantomData};

use std::mem::{replace, size_of};

use super::alloc::{alloc_size_of, AllocHeader, AllocObject, AllocRaw, ArraySize, Mark};
use super::block::{BumpBlock, CstPtr};
use super::errors::AllocError;
use super::ptr::RawPtr;
use super::size::SizeClass;

struct BlockList {
    head: Option<BumpBlock>,
    overflow: Option<BumpBlock>,
    rest: Vec<BumpBlock>,
}

impl BlockList {
    fn new() -> BlockList {
        BlockList {
            head: None,
            overflow: None,
            rest: Vec::new(),
        }
    }

    fn overflow_alloc(&mut self, alloc_size: usize) -> Result<*const u8, AllocError> {
        match self.overflow {
            Some(ref mut overflow) => match overflow.inner_alloc(alloc_size) {
                Ok(space) => Ok(space),
                Err(__) => {
                    // current bump block is full

                    let new_bump = BumpBlock::new();
                    if new_bump.is_err() {
                        return Err(AllocError::OutOfMemory);
                    }
                    let new_bump = new_bump.unwrap();

                    let previous = replace(overflow, new_bump);

                    self.rest.push(previous);

                    let space = overflow.inner_alloc(alloc_size);

                    if space.is_err() {
                        return Err(AllocError::OutOfMemory);
                    }
                    Ok(space.unwrap())
                }
            },
            None => {
                let overflow = BumpBlock::new();
                if overflow.is_err() {
                    return Err(AllocError::OutOfMemory);
                }
                let mut overflow = overflow.unwrap();

                let space = overflow
                    .inner_alloc(alloc_size)
                    .expect("We expect this object to fit!");

                self.overflow = Some(overflow);

                Ok(space)
            }
        }
    }
}

pub struct Immix<T> {
    blocks: UnsafeCell<BlockList>,
    _header_type: PhantomData<*const T>,
}

impl<T> Immix<T> {
    pub fn new() -> Immix<T> {
        Immix {
            blocks: UnsafeCell::new(BlockList::new()),
            _header_type: PhantomData,
        }
    }

    fn find_space(&self, alloc_size: usize, size_class: SizeClass) -> Result<CstPtr, AllocError> {
        let blocks = unsafe { &mut *self.blocks.get() };

        if size_class == SizeClass::Large {
            return Err(AllocError::SizeTooBig);
        }

        let space = match blocks.head {
            Some(ref mut head) => {
                if size_class == SizeClass::Medium && alloc_size > head.current_hole_size() {
                    return blocks.overflow_alloc(alloc_size);
                }

                match head.inner_alloc(alloc_size) {
                    Ok(space) => space,
                    Err(__) => {
                        let new_bump_rst = BumpBlock::new();
                        if new_bump_rst.is_err() {
                            return Err(AllocError::InternalError(new_bump_rst.unwrap_err()));
                        }
                        let previous = replace(head, new_bump_rst.unwrap());

                        blocks.rest.push(previous);

                        let ptr_rst = head.inner_alloc(alloc_size);
                        if ptr_rst.is_err() {
                            return Err(AllocError::InternalError(ptr_rst.unwrap_err()));
                        }

                        ptr_rst.unwrap()
                    }
                }
            }

            None => {
                let head = BumpBlock::new();
                if head.is_err() {
                    return Err(AllocError::InternalError(head.unwrap_err()));
                }
                let mut head = head.unwrap();

                let space = head.inner_alloc(alloc_size);
                if space.is_err() {
                    return Err(AllocError::InternalError(space.unwrap_err()));
                }
                blocks.head = Some(head);

                space.unwrap()
            }
        } as CstPtr;

        Ok(space)
    }
}

impl<H: AllocHeader> AllocRaw for Immix<H> {
    type Header = H;

    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, AllocError>
    where
        T: AllocObject<<Self::Header as AllocHeader>::TypeId>,
    {
        let header_size = size_of::<Self::Header>();
        let object_size = size_of::<T>();
        let total_size = header_size + object_size;

        let alloc_size = alloc_size_of(total_size);
        let size_class = SizeClass::from(alloc_size);
        if size_class.is_err() {
            return Err(AllocError::SizeTooBig);
        }
        let size_class = size_class.unwrap();

        // attempt to allocate space for header and the object
        let space = self.find_space(alloc_size, size_class)?;

        // instantiate an object header for T, setting the mark bit to allocated
        let header = Self::Header::new::<T>(object_size as ArraySize, size_class, Mark::Allocated);

        // write the header into the front of the allocated space
        unsafe {
            write(space as *mut Self::Header, header);
        }

        // write the object to space after the header
        let object_space = unsafe { space.offset(header_size as isize) };

        unsafe {
            write(object_space as *mut T, object);
        }

        Ok(RawPtr::new(object_space as *const T))
    }

    fn alloc_array(&self, size_bytes: ArraySize) -> Result<RawPtr<u8>, AllocError> {
        let header_size = size_of::<Self::Header>();
        let total_size = header_size + size_bytes;

        let alloc_size = alloc_size_of(total_size);
        let size_class = SizeClass::from(alloc_size);
        if size_class.is_err() {
            return Err(AllocError::SizeTooBig);
        }
        let size_class = size_class.unwrap();

        // attempt to allocate space for header and the object
        let space = self.find_space(alloc_size, size_class)?;

        let header = Self::Header::new_array(size_bytes, size_class, Mark::Allocated);

        unsafe {
            write(space as *mut Self::Header, header);
        }

        let array_space = unsafe { space.offset(header_size as isize) };

        let array = unsafe { from_raw_parts_mut(array_space as *mut u8, size_bytes as usize) };

        for byte in array {
            *byte = 0;
        }

        Ok(RawPtr::new(array_space as *const u8))
    }

    fn get_header(object: NonNull<()>) -> NonNull<Self::Header> {
        // get header by subtract header size from the object pointer
        unsafe { NonNull::new_unchecked(object.cast::<Self::Header>().as_ptr().offset(-1)) }
    }

    fn get_object(header: NonNull<Self::Header>) -> NonNull<()> {
        // get object by add header size from the header pointer
        unsafe { NonNull::new_unchecked(header.as_ptr().offset(1).cast::<()>()) }
    }
}
