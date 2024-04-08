use crate::{
    alloc::{immix::Immix, ptr::RawPtr, AllocObject, AllocRaw},
    object::{header::ObjectHeader, types::ObjectType},
};

use super::{
    errors::MemoryError,
    pointers::{fat_ptr::FatPtr, tagged_ptr::TaggedPtr, Mutatorscope, ScopedPtr, ScopedRef},
    symbol::SymbolMap,
    Memory,
};

type HeapStorage = Immix<ObjectHeader>;

pub struct Storage {
    heap: HeapStorage,
    syms: SymbolMap,
}

impl Storage {
    fn alloc<T>(&self, object: T) -> Result<RawPtr<T>, MemoryError>
    where
        T: AllocObject<ObjectType>,
    {
        Ok(self.heap.alloc(object)?)
    }
    fn alloc_tagged<T>(&self, object: T) -> Result<TaggedPtr, MemoryError>
    where
        FatPtr: From<RawPtr<T>>,
        T: AllocObject<ObjectType>,
    {
        Ok(TaggedPtr::from(FatPtr::from(self.heap.alloc(object)?)))
    }
}

pub struct MutatorView<'memory> {
    heap: &'memory Storage,
}
impl<'memory> Mutatorscope for MutatorView<'memory> {}

impl<'memory> MutatorView<'memory> {
    pub fn new(mem: &'memory Memory) -> MutatorView<'memory> {
        MutatorView { heap: &mem.heap }
    }
    pub fn alloc<T>(&self, object: T) -> Result<ScopedPtr<'_, T>, MemoryError>
    where
        T: AllocObject<ObjectType>,
    {
        Ok(ScopedPtr::new(
            self,
            self.heap.alloc(object)?.scoped_ref(self),
        ))
    }
}

pub trait Mutator: Sized {
    type Input;
    type Output;

    fn run(&self, mem: &MutatorView, input: Self::Input) -> Result<Self::Output, MemoryError>;
}
