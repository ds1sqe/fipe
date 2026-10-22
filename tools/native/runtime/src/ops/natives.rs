//! The natives' table, copied from `src/vm/natives.rs`: the prelude's
//! operations, chosen once per tables by label and, where a label is
//! shared, by the declared type; and the dispatch members a walk by tag
//! consults.
use fipe::ir;

/// The meaning a shared label is declared for, where the value alone
/// cannot tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Of {
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
pub enum Native {
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
pub struct Members {
    pub eq: Option<usize>,
    pub cmp: Option<usize>,
    pub show: Option<usize>,
    pub default: Option<usize>,
    pub add: Option<usize>,
    pub zero: Option<usize>,
    pub len: Option<usize>,
    pub read: Option<usize>,
}

/// The natives of one tables, by declaration index.
pub struct Natives {
    table: Vec<Option<Native>>,
    pub members: Members,
}

impl Natives {
    /// The natives for every declaration of the prelude's namespace.
    ///
    /// # Panics
    ///
    /// On a prelude declaration no native answers, a crate defect.
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

    /// The native of a declaration; `None` for a host's.
    pub fn get(&self, op: usize) -> Option<Native> {
        self.table.get(op).copied().flatten()
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

pub fn of_type(ty: &ir::Type, tables: &ir::Tables) -> Of {
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
