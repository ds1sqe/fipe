pub mod environment;
pub mod types;

use std::{cell::RefCell, fmt::Debug, rc::Weak};

use crate::ast::{BlockStatement, Identifier, Nodetrait};

use self::environment::Environment;

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Return(Return),
    Int(Int),
    Bool(Bool),
    String(StringObject),
    Function(Function),
    Array(Array),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectType {
    Return,
    Int,
    Bool,
    String,
    Function,
    Array,
}

pub trait ObjectTrait {
    fn get_type(&self) -> ObjectType;
    fn to_str(&self) -> String;
}

impl ObjectTrait for Object {
    fn get_type(&self) -> ObjectType {
        match self {
            Object::Return(x) => x.get_type(),
            Object::Int(x) => x.get_type(),
            Object::Bool(x) => x.get_type(),
            Object::Function(x) => x.get_type(),
            Object::String(x) => x.get_type(),
            Object::Array(x) => x.get_type(),
        }
    }

    fn to_str(&self) -> String {
        let inner = match self {
            Object::Return(x) => x.to_str(),
            Object::Int(x) => x.to_str(),
            Object::Bool(x) => x.to_str(),
            Object::String(x) => x.to_str(),
            Object::Function(x) => x.to_str(),
            Object::Array(x) => x.to_str(),
        };
        inner
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Return {
    pub value: Option<Box<Object>>,
}

impl ObjectTrait for Return {
    fn get_type(&self) -> ObjectType {
        ObjectType::Return
    }

    fn to_str(&self) -> String {
        format!("{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct Int {
    pub value: i64,
}
impl ObjectTrait for Int {
    fn get_type(&self) -> ObjectType {
        ObjectType::Int
    }
    fn to_str(&self) -> String {
        format!("Int:{}", self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct Bool {
    pub value: bool,
}
impl ObjectTrait for Bool {
    fn get_type(&self) -> ObjectType {
        ObjectType::Bool
    }
    fn to_str(&self) -> String {
        format!("Bool:{}", self.value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StringObject {
    pub value: String,
}
impl ObjectTrait for StringObject {
    fn get_type(&self) -> ObjectType {
        ObjectType::String
    }
    fn to_str(&self) -> String {
        format!("String:{}", self.value)
    }
}

#[derive(Debug, Clone)]
pub struct Function {
    pub identifier: Option<String>,
    pub args: Vec<Identifier>,
    pub block: BlockStatement,
    pub env: Weak<RefCell<Environment<String>>>,
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier && self.args == other.args && self.block == other.block
    }
}

impl ObjectTrait for Function {
    fn get_type(&self) -> ObjectType {
        ObjectType::Function
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        if self.identifier.is_some() {
            buf += &format!("fn {}", &self.identifier.clone().unwrap());
        }
        let mut arguments = Vec::new();
        for arg in self.args.clone() {
            arguments.push(arg.to_str())
        }
        buf += "(";
        buf += &arguments.join(", ");
        buf += ") {\n";
        buf += &format!("{}", self.block.to_str());
        buf += "}";

        buf
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    pub elements: Vec<Object>,
}

impl ObjectTrait for Array {
    fn get_type(&self) -> ObjectType {
        ObjectType::Array
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();

        let mut elements_buf = Vec::new();
        for obj in self.elements.iter() {
            elements_buf.push(obj.to_str())
        }
        buf += "[";
        buf += &elements_buf.join(", ");
        buf += "]";

        buf
    }
}

pub fn is_same_type(left: &Object, right: &Object) -> bool {
    left.get_type() == right.get_type()
}
