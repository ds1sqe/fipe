//! The natives: the prelude's operations, chosen once per tables by
//! label and, where a label is shared, by the declared type. One that
//! needs no call computes at once; one that calls back is a state in a
//! native frame, moved by [`Pending::step`].
use super::calls::Invoke;
use crate::{
    ir, mir,
    value::{Prelude, Reason, Value},
};
use alloc::vec::Vec;

/// What a native did at one move: the call it waits on, its value,
/// its Fault, or (from the first move only) the frame it becomes.
pub(super) enum Step {
    Call(Invoke),
    /// One private ordered argument guard, using its creator's witnesses.
    AdmitArgument {
        value: Value,
        guard: super::pending::GuardJob,
    },
    Done(Value),
    Fault(Reason),
    Start(super::pending::Pending),
}

/// What a native reads besides its arguments.
pub(super) struct Ctx<'a> {
    pub(super) program: &'a mir::Program,
    pub(super) tables: &'a ir::Tables,
    pub(super) prelude: &'a Prelude,
    pub(super) natives: &'a Natives,
    pub(super) globals: &'a [Option<Value>],
    /// The type of the call's destination, witnesses in place.
    pub(super) result: Option<&'a ir::Type>,
    pub(super) work: &'a super::resource::Work,
}

/// The meaning a shared label is declared for, where the value alone
/// cannot tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Of {
    Integer,
    F32,
    F64,
    Text,
    Bytes,
    Seq,
    Maybe,
    Result,
    Other,
}

/// One native, by what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Native {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Cmp,
    Show,
    Len,
    Not,
    Concat,
    Literal(Of),
    Read(Of),
    Zero(Of),
    Default(Of),
    Round,
    Trunc,
    Floor,
    Ceil,
    Sqrt,
    Sin,
    Cos,
    Atan2,
    Hypot,
    Map,
    Pure(Of),
    Then,
    Empty,
    Or,
    Fold,
    Filter,
    Sum,
    First,
    Head,
    Take,
    Fault,
    Log,
    Sequence,
    /// `to` and `try` written as function values (`map to<U8>`): the
    /// value passes, or passes in `Ok`; the target's limits ride on the
    /// function entry the checker attaches, checked at each call.
    To,
    Try,
    Bytes,
    Utf8,
    Lines,
    Words,
    Split,
    Join,
    Lower,
    Upper,
    Flatten,
    Collect,
    Count,
    Sort,
    Drop,
    Last,
    Nth,
    Reverse,
    Enumerate,
    Zip,
    Contains,
    StartsWith,
    EndsWith,
    Replace,
}

/// The dispatch members a walk by tag consults, by index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Members {
    pub(super) eq: Option<usize>,
    pub(super) cmp: Option<usize>,
    pub(super) show: Option<usize>,
    pub(super) default: Option<usize>,
    pub(super) add: Option<usize>,
    pub(super) zero: Option<usize>,
    pub(super) len: Option<usize>,
    pub(super) read: Option<usize>,
}

/// The natives of one tables, by declaration index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Natives {
    table: Vec<Option<Native>>,
    pub(super) members: Members,
}

impl Natives {
    /// The natives for every declaration of the prelude's namespace.
    ///
    /// # Panics
    /// On a prelude declaration no native answers, a crate defect.
    #[must_use]
    pub fn new(tables: &ir::Tables) -> Self {
        let prelude = tables
            .namespaces
            .iter()
            .position(|label| label == "prelude");
        let table = tables
            .declarations
            .iter()
            .map(|declaration| {
                (Some(declaration.namespace) == prelude)
                    .then(|| choose(declaration, tables))
            })
            .collect();
        let member = |label: &str| {
            tables
                .dispatch
                .iter()
                .position(|member| member.label == label)
        };
        Self {
            table,
            members: Members {
                eq: member("eq"),
                cmp: member("cmp"),
                show: member("show"),
                default: member("default"),
                add: member("add"),
                zero: member("zero"),
                len: member("len"),
                read: member("read"),
            },
        }
    }

    /// Whether every prelude declaration has a native.
    #[must_use]
    pub fn covers(&self, tables: &ir::Tables) -> bool {
        let prelude = tables
            .namespaces
            .iter()
            .position(|label| label == "prelude");
        tables.declarations.iter().zip(&self.table).all(
            |(declaration, native)| {
                (Some(declaration.namespace) == prelude) == native.is_some()
            },
        )
    }

    /// The native of a declaration; `None` for a host's.
    pub(super) fn get(&self, op: usize) -> Option<Native> {
        self.table.get(op).copied().flatten()
    }

    pub(super) fn of(&self, op: usize) -> Native {
        self.table[op].expect("a prelude declaration")
    }

    /// Run a native: its value at once, or the frame it starts as.
    ///
    /// # Errors
    /// The reason the native Faults.
    pub(super) fn call(
        &self,
        op: usize,
        args: Vec<Value>,
        ctx: &Ctx<'_>,
    ) -> Result<Step, Reason> {
        super::immediate::immediate(self.of(op), args, ctx)
    }
}

/// The meaning of the last result of a declared type, by label.
fn result_of(ty: &ir::Type, tables: &ir::Tables) -> Of {
    let mut ty = ty;
    while let ir::Type::Function { result, .. } = ty {
        ty = result;
    }
    of_type(ty, tables)
}

pub(super) fn of_type(ty: &ir::Type, tables: &ir::Tables) -> Of {
    let ir::Type::Meaning { index, arguments } = ty else {
        return Of::Other;
    };
    match tables.meanings[*index].label.as_str() {
        "Integer" => Of::Integer,
        "F32" => Of::F32,
        "F64" => Of::F64,
        "String" => Of::Text,
        "Maybe" => Of::Maybe,
        "Result" => Of::Result,
        "Vec" | "Deque"
            if matches!(arguments.as_slice(), [ir::Type::Meaning { index, .. }]
                if tables.meanings[*index].label == "U8") =>
        {
            Of::Bytes
        }
        "Vec" | "Deque" => Of::Seq,
        _ => Of::Other,
    }
}

/// The native of a prelude declaration.
fn choose(declaration: &ir::Declaration, tables: &ir::Tables) -> Native {
    let of = result_of(&declaration.ty, tables);
    by_label(&declaration.label, of).unwrap_or_else(|| {
        unreachable!("no native for the prelude's {}", declaration.label)
    })
}

impl Natives {
    /// Whether a prelude label has a native; the coverage test asks
    /// for every label the prelude declares.
    #[must_use]
    pub fn known(label: &str) -> bool {
        by_label(label, Of::Other).is_some()
    }
}

/// The native a label names, its shared-label kind given.
fn by_label(label: &str, of: Of) -> Option<Native> {
    Some(match label {
        "add" => Native::Add,
        "sub" => Native::Sub,
        "mul" => Native::Mul,
        "div" => Native::Div,
        "rem" => Native::Rem,
        "eq" => Native::Eq,
        "cmp" => Native::Cmp,
        "show" => Native::Show,
        "len" => Native::Len,
        "not" => Native::Not,
        "concat" => Native::Concat,
        "literal" => Native::Literal(of),
        "read" => Native::Read(of),
        "zero" => Native::Zero(of),
        "default" => Native::Default(of),
        "round" => Native::Round,
        "trunc" => Native::Trunc,
        "floor" => Native::Floor,
        "ceil" => Native::Ceil,
        "sqrt" => Native::Sqrt,
        "sin" => Native::Sin,
        "cos" => Native::Cos,
        "atan2" => Native::Atan2,
        "hypot" => Native::Hypot,
        "map" => Native::Map,
        "pure" => Native::Pure(of),
        "then" => Native::Then,
        "empty" => Native::Empty,
        "or" => Native::Or,
        "fold" => Native::Fold,
        "filter" => Native::Filter,
        "sum" => Native::Sum,
        "first" => Native::First,
        "head" => Native::Head,
        "take" => Native::Take,
        "fault" => Native::Fault,
        "log" => Native::Log,
        "sequence" | "from" => Native::Sequence,
        "to" => Native::To,
        "try" => Native::Try,
        "bytes" => Native::Bytes,
        "utf8" => Native::Utf8,
        "lines" => Native::Lines,
        "words" => Native::Words,
        "split" => Native::Split,
        "join" => Native::Join,
        "lower" => Native::Lower,
        "upper" => Native::Upper,
        "flatten" => Native::Flatten,
        "collect" => Native::Collect,
        "count" => Native::Count,
        "sort" => Native::Sort,
        "drop" => Native::Drop,
        "last" => Native::Last,
        "nth" => Native::Nth,
        "reverse" => Native::Reverse,
        "enumerate" => Native::Enumerate,
        "zip" => Native::Zip,
        "contains" => Native::Contains,
        "starts_with" => Native::StartsWith,
        "ends_with" => Native::EndsWith,
        "replace" => Native::Replace,
        _ => return None,
    })
}
