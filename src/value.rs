//! The runtime's value, shared by every backend.
//!
//! The VM interprets it; the native and LLVM backends link the same
//! runtime. Immutable, shared by reference, sixteen bytes. The
//! operations live in the submodules: `arith` on integers, `float` on
//! the two float widths, `text` on text, `structural` on any value by
//! its tag. Each returns a [`Reason`] where the language Faults.
pub mod arith;
pub mod float;
pub mod resource;
pub mod structural;
pub mod text;

use crate::{ir, token::Magnitude};
use alloc::{rc::Rc, string::String, vec::Vec};
use core::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    /// A genuine opaque Host reference, never script-constructible data.
    Resource(Rc<resource::ResourceRef>),
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
    Meaning(Rc<Meaning>),
    /// An immutable packed value with descriptive source and target types.
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

/// An owned hidden type witness, not a resource identity or authorization.
///
/// Cloning its `Rc` preserves aliases; the last owner drops the type iteratively.
pub struct Meaning {
    ty: ir::Type,
}
impl Meaning {
    /// Own an existing type tree as a hidden witness.
    #[must_use]
    pub const fn new(ty: ir::Type) -> Self {
        Self { ty }
    }
    /// Borrow the underlying type without cloning its tree.
    #[must_use]
    pub const fn as_type(&self) -> &ir::Type {
        &self.ty
    }
}
impl fmt::Debug for Meaning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.ty, f)
    }
}
impl Drop for Meaning {
    fn drop(&mut self) {
        let root = core::mem::replace(&mut self.ty, ir::Type::Unit);
        drop(resource::Forest(alloc::vec![root]));
    }
}

/// An immutable packed payload. Its types describe the captured source context;
/// they are not current-table identities or permission to use a resource.
#[derive(Clone)]
pub struct Packed {
    source: Rc<Meaning>,
    target: Rc<Meaning>,
    value: Value,
    association: Option<Rc<PackAssociation>>,
}
#[derive(Debug)]
pub(crate) struct PackAssociation {
    pub(crate) bindings: Vec<(usize, Rc<()>)>,
}
impl Packed {
    /// Describe resource-free data without asserting a checked membership claim.
    /// Returns `None` for a genuine resource anywhere in the payload, including
    /// closure captures and filled spots. The borrowed payload stays with its owner.
    #[must_use]
    pub fn from_data(
        source: ir::Type,
        target: ir::Type,
        value: &Value,
    ) -> Option<Self> {
        Self::data_roots(
            Rc::new(Meaning::new(source)),
            Rc::new(Meaning::new(target)),
            value,
        )
    }
    pub(crate) fn data_roots(
        source: Rc<Meaning>,
        target: Rc<Meaning>,
        value: &Value,
    ) -> Option<Self> {
        let free = resource::data_only(value, &mut |_| {
            Ok::<(), core::convert::Infallible>(())
        });
        if !free.unwrap_or_else(|never| match never {}) {
            return None;
        }
        Some(Self {
            source,
            target,
            value: value.clone(),
            association: None,
        })
    }
    /// The original payload type, using indices of its captured source context.
    /// This description alone is not a current-table identity or resource proof.
    #[must_use]
    pub fn source_type(&self) -> &ir::Type {
        self.source.as_type()
    }
    /// The packed contract, in the same captured index context as the source.
    #[must_use]
    pub fn target_type(&self) -> &ir::Type {
        self.target.as_type()
    }
    /// Borrow the exact immutable payload; no mutable view can alter its association.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }
    pub(crate) fn roots(&self) -> (Rc<Meaning>, Rc<Meaning>) {
        (Rc::clone(&self.source), Rc::clone(&self.target))
    }
    pub(crate) fn association(&self) -> Option<&PackAssociation> {
        self.association.as_deref()
    }
    pub(crate) fn checked(
        source: ir::Type,
        target: ir::Type,
        value: Value,
        bindings: Vec<(usize, Rc<()>)>,
    ) -> Self {
        Self {
            source: Rc::new(Meaning::new(source)),
            target: Rc::new(Meaning::new(target)),
            value,
            association: Some(Rc::new(PackAssociation { bindings })),
        }
    }
}
impl PartialEq for Packed {
    fn eq(&self, other: &Self) -> bool {
        resource::type_equal(self.source_type(), other.source_type())
            && self.value == other.value
    }
}
impl fmt::Debug for Packed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Metadata can be deeply nested; accessors expose it without a recursive debug walk.
        f.debug_struct("Packed")
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}
impl PartialEq for Meaning {
    fn eq(&self, other: &Self) -> bool {
        resource::type_equal(&self.ty, &other.ty)
    }
}
impl Eq for Meaning {}

/// A constructed value of a meaning, by constructor index.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub meaning: usize,
    pub constructor: usize,
    pub fields: Vec<Value>,
}

/// A body with what it captured; `filled` is the prefix of spots a
/// run-time partial application supplied.
///
/// Witnesses are dropped iteratively. Transfer owned fields with
/// `core::mem::take(&mut closure.field)` rather than moving out of this `Drop` type.
#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    pub body: usize,
    pub captures: Vec<Value>,
    pub meanings: Vec<ir::Type>,
    pub filled: Vec<Value>,
    /// The tests its calls run on their arguments before the body: an
    /// open function entry's parameter limits.
    pub guards: Vec<Guard>,
}

impl Drop for Closure {
    fn drop(&mut self) {
        drop(resource::Forest(core::mem::take(&mut self.meanings)));
    }
}

/// A spot of a closure tested on each call by a parameter limit of an
/// open entry, whose failure Faults naming that entry.
///
/// Drops its predicate tree iteratively. Because `Guard` implements `Drop`,
/// moving non-`Copy` fields out of an owned guard (including destructuring or
/// struct update) may fail with E0509. Borrow fields, or clone them when a
/// separate owned value is needed. The derived `Clone` recursively copies
/// predicate/type trees and is not charged against a VM budget; iterative
/// destruction does not make arbitrary deep cloning or execution stack-safe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guard {
    pub spot: usize,
    pub entry: usize,
    pub predicate: ir::Predicate,
    /// Creator-owned witnesses used when a guard reaches a limit test.
    pub witnesses: Rc<[(usize, Rc<Meaning>)]>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let predicate = core::mem::replace(
            &mut self.predicate,
            ir::Predicate::All(Vec::new()),
        );
        drop(resource::Predicates(alloc::vec![predicate]));
    }
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
