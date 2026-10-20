//! The runtime's value, shared by every backend.
//!
//! The VM interprets it; the native and LLVM backends link the same
//! runtime. Immutable, shared by reference, sixteen bytes. The
//! operations live in the submodules: `arith` on integers, `float` on
//! the two float widths, `text` on text, `structural` on any value by
//! its tag. Each returns a [`Reason`] where the language Faults.
pub mod arith;
pub mod float;
pub mod structural;
pub mod text;

use crate::{ir, token::Magnitude};
use alloc::{rc::Rc, string::String, vec::Vec};
use core::fmt;

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
    /// A `Vec<T>`, a `Deque<T>` or an array; `Vec<U8>` is one of `Int`s.
    Seq(Rc<Vec<Self>>),
    Tuple(Rc<Vec<Self>>),
    /// Every sum and product, `Bool` and `Maybe` included.
    Record(Rc<Record>),
    Function(Rc<Closure>),
    /// A hidden meaning: the witness a generic body dispatches on.
    Meaning(Rc<ir::Type>),
    /// A value of a packed type with the meaning index of its tag.
    Packed(Rc<Packed>),
}

impl Value {
    /// An integer from its sign and magnitude: `Int` when it fits.
    #[must_use]
    pub fn integer(negative: bool, magnitude: Magnitude) -> Self {
        if let Some(small) = magnitude.to_u64() {
            if let Ok(value) = i64::try_from(small) {
                return Self::Int(if negative { -value } else { value });
            }
            if negative && small == 1 << 63 {
                return Self::Int(i64::MIN);
            }
        }
        Self::Big(Rc::new(BigInt {
            negative,
            magnitude,
        }))
    }

    #[must_use]
    pub fn text(text: &str) -> Self {
        Self::Text(Rc::new(String::from(text)))
    }

    #[must_use]
    pub fn seq(parts: Vec<Self>) -> Self {
        Self::Seq(Rc::new(parts))
    }

    /// A `Vec<U8>`: one `Int` per byte.
    #[must_use]
    pub fn bytes(bytes: &[u8]) -> Self {
        Self::seq(
            bytes
                .iter()
                .map(|byte| Self::Int(i64::from(*byte)))
                .collect(),
        )
    }
}

/// A packed value: the meaning it was packed as, and the value.
#[derive(Clone, Debug, PartialEq)]
pub struct Packed {
    pub meaning: usize,
    pub value: Value,
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

/// Why a run ends: the runtime's reasons, shared by every backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Budget,
    DivideByZero,
    NotFinite,
    /// A number the target cannot hold exactly: the value in the
    /// language's spelling and the target's name.
    NotExact {
        value: String,
        target: String,
    },
    /// An entry of the plan refuted at run time.
    Failed {
        entry: usize,
        /// The refuted value, in the language's spelling.
        value: String,
    },
    Cast,
    /// A block coverage excluded was reached: a crate defect.
    Unreachable,
    Host(String),
    /// The language's `fault`.
    Message(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget => f.write_str("the budget is exhausted"),
            Self::DivideByZero => f.write_str("division by zero"),
            Self::NotFinite => f.write_str("the result is not finite"),
            Self::NotExact { value, target } => {
                write!(f, "{value} is not an exact {target}")
            }
            Self::Failed { entry, value } => {
                write!(f, "entry {entry} refutes {value}")
            }
            Self::Cast => f.write_str("the cast failed"),
            Self::Unreachable => f.write_str("an excluded block was reached"),
            Self::Host(message) => write!(f, "host: {message}"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

/// The prelude's meanings the runtime constructs, by index, found
/// once per tables by label in the prelude's namespace. A meaning the
/// unit never named is `None`; constructing it is a crate defect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prelude {
    pub bool: Option<usize>,
    pub ordering: Option<usize>,
    pub maybe: Option<usize>,
    pub result: Option<usize>,
    pub either: Option<usize>,
    pub parse_error: Option<usize>,
    pub utf8_error: Option<usize>,
    pub count: Option<usize>,
    pub form: Option<usize>,
    pub integer: Option<usize>,
    pub f32: Option<usize>,
    pub f64: Option<usize>,
    pub string: Option<usize>,
    pub vec: Option<usize>,
    pub deque: Option<usize>,
    pub limit_error: Option<usize>,
}

impl Prelude {
    #[must_use]
    pub fn new(tables: &ir::Tables) -> Self {
        let prelude = tables
            .namespaces
            .iter()
            .position(|label| label == "prelude");
        let find = |label: &str| {
            tables.meanings.iter().position(|meaning| {
                Some(meaning.namespace) == prelude && meaning.label == label
            })
        };
        Self {
            bool: find("Bool"),
            ordering: find("Ordering"),
            maybe: find("Maybe"),
            result: find("Result"),
            either: find("Either"),
            parse_error: find("ParseError"),
            utf8_error: find("Utf8Error"),
            count: find("Count"),
            form: find("Form"),
            integer: find("Integer"),
            f32: find("F32"),
            f64: find("F64"),
            string: find("String"),
            vec: find("Vec"),
            deque: find("Deque"),
            limit_error: find("LimitError"),
        }
    }

    fn record(
        meaning: Option<usize>,
        constructor: usize,
        fields: Vec<Value>,
    ) -> Value {
        let meaning =
            meaning.expect("the unit names the meaning it constructs");
        Value::Record(Rc::new(Record {
            meaning,
            constructor,
            fields,
        }))
    }

    /// `True` is constructor 0, `False` 1.
    #[must_use]
    pub fn bool(&self, value: bool) -> Value {
        Self::record(self.bool, usize::from(!value), Vec::new())
    }

    /// `Less` 0, `Equal` 1, `Greater` 2.
    #[must_use]
    pub fn ordering(&self, ordering: core::cmp::Ordering) -> Value {
        let constructor = match ordering {
            core::cmp::Ordering::Less => 0,
            core::cmp::Ordering::Equal => 1,
            core::cmp::Ordering::Greater => 2,
        };
        Self::record(self.ordering, constructor, Vec::new())
    }

    #[must_use]
    pub fn just(&self, value: Value) -> Value {
        Self::record(self.maybe, 0, alloc::vec![value])
    }

    #[must_use]
    pub fn nothing(&self) -> Value {
        Self::record(self.maybe, 1, Vec::new())
    }

    #[must_use]
    pub fn ok(&self, value: Value) -> Value {
        Self::record(self.result, 0, alloc::vec![value])
    }

    #[must_use]
    pub fn err(&self, error: Value) -> Value {
        Self::record(self.result, 1, alloc::vec![error])
    }

    #[must_use]
    pub fn left(&self, value: Value) -> Value {
        Self::record(self.either, 0, alloc::vec![value])
    }

    #[must_use]
    pub fn right(&self, value: Value) -> Value {
        Self::record(self.either, 1, alloc::vec![value])
    }

    /// `Err (ParseError message)`.
    #[must_use]
    pub fn parse_error(&self, message: &str) -> Value {
        let error = Self::record(
            self.parse_error,
            0,
            alloc::vec![Value::text(message)],
        );
        self.err(error)
    }

    /// `Err (Utf8Error at)`.
    #[must_use]
    pub fn utf8_error(&self, at: usize) -> Value {
        let at = i64::try_from(at).unwrap_or(i64::MAX);
        let error =
            Self::record(self.utf8_error, 0, alloc::vec![Value::Int(at)]);
        self.err(error)
    }

    #[must_use]
    pub fn count(&self, item: Value, n: usize) -> Value {
        let n = i64::try_from(n).unwrap_or(i64::MAX);
        Self::record(self.count, 0, alloc::vec![item, Value::Int(n)])
    }

    /// `Err LimitError`, the failure of a checked conversion.
    #[must_use]
    pub fn limit_error(&self) -> Value {
        self.err(Self::record(self.limit_error, 0, Vec::new()))
    }

    /// Whether a value is the record `True`.
    #[must_use]
    pub fn is_true(&self, value: &Value) -> bool {
        matches!(value, Value::Record(record) if Some(record.meaning) == self.bool && record.constructor == 0)
    }
}
