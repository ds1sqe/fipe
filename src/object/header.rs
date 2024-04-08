use crate::alloc::{size::SizeClass, AllocHeader, Mark};

use super::types::ObjectType;

pub struct ObjectHeader {
    mark: Mark,
    size_class: SizeClass,
    type_id: ObjectType,
    size_bytes: usize,
}

impl AllocHeader for ObjectHeader {
    type TypeId = ObjectType;

    fn new<O: crate::alloc::AllocObject<Self::TypeId>>(
        size: usize,
        size_class: SizeClass,
        mark: crate::alloc::Mark,
    ) -> Self {
        todo!()
    }

    fn new_array(
        size: crate::alloc::ArraySize,
        size_class: SizeClass,
        mark: crate::alloc::Mark,
    ) -> Self {
        todo!()
    }

    fn mark(&mut self) {
        todo!()
    }

    fn is_marked(&self) -> bool {
        todo!()
    }

    fn size_class(&self) -> SizeClass {
        todo!()
    }

    fn size(&self) -> usize {
        todo!()
    }

    fn type_id(&self) -> Self::TypeId {
        todo!()
    }
}
