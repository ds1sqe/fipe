use crate::alloc::AllocTypeId;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ObjectType {
    Return,
    Int,
    Bool,
    String,
    Function,
    Array,
}

impl AllocTypeId for ObjectType {}
