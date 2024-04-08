use self::{
    errors::MemoryError,
    storage::{Mutator, MutatorView, Storage},
};

pub mod errors;
pub mod pointers;
pub mod storage;
pub mod symbol;

pub struct Memory {
    heap: Storage,
}

impl Memory {
    pub fn mutate<M: Mutator>(&self, m: &M, input: M::Input) -> Result<M::Output, MemoryError> {
        let mut guard = MutatorView::new(self);
        m.run(&mut guard, input)
    }
}
