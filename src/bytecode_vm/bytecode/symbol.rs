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
    outer: Option<Box<SymbolTable>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            len: 0,
            outer: None,
        }
    }

    pub fn enclose(outer: Self) -> Self {
        Self {
            table: HashMap::new(),
            len: 0,
            outer: Some(Box::new(outer)),
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

    pub fn define(&mut self, name: &String) -> usize {
        let scope = if self.outer.is_none() {
            Scope::Global
        } else {
            Scope::Local
        };
        self.table.insert(
            name.clone(),
            Symbol {
                name: name.clone(),
                scope,
                index: self.len,
            },
        );
        let len = self.len;

        self.len = self.table.len();

        len
    }

    pub fn resolve(&self, name: &String) -> Option<&Symbol> {
        let rst = self.table.get(name);
        if rst.is_none() {
            if self.outer.is_some() {
                return self.outer.as_ref().unwrap().resolve(name);
            }
        }
        rst
    }
}
