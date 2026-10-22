//! The natives, copied from `src/vm/natives.rs` (the table, by label in
//! the prelude's namespace) and `src/vm/immediate.rs` (what each does).
//! Only the `Integer` operations are copied so far; the lowering
//! refuses a program calling another.
use fipe::{
    ir,
    value::{arith, structural, Prelude, Reason, Value},
};

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
}

/// What a declaration is to a call of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A declaration of a host's namespace.
    Host,
    /// A prelude operation, `None` when not copied yet.
    Native(Option<Native>),
}

/// The natives of one tables, by declaration index.
pub struct Natives(Vec<Kind>);

impl Natives {
    /// The natives for every declaration of the prelude's namespace.
    pub fn new(tables: &ir::Tables) -> Self {
        let prelude = tables
            .namespaces
            .iter()
            .position(|label| label == "prelude");
        Self(
            tables
                .declarations
                .iter()
                .map(|declaration| {
                    if Some(declaration.namespace) == prelude {
                        let integer = integer(&declaration.ty, tables);
                        Kind::Native(
                            by_label(&declaration.label).filter(|_| integer),
                        )
                    } else {
                        Kind::Host
                    }
                })
                .collect(),
        )
    }

    pub fn get(&self, op: usize) -> Kind {
        self.0[op]
    }
}

/// Whether a declared type takes an `Integer` first: an operation of
/// `impl Integer`, the one meaning whose natives are copied.
fn integer(ty: &ir::Type, tables: &ir::Tables) -> bool {
    matches!(ty, ir::Type::Function { parameter, .. }
        if matches!(**parameter, ir::Type::Meaning { index, .. }
            if tables.meanings[index].label == "Integer"))
}

/// The native a label names.
fn by_label(label: &str) -> Option<Native> {
    Some(match label {
        "add" => Native::Add,
        "sub" => Native::Sub,
        "mul" => Native::Mul,
        "div" => Native::Div,
        "rem" => Native::Rem,
        "eq" => Native::Eq,
        "cmp" => Native::Cmp,
        _ => return None,
    })
}

/// A native with its arguments: the value at once.
pub fn immediate(
    native: Native,
    mut args: Vec<Value>,
    prelude: &Prelude,
) -> Result<Value, Reason> {
    let mut arg =
        |index: usize| core::mem::replace(&mut args[index], Value::Unit);
    // `other: Self -> (self: Self)`: the receiver is spot 1.
    let (other, this) = (arg(0), arg(1));
    Ok(match native {
        Native::Add => arith::add(&this, &other),
        Native::Sub => arith::sub(&this, &other),
        Native::Mul => arith::mul(&this, &other),
        Native::Div => arith::div(&this, &other)?,
        Native::Rem => arith::rem(&this, &other)?,
        Native::Eq | Native::Cmp => {
            assert!(
                leaf(&this) && leaf(&other),
                "an integer native on integers"
            );
            if native == Native::Eq {
                prelude.bool(structural::eq(&this, &other))
            } else {
                prelude.ordering(structural::cmp(&this, &other))
            }
        }
    })
}

/// Whether a value is a leaf no written member can be met under
/// (`src/vm/step.rs`).
const fn leaf(value: &Value) -> bool {
    !matches!(
        value,
        Value::Record(_) | Value::Seq(_) | Value::Tuple(_) | Value::Packed(_)
    )
}
