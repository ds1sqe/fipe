use std::{cell::RefCell, collections::HashMap, fmt::Debug, hash::Hash, rc::Rc};

use super::Object;

pub type Environ<T> = Rc<RefCell<Environment<T>>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Environment<T: Hash + Eq + PartialEq + Debug> {
    binding: HashMap<T, Object>,
    outer: Option<Environ<T>>,
    // 0 for global, the bigger is the outter
    level: usize,
}

impl<T: Hash + Eq + PartialEq + Debug> Environment<T> {
    // get object clone from environment
    pub fn get_clone(&self, key: &T) -> Option<Object> {
        // first, try get object from current scope
        let rst = self.binding.get(key);

        if rst.is_none() && self.outer.is_some() {
            // if not found, try get object from outer scope
            return self.outer.as_ref().unwrap().borrow().get_clone(&key);
        }
        rst.cloned()
    }

    // set object to environment
    pub fn set(&mut self, key: T, obj: Object) -> Option<Object> {
        self.binding.insert(key, obj)
    }

    pub fn new() -> Self {
        Environment {
            binding: HashMap::new(),
            outer: None,
            level: 0,
        }
    }

    pub fn new_inner(outer: &Environ<T>) -> Self {
        Environment {
            binding: HashMap::new(),
            level: outer.borrow().level + 1,
            outer: Some(Rc::clone(outer)),
        }
    }
}

impl<T> Drop for Environment<T>
where
    T: Hash + Eq + PartialEq + Debug,
{
    fn drop(&mut self) {
        dbg!("Droping..");
        dbg!(&self);
    }
}
