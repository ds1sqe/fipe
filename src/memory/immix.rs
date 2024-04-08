use std::{cell::UnsafeCell, marker::PhantomData};

use std::mem::replace;

use super::block::{BumpBlock, RawPtr};
use super::errors::AllocError;
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

    fn find_space(&self, alloc_size: usize, size_class: SizeClass) -> Result<RawPtr, AllocError> {
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
        } as RawPtr;

        Ok(space)
    }
}
