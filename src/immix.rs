use std::{collections::HashMap, hash::Hash, marker::PhantomData, mem::size_of};

use self::{
    blocks::BlockList,
    errors::ImmixError,
    ptr::{PairPtr, RawPtr},
};

pub mod blocks;
pub mod errors;
pub mod mark;
pub mod ptr;
pub mod size;
/// Align up to double word boundary
pub fn alloc_size_of(object_size: usize) -> usize {
    // align will be 4 in 32bit machine
    //   and will be 8 in 64bit machine
    let align = size_of::<usize>();
    // align to double word boundary
    (object_size + (align - 1)) & !(align - 1)
}

#[derive(Debug)]
pub struct Immix<K, V>
where
    K: Hash + Eq,
{
    memory: BlockList,
    _memory_type: PhantomData<V>,
    entities: HashMap<K, PairPtr>,
}

impl<K, V> Immix<K, V>
where
    K: Hash + Eq,
{
    pub fn new() -> Self {
        Self {
            memory: BlockList::new(),
            _memory_type: PhantomData,
            entities: HashMap::new(),
        }
    }

    pub fn alloc(&mut self, key: K, val: V) -> Result<PairPtr, ImmixError> {
        if !self.entities.contains_key(&key) {
            let ptr = self.memory.alloc(size_of::<V>())?;

            // write inner memory with given val
            unsafe { std::ptr::write(ptr.data as *mut V, val) }
            self.entities.insert(key, ptr);

            Ok(ptr)
        } else {
            Err(ImmixError::DuplicatedKey)
        }
    }

    pub fn unmark_all(&mut self) {
        for (_, pair) in self.entities.iter_mut() {
            pair.set_mark(&mark::Mark::Unmarked);
        }
    }
}
