use crate::{alloc::ptr::RawPtr, object::Int};

use super::{value::Value, Mutatorscope, ScopedPtr, ScopedRef};

pub enum FatPtr {
    Int(RawPtr<Int>),
}

impl FatPtr {
    pub fn as_value<'guard>(&self, guard: &'guard dyn Mutatorscope) -> Value<'guard> {
        match self {
            FatPtr::Int(raw_ptr) => {
                return Value::Int(ScopedPtr::new(guard, raw_ptr.scoped_ref(guard)));
            }
        }
    }
}
