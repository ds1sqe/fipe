//! The runtime's value, shared by every backend.
//!
//! The VM interprets it; the native and LLVM backends link the same
//! runtime. Immutable, shared by reference, sixteen bytes. Operations
//! arrive in 08.2; this module is data.
use crate::{ir, token::Magnitude};
use alloc::{rc::Rc, string::String, vec::Vec};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    /// An `Integer` that fits; the common case.
    Int(i64),
    /// An `Integer` that does not fit `i64`; never one that does.
    Big(Rc<BigInt>),
    /// Always finite.
    F32(f32),
    /// Always finite.
    F64(f64),
    Text(Rc<String>),
    Bytes(Rc<Vec<u8>>),
    /// A `Vec<T>` or an array.
    Seq(Rc<Vec<Self>>),
    Tuple(Rc<Vec<Self>>),
    /// Every sum and product, `Bool` and `Maybe` included.
    Record(Rc<Record>),
    Function(Rc<Closure>),
    /// A hidden meaning: the witness a generic body dispatches on.
    Meaning(Rc<ir::Type>),
}

/// A constructed value of a meaning, by constructor index.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub meaning: usize,
    pub constructor: usize,
    pub fields: Vec<Value>,
}

/// A body with what it captured; `filled` is the prefix of spots a
/// run-time partial application supplied.
#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    pub body: usize,
    pub captures: Vec<Value>,
    pub meanings: Vec<ir::Type>,
    pub filled: Vec<Value>,
}

/// A signed integer of any size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    pub negative: bool,
    pub magnitude: Magnitude,
}
