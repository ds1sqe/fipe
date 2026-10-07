//! Owned syntax with flat, module-local arenas for recursive node references.

use crate::token::{Affixed, Magnitude, Span};
use alloc::{boxed::Box, string::String, vec::Vec};

/// A reference to an expression in its originating module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExprId(pub(crate) usize);

/// A reference to a pattern in its originating module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternId(pub(crate) usize);

/// A reference to type syntax in its originating module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeId(pub(crate) usize);

/// A complete syntactically valid file; no name or type has been resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub doc: Option<String>,
    /// Ordinary comment trivia retained for source tools.
    pub comments: Vec<Span>,
    pub span: Span,
    pub entries: Vec<Entry>,
    pub(crate) expressions: Vec<Expr>,
    pub(crate) patterns: Vec<Pattern>,
    pub(crate) types: Vec<Type>,
}

impl Module {
    #[must_use]
    pub fn expr(&self, id: ExprId) -> Option<&Expr> {
        self.expressions.get(id.0)
    }

    #[must_use]
    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.get(id.0)
    }

    #[must_use]
    pub fn ty(&self, id: TypeId) -> Option<&Type> {
        self.types.get(id.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Line(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub segments: Vec<String>,
    pub span: Span,
}

impl Name {
    /// The case rule: a name whose last segment starts with an ASCII
    /// uppercase letter is a constructor or type in the positions where the
    /// parser must tell the two apart.
    #[must_use]
    pub fn is_uppercase(&self) -> bool {
        self.segments
            .last()
            .and_then(|segment| segment.chars().next())
            .is_some_and(|first| first.is_ascii_uppercase())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Literal(Literal),
    Name(Name),
    Word(String),
    Path(String),
    Selector(Name),
    /// A parenthesized line sequence; `( )` alone is the unit literal.
    Group(Vec<ExprId>),
    /// `(a, b)`, two parts or more, not touching a callee.
    Tuple(Vec<ExprId>),
    List(Vec<ExprId>),
    /// `[:e]`, `[:]`, `[!e]` or `[!]`: a side of an outcome.
    Outcome {
        good: bool,
        payload: Option<ExprId>,
    },
    /// A name with explicit type arguments, such as `read<I64>` or `Rd<fs>`.
    Instance {
        name: Name,
        arguments: Vec<TypeId>,
    },
    /// Raw construction after an uppercase name: `Point { x, y: 2 }`.
    Record {
        name: Name,
        fields: Vec<FieldValue>,
    },
    Call {
        callee: ExprId,
        arguments: Vec<Argument>,
        style: CallStyle,
    },
    Field {
        value: ExprId,
        field: Name,
    },
    Not {
        operator: Span,
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        operator: Span,
        left: ExprId,
        right: ExprId,
    },
    Pipe {
        op: PipeOp,
        operator: Span,
        left: ExprId,
        right: ExprId,
    },
    /// `pattern = line`, optionally annotated; `pub` only at file level.
    Bind {
        doc: Option<String>,
        public: bool,
        pattern: PatternId,
        annotation: Option<TypeId>,
        value: ExprId,
    },
    With {
        key: Name,
        value: ExprId,
        body: ExprId,
    },
    If {
        condition: ExprId,
        then_body: ExprId,
        else_body: Option<ExprId>,
    },
    Block(Vec<ExprId>),
    Arms(Vec<Arm>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Literal {
    Unit,
    Int {
        negative: bool,
        magnitude: Magnitude,
    },
    Float(String),
    Str {
        value: String,
        raw: bool,
    },
    /// A number or quote with a prefix or suffix; `b"…"` is one.
    Affixed(Box<Affixed>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStyle {
    Space,
    Parenthesized,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argument {
    pub kind: ArgumentKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgumentKind {
    Positional(ExprId),
    Flag { name: String, value: Option<ExprId> },
}

/// A record field value; a punned field `{ x }` holds the name `x`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldValue {
    pub name: Name,
    pub value: ExprId,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    /// `++`, which spells `concat`.
    Concat,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeOp {
    Plain,
    Good,
    Bad,
    Or,
    And,
}

/// One arm: a head of one or more patterns, a guard and a body. A head with
/// several patterns is a curried function of that many parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub patterns: Vec<PatternId>,
    pub guard: Option<ExprId>,
    pub body: ExprId,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Name(Name),
    Literal(Literal),
    /// An uppercase name followed by one payload pattern.
    Constructor {
        name: Name,
        payload: PatternId,
    },
    /// `(name = operand)`: a parameter with a default, in an arm head.
    Default {
        name: Name,
        value: ExprId,
    },
    Group(PatternId),
    /// `(a, b)`, two parts or more.
    Tuple(Vec<PatternId>),
    List(Vec<PatternId>),
    Outcome {
        good: bool,
        payload: Option<PatternId>,
    },
    Record {
        name: Name,
        fields: Vec<PatternField>,
        rest: Option<Span>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternField {
    pub name: Name,
    pub pattern: Option<PatternId>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Unit,
    Named {
        name: Name,
        arguments: Vec<TypeId>,
    },
    /// `[T; n]`: the length is a [`TypeKind::Number`] or a lowercase
    /// name, a number parameter.
    Array {
        element: TypeId,
        length: TypeId,
    },
    /// A number as a type argument, `Bits<8>`, or an array's length.
    Number(Magnitude),
    /// `A * B * C`, flat, two parts or more.
    Tuple(Vec<TypeId>),
    /// `A & B`, flat: holds both.
    Both(Vec<TypeId>),
    /// `A | B`, flat: holds either.
    Either(Vec<TypeId>),
    /// `!A`: fails at least one of `A`'s tests.
    Not(TypeId),
    /// `A -> B`; `A -> B -> C` nests to the right.
    Function {
        parameter: TypeId,
        result: TypeId,
    },
    /// `Show a, b.age: c => type`: constraints at the top of an annotation.
    /// A prelude and declaration form.
    Qualified {
        constraints: Vec<Constraint>,
        ty: TypeId,
    },
    /// `(name: type = operand)`: a labeled parameter with an optional
    /// default, in a declaration's function type. A declaration form.
    Labeled {
        name: Name,
        ty: TypeId,
        default: Option<ExprId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub kind: ConstraintKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    /// `Class<args> variable`, such as `Show a` or `From<a> b`.
    Named { class: TypeId, variable: Name },
    /// `b.age: a`: a member limit on a variable.
    Member { member: Name, ty: TypeId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub doc: Option<String>,
    pub kind: ItemKind,
    pub public: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// `use x`, `use x.inc x.Point`, `use text 2`.
    Use {
        names: Vec<Name>,
        version: Option<Magnitude>,
    },
    /// `type Name<params>: base { members } where limits`, each part
    /// optional; with none, an opaque type, which is a declaration form.
    Type {
        name: Name,
        parameters: Vec<TypeParameter>,
        base: Option<TypeId>,
        members: Option<Vec<Member>>,
        limits: Vec<Limit>,
    },
    /// `impl Target [: Claim] { members }`.
    Impl {
        target: TypeId,
        claim: Option<TypeId>,
        members: Vec<ImplMember>,
    },
    /// `name: type` with no value: a signature-only declaration, the form
    /// of prelude primitives and host declarations.
    Declare { name: Name, ty: TypeId },
}

/// A member of a `type` body: a lowercase field or an uppercase constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Member {
    Field(Field),
    Constructor {
        doc: Option<String>,
        name: Name,
        fields: Vec<Field>,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub doc: Option<String>,
    pub public: bool,
    pub name: Name,
    pub ty: TypeId,
    pub default: Option<ExprId>,
    pub span: Span,
}

/// One limit after `where`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Limit {
    /// `self.area: F64`.
    Member { name: Name, ty: TypeId, span: Span },
    /// A Bool over `self`, a pipeline.
    Test(ExprId),
}

/// A named member `name = value`, or `name: type` with no value, an
/// bare arm of the written
/// constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImplMember {
    Named {
        doc: Option<String>,
        public: bool,
        name: Name,
        annotation: Option<TypeId>,
        /// `None` for a member with no value, a declaration form.
        value: Option<ExprId>,
        span: Span,
    },
    Constructor {
        doc: Option<String>,
        arm: Arm,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParameter {
    pub name: Name,
    /// `T: Ord`: a type the parameter must meet.
    pub bound: Option<TypeId>,
    /// Used when the corresponding type argument is omitted.
    pub default: Option<TypeId>,
    pub span: Span,
}
