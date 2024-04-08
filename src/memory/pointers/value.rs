use crate::object::Int;

use super::{fat_ptr::FatPtr, ScopedPtr};

pub enum Value<'guard> {
    Int(ScopedPtr<'guard, Int>),
}
