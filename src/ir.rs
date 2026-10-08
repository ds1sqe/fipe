//! Checked, indexed input to a backend. Names here are display labels;
//! every executable reference is a table index or a value slot.
use crate::token::{Magnitude, Span};
use alloc::{boxed::Box, rc::Rc, string::String, vec::Vec};
use core::cell::{Ref, RefCell};

/// One compilation unit. Session units share append-only indexed tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub(crate) tables: Rc<RefCell<Tables>>,
    pub parameters: Vec<usize>,
    pub main: Vec<usize>,
    pub plan: Vec<Entry>,
}

impl Unit {
    /// Read the tables, including meaning slots filled by later session lines.
    #[must_use]
    pub fn tables(&self) -> Ref<'_, Tables> {
        self.tables.borrow()
    }
}

/// Indexed data consumed by a backend without source trees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tables {
    pub namespaces: Vec<String>,
    pub keys: Vec<String>,
    pub meanings: Vec<Meaning>,
    pub dispatch: Vec<Dispatch>,
    pub definitions: Vec<Definition>,
    pub declarations: Vec<Declaration>,
    pub modules: Vec<Module>,
    pub tests: Vec<Test>,
    /// Shared entry identities used by Enter nodes across session units.
    pub entries: Vec<Entry>,
    pub nodes: Vec<Node>,
    pub slots: Vec<Slot>,
    /// None is an unfixed session meaning, distinct from a generic parameter.
    pub session_meanings: Vec<Option<Type>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub ty: Type,
    pub parameters: Vec<usize>,
    pub hidden: Vec<usize>,
    pub definition: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Unit,
    Parameter(usize),
    Session(usize),
    Number(Magnitude),
    Meaning {
        index: usize,
        arguments: Vec<Self>,
    },
    Tuple(Vec<Self>),
    Array {
        element: Box<Self>,
        length: Box<Self>,
    },
    Function {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    Packed(Vec<Self>),
    All(Vec<Self>),
    Not(Box<Self>),
    Sum(Vec<Self>),
}

impl Type {
    /// This type with each of `parameters` replaced by the argument at
    /// its position; a parameter without an argument stays.
    #[must_use]
    pub fn substituted(
        &self,
        parameters: &[usize],
        arguments: &[Self],
    ) -> Self {
        let each = |parts: &[Self]| {
            parts
                .iter()
                .map(|part| part.substituted(parameters, arguments))
                .collect()
        };
        let one =
            |part: &Self| Box::new(part.substituted(parameters, arguments));
        match self {
            Self::Parameter(parameter) => parameters
                .iter()
                .position(|known| known == parameter)
                .and_then(|position| arguments.get(position))
                .map_or_else(|| self.clone(), Clone::clone),
            Self::Unit | Self::Session(_) | Self::Number(_) => self.clone(),
            Self::Meaning {
                index,
                arguments: inner,
            } => Self::Meaning {
                index: *index,
                arguments: each(inner),
            },
            Self::Tuple(parts) => Self::Tuple(each(parts)),
            Self::Packed(parts) => Self::Packed(each(parts)),
            Self::All(parts) => Self::All(each(parts)),
            Self::Sum(parts) => Self::Sum(each(parts)),
            Self::Array { element, length } => Self::Array {
                element: one(element),
                length: one(length),
            },
            Self::Function { parameter, result } => Self::Function {
                parameter: one(parameter),
                result: one(result),
            },
            Self::Not(inner) => Self::Not(one(inner)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meaning {
    pub label: String,
    pub namespace: usize,
    pub parameters: Vec<usize>,
    pub constructors: Vec<Constructor>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constructor {
    pub label: String,
    pub fields: Vec<Field>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub ty: Type,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub namespace: usize,
    pub label: String,
    pub span: Span,
    pub parameters: Vec<usize>,
    pub hidden: Vec<usize>,
    pub ty: Type,
    pub body: usize,
    pub slot: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub namespace: usize,
    pub index: usize,
    pub label: String,
    pub ty: Type,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub label: String,
    pub lines: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    pub parameters: Vec<usize>,
    pub namespace: usize,
    pub span: Span,
    pub body: usize,
    pub self_slot: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<Projection>,
    pub span: Span,
    pub namespace: usize,
    pub label: String,
    pub predicate: Predicate,
    pub phase: Phase,
    pub frequency: Frequency,
    pub reason: Reason,
}
/// A component reached from the value at an entry site.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Projection {
    Field(usize),
    /// Visit the field only when the value has this constructor.
    CaseField {
        constructor: usize,
        field: usize,
    },
    Part(usize),
    Element,
    /// Good/left payload is zero; the bad/right payload is one.
    Payload(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Compile,
    Init,
    Eval,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    Once,
    PerCall,
    PerElement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    UnknownInput,
    NoRule,
    CompileTimeEvaluation,
    Narrowing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub namespace: usize,
    pub span: Span,
    pub ty: Type,
    pub kind: Kind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Timing {
        phase: Phase,
        value: usize,
    },
    /// Callable adapter for a statically selected explicit conversion.
    Convert {
        callee: Option<usize>,
        mode: ConversionMode,
        from: Type,
        to: Type,
        parameter: Predicate,
        target: Predicate,
    },
    Literal {
        form: Form,
        reader: Option<usize>,
    },
    Local {
        slot: usize,
        meanings: Vec<Type>,
    },
    Capture {
        slot: usize,
        meanings: Vec<Type>,
    },
    Definition {
        index: usize,
        meanings: Vec<Type>,
    },
    Declared(usize),
    Derived {
        kind: Derived,
        ty: Type,
    },
    Construct {
        meaning: usize,
        constructor: usize,
        fields: Vec<usize>,
    },
    Constructor {
        meaning: usize,
        constructor: usize,
        /// Physical field index for each callable spot.
        fields: Vec<usize>,
    },
    Ordering {
        value: usize,
        accepted: Vec<usize>,
    },
    Field {
        value: usize,
        index: usize,
    },
    Tuple(Vec<usize>),
    Array(Vec<usize>),
    Repeat {
        value: usize,
        length: Type,
    },
    Dispatch {
        receiver: Receiver,
        member: usize,
    },
    Call {
        callee: usize,
        slots: Vec<usize>,
        supplies: Vec<Supply>,
        defaults: Vec<(usize, usize)>,
        open: Vec<usize>,
    },
    Function {
        parameters: Vec<usize>,
        hidden: Vec<usize>,
        captures: Vec<usize>,
        arms: Vec<Arm>,
    },
    If {
        condition: usize,
        then_body: usize,
        else_body: usize,
    },
    Block(Vec<usize>),
    Bind {
        pattern: Pattern,
        value: usize,
    },
    With {
        key: usize,
        value: usize,
        body: usize,
    },
    Lift {
        pattern: Pattern,
        value: usize,
        body: usize,
        rebuild: Rebuild,
    },
    Pipe {
        left: usize,
        stage: usize,
    },
    Marked {
        good: bool,
        left: usize,
        stage: usize,
        rebuild: Rebuild,
    },
    Or {
        mode: Mode,
        left: usize,
        right: usize,
    },
    And {
        mode: Mode,
        rebuild: Option<Rebuild>,
        left: usize,
        right: usize,
    },
    Cast {
        operation: Option<usize>,
        row: CastRow,
        from: Type,
        to: Type,
        value: usize,
    },
    Pack {
        tag: Type,
        value: usize,
    },
    Enter {
        entry: usize,
        value: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supply {
    pub spot: usize,
    pub value: usize,
    pub element: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub span: Span,
    pub patterns: Vec<Pattern>,
    pub guard: Option<usize>,
    pub body: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub span: Span,
    pub kind: PatternKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Any,
    Slot(usize),
    Literal {
        value: usize,
        equality: usize,
        self_spot: usize,
    },
    Constructor {
        meaning: usize,
        constructor: usize,
        parts: Vec<Pattern>,
    },
    Record {
        meaning: usize,
        constructor: usize,
        fields: Vec<(usize, Pattern)>,
    },
    Side {
        good: bool,
        payload: Option<Box<Pattern>>,
    },
    Gate {
        predicate: Predicate,
        payload: Box<Pattern>,
    },
    Tag {
        meaning: usize,
        payload: Box<Pattern>,
    },
    List(Vec<Pattern>),
    Tuple(Vec<Pattern>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Form {
    Unit,
    Whole {
        negative: bool,
        magnitude: Magnitude,
        radix: u8,
        digits: usize,
    },
    Fraction {
        negative: bool,
        digits: Magnitude,
        exponent_negative: bool,
        exponent: Magnitude,
    },
    Text(String),
    Bytes(Vec<u8>),
}
/// The rebuild selected by the checker, never rediscovered by a backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rebuild {
    Direct,
    Wrap,
    Discard,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Bool,
    Family,
    Payload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastRow {
    Number,
    Read,
    Show,
    Codec,
    Bytes,
}

/// A member's indexed runtime choices; labels are for diagnostics only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub label: String,
    pub rows: Vec<DispatchRow>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchRow {
    pub receiver: Type,
    pub receiver_spot: Option<usize>,
    pub argument: Option<Type>,
    pub target: Implementation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Implementation {
    Field(usize),
    Definition { index: usize, meanings: Vec<Type> },
    Declared(usize),
    Derived { kind: Derived, ty: Type },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Derived {
    Eq,
    Cmp,
    Show,
    Default,
}

/// A generic member reads a value tag or a supplied hidden meaning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receiver {
    Value(usize),
    Meaning(Type),
}

/// A gate's checked test composition, including inherited limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    Nonzero {
        when_integer: bool,
    },
    Function {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    Test {
        index: usize,
        meanings: Vec<Type>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionMode {
    Direct,
    Wrap,
    Unwrap,
    Check,
    TryCheck,
}
