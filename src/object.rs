pub mod environment;

use std::fmt::Debug;

use crate::{
    ast::{BlockStatement, Identifier, Nodetrait},
    bytecode_vm::bytecode::instructions::Instructions,
    utils::add_pad,
};

use self::environment::Environment;

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Return(Return),
    Int(Int),
    Bool(Bool),
    String(StringObject),
    Function(Function),
    CompiledFunction(CompiledFunction),
    Array(Array),
    Closure(ClosureFunction),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectType {
    Return,
    Int,
    Bool,
    String,
    Function,
    CompiledFunction,
    Array,
    Closure,
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
            Object::CompiledFunction(x) => x.get_type(),
            Object::Closure(x) => x.get_type(),
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
            Object::CompiledFunction(x) => x.to_str(),
            Object::Closure(x) => x.to_str(),
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
    pub env: Environment<String>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFunction {
    pub instructions: Instructions,
    pub local_len: usize,
    pub arg_len: usize,
}

impl ObjectTrait for CompiledFunction {
    fn get_type(&self) -> ObjectType {
        ObjectType::CompiledFunction
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf += "CompiledFunction\n";
        buf += &format!(
            ">\tlocal_len={}, arg_len={}\n",
            self.local_len, self.arg_len
        );
        buf += ">\tFuntion's Instructions\n";
        buf += &add_pad(&self.instructions.to_string(), ">\t");
        buf += "\n";

        buf
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureFunction {
    pub fun: CompiledFunction,
    pub free: Vec<Object>,
}

impl ObjectTrait for ClosureFunction {
    fn get_type(&self) -> ObjectType {
        ObjectType::CompiledFunction
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf += "Closure\n";
        buf += ">Free variables\n";
        for (idx, obj) in self.free.iter().enumerate() {
            buf += &format!("{idx}->{}\n", &obj.to_str());
        }
        buf += ">Inner Function\n";
        buf += &self.fun.to_str();

        buf
    }
}

pub fn is_same_type(left: &Object, right: &Object) -> bool {
    left.get_type() == right.get_type()
}
