use std::mem::replace;

use crate::alloc::errors::AllocError;

use super::bump::BumpBlock;

pub struct BlockList {
    pub head: Option<BumpBlock>,
    pub overflow: Option<BumpBlock>,
    pub rest: Vec<BumpBlock>,
}

impl BlockList {
    pub fn new() -> BlockList {
        BlockList {
            head: None,
            overflow: None,
            rest: Vec::new(),
        }
    }

    pub fn overflow_alloc(&mut self, alloc_size: usize) -> Result<*const u8, AllocError> {
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
