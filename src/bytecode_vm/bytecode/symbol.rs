use std::collections::HashMap;

#[derive(PartialEq, Debug, Clone)]
pub enum Scope {
    Global,
    Local,
    Free,
    Function,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    name: String,
    scope: Scope,
    pub index: usize,
}

impl Symbol {
    #![allow(dead_code)]
    pub fn is_global(&self) -> bool {
        self.scope == Scope::Global
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

#[derive(Debug)]
pub struct SymbolTable {
    table: HashMap<String, Symbol>,
    // length of local symbol
    pub len: usize,
    outer: Option<Box<SymbolTable>>,
    free: Vec<Symbol>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
            len: 0,
            outer: None,
            free: Vec::new(),
        }
    }

    pub fn enclose(outer: Self) -> Self {
        Self {
            table: HashMap::new(),
            len: 0,
            outer: Some(Box::new(outer)),
            free: Vec::new(),
        }
    }

    pub fn get_outer(&mut self) -> Self {
        *self.outer.take().unwrap()
    }

    pub fn is_global(&self) -> bool {
        self.outer.is_none()
    }

    /// TODO: add duplicate checking rule
    pub fn define(&mut self, name: &str) -> usize {
        let scope = if self.is_global() {
            Scope::Global
        } else {
            Scope::Local
        };
        self.table.insert(
            name.to_string(),
            Symbol {
                name: name.to_string(),
                scope,
                index: self.len,
            },
        );
        let len = self.len;

        self.len += 1;

        len
    }

    fn define_free(&mut self, sym: &Symbol) -> Symbol {
        self.free.push(sym.clone());

        let symbol = Symbol {
            name: sym.name.clone(),
            scope: Scope::Free,
            index: self.free.len() - 1,
        };

        self.table.insert(sym.name.clone(), symbol.clone());
        symbol
    }

    pub fn define_function_name(&mut self, name: &str) -> Symbol {
        let func_sym = Symbol {
            name: name.to_string(),
            scope: Scope::Function,
            index: 0,
        };
        self.table.insert(name.to_string(), func_sym.clone());
        func_sym
    }

    pub fn get_free(&self) -> Vec<Symbol> {
        self.free.clone()
    }

    pub fn resolve(&mut self, name: &String) -> Option<Symbol> {
        if let Some(sym) = self.table.get(name) {
            return Some(sym.clone());
        } else if self.outer.is_some() {
            // find symbol in self.outer
            let outer_rst = self.outer.as_mut().unwrap().resolve(name);
            if outer_rst.is_some() {
                if outer_rst.as_ref().unwrap().scope == Scope::Global {
                    return outer_rst;
                } else {
                    // outer local symbol means it's a free variable
                    let free =
                        self.define_free(&outer_rst.as_ref().unwrap().clone());
                    return Some(free);
                }
            }
        }

        None
    }
}
