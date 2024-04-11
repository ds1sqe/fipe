use std::marker::PhantomData;

use crate::{
    immix::{
        errors::ImmixError,
        ptr::{RawPtr, TypedPtr},
        Immix,
    },
    object::{environment::Environment, Object},
};

#[derive(Debug)]
pub struct Heap {
    obj_immix: Immix<usize, Object>,
    // env_immix: Immix<usize, Environment<String>>,
    count: usize,
}

impl Heap {
    pub fn new() -> Self {
        Self {
            count: 0,
            obj_immix: Immix::new(),
        }
    }

    /// allocate object into heap
    pub fn enlist(
        &mut self,
        env: &mut Environment<String>,
        id: String,
        obj: Object,
    ) -> Result<RawPtr<Object>, ImmixError> {
        let ptr = self.obj_immix.alloc(self.count, obj)?;
        self.count += 1;
        let typed: TypedPtr<Object> = TypedPtr {
            ptr,
            tag: PhantomData,
        };

        env.set(id, typed.clone());

        Ok(RawPtr::new(typed.ptr.data as *const Object))
    }

    pub fn get(
        env: &mut Environment<String>,
        key: String,
    ) -> Option<TypedPtr<Object>> {
        env.get_clone(&key)
    }

    /// mark and sweep
    pub fn run_gc(&mut self) {
        self.obj_immix.unmark_all();

        // mark all reachables
        // self.env.mark_all();

        // manage memory
        // self.immix.sweep()
    }
}
