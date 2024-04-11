use std::{collections::HashMap, fmt::Debug, hash::Hash};

use crate::immix::ptr::TypedPtr;

use super::Object;

#[derive(Debug, Clone, PartialEq)]
pub struct Environment<T: Hash + Eq + PartialEq + Debug>
where
    T: Clone,
{
    binding: HashMap<T, TypedPtr<Object>>,
    outer: Option<Box<Environment<T>>>,
    // 0 for global, the bigger is the outter
    level: usize,
}

impl<T: Hash + Eq + PartialEq + Debug + Clone> Environment<T> {
    // get object clone from environment
    pub fn get_clone(&self, key: &T) -> Option<TypedPtr<Object>> {
        // first, try get object from current scope
        let rst = self.binding.get(key);

        if rst.is_none() && self.outer.is_some() {
            // if not found, try get object from outer scope
            return self.outer.as_ref().unwrap().get_clone(&key);
        }
        rst.cloned()
    }

    // set object to environment
    pub fn set(
        &mut self,
        key: T,
        obj: TypedPtr<Object>,
    ) -> Option<TypedPtr<Object>> {
        self.binding.insert(key, obj)
    }

    pub fn new() -> Self {
        Environment {
            binding: HashMap::new(),
            outer: None,
            level: 0,
        }
    }

    pub fn capture(tgt: &Environment<T>) -> Self {
        let level = tgt.level + 1;

        let outer = (*tgt).clone();

        Environment {
            binding: HashMap::new(),
            level,
            outer: Some(Box::new(outer)),
        }
    }
}

impl<T> Drop for Environment<T>
where
    T: Hash + Eq + PartialEq + Debug + Clone,
{
    fn drop(&mut self) {
        dbg!("Droping..");
        dbg!(&self);
    }
}
