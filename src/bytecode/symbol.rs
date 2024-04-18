use std::collections::HashMap;

pub enum Scope {
    Global,
    Local,
}

pub struct Symbol {
    name: String,
    scope: Scope,
    pub index: usize,
}

pub struct SymbolTable {
    table: HashMap<String, Symbol>,
    len: usize,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            len: 0,
        }
    }

    pub fn define_global(&mut self, name: &String) -> usize {
        self.table.insert(
            name.clone(),
            Symbol {
                name: name.clone(),
                scope: Scope::Global,
                index: self.len,
            },
        );
        let len = self.len;

        self.len = self.table.len();

        len
    }

    pub fn get_global(&self, name: &String) -> Option<&Symbol> {
        self.table.get(name)
    }
}
