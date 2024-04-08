use crate::alloc::errors::AllocError;

pub enum MemoryError {
    AllocError(AllocError),
}

impl From<AllocError> for MemoryError {
    fn from(value: AllocError) -> Self {
        Self::AllocError(value)
    }
}
