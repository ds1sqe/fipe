//! Structural walks that may meet a written member: equality, order and
//! display of a value by its tag, and the default of a type. Each walks
//! with an explicit work stack and, at a record whose meaning has a
//! written row for the member, calls it and continues with its answer.
use super::{
    natives::{Ctx, Step},
    pending::{member_call, written_row},
};
use crate::{
    ir,
    value::{arith, float, structural, text, Value},
};
use alloc::{string::String, vec::Vec};
use core::cmp::Ordering;

#[derive(Debug)]
pub(super) enum Walk {
    Eq {
        work: Vec<(Value, Value)>,
    },
    Cmp {
        work: Vec<Order>,
    },
    Show {
        work: Vec<Piece>,
        out: String,
        /// Whether the text a written `show` returns is an argument.
        argument: bool,
    },
    Default {
        work: Vec<Build>,
    },
}

#[derive(Debug)]
pub(super) enum Order {
    Pair(Value, Value),
    Decided(Ordering),
}

#[derive(Debug)]
pub(super) enum Piece {
    Value(Value, bool),
    Text(String),
}

/// A value under construction from its parts' defaults.
#[derive(Debug)]
pub(super) struct Build {
    pub(super) shape: Shape,
    pub(super) parts: Vec<ir::Type>,
    pub(super) built: Vec<Value>,
}

#[derive(Debug)]
pub(super) enum Shape {
    Tuple,
    Array(usize),
    Record {
        meaning: usize,
        constructor: usize,
    },
    /// The one type a default is asked of.
    Top,
}

impl Walk {
    pub(super) fn eq(a: Value, b: Value) -> Self {
        Self::Eq {
            work: alloc::vec![(a, b)],
        }
    }

    pub(super) fn cmp(a: Value, b: Value) -> Self {
        Self::Cmp {
            work: alloc::vec![Order::Pair(a, b)],
        }
    }

    pub(super) fn show(value: Value) -> Self {
        Self::Show {
            work: alloc::vec![Piece::Value(value, false)],
            out: String::new(),
            argument: false,
        }
    }

    pub(super) fn default(ty: ir::Type) -> Self {
        Self::Default {
            work: alloc::vec![Build {
                shape: Shape::Top,
                parts: alloc::vec![ty],
                built: Vec::new(),
            }],
        }
    }

    /// A derived member with the call's arguments: `other` then `self`
    /// for `eq` and `cmp`, `self` for `show`, none for `default`.
    pub(super) fn derived(
        kind: ir::Derived,
        mut args: Vec<Value>,
        ty: ir::Type,
    ) -> Self {
        let mut arg =
            |index: usize| core::mem::replace(&mut args[index], Value::Unit);
        match kind {
            ir::Derived::Eq => {
                let (other, this) = (arg(0), arg(1));
                Self::eq(this, other)
            }
            ir::Derived::Cmp => {
                let (other, this) = (arg(0), arg(1));
                Self::cmp(this, other)
            }
            ir::Derived::Show => Self::show(arg(0)),
            ir::Derived::Default => Self::default(ty),
        }
    }

    pub(super) fn step(
        &mut self,
        ctx: &Ctx<'_>,
        returned: Option<Value>,
    ) -> Step {
        match self {
            Self::Eq { work } => eq_step(work, ctx, returned),
            Self::Cmp { work } => cmp_step(work, ctx, returned),
            Self::Show {
                work,
                out,
                argument,
            } => show_step(work, out, argument, ctx, returned),
            Self::Default { work } => {
                super::defaults::default_step(work, ctx, returned)
            }
        }
    }
}

/// The children of two values of one shape, `None` when they differ
/// here, `Some(empty)` for equal leaves.
fn children(a: &Value, b: &Value) -> Option<Vec<(Value, Value)>> {
    let zip = |a: &[Value], b: &[Value]| {
        (a.len() == b.len())
            .then(|| a.iter().cloned().zip(b.iter().cloned()).collect())
    };
    match (a, b) {
        (Value::Seq(a), Value::Seq(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
            zip(a, b)
        }
        (Value::Record(a), Value::Record(b)) => (a.meaning == b.meaning
            && a.constructor == b.constructor)
            .then(|| zip(&a.fields, &b.fields))
            .flatten(),
        (Value::Packed(a), Value::Packed(b)) => (a.meaning == b.meaning)
            .then(|| alloc::vec![(a.value.clone(), b.value.clone())]),
        _ => structural::eq(a, b).then(Vec::new),
    }
}

fn eq_step(
    work: &mut Vec<(Value, Value)>,
    ctx: &Ctx<'_>,
    returned: Option<Value>,
) -> Step {
    if let Some(answer) = returned {
        if !ctx.prelude.is_true(&answer) {
            return Step::Done(ctx.prelude.bool(false));
        }
    }
    while let Some((a, b)) = work.pop() {
        if let (Value::Record(_), Value::Record(_)) = (&a, &b) {
            if written_row(ctx, ctx.natives.members.eq, &a).is_some() {
                return member_call(
                    ctx,
                    ctx.natives.members.eq,
                    &a,
                    alloc::vec![b],
                );
            }
        }
        match children(&a, &b) {
            None => return Step::Done(ctx.prelude.bool(false)),
            Some(pairs) => work.extend(pairs.into_iter().rev()),
        }
    }
    Step::Done(ctx.prelude.bool(true))
}

/// An `Ordering` value's order.
fn ordering_of(value: &Value) -> Ordering {
    match value {
        Value::Record(record) => match record.constructor {
            0 => Ordering::Less,
            1 => Ordering::Equal,
            _ => Ordering::Greater,
        },
        other => unreachable!("an Ordering, not {other:?}"),
    }
}

fn cmp_step(
    work: &mut Vec<Order>,
    ctx: &Ctx<'_>,
    returned: Option<Value>,
) -> Step {
    if let Some(answer) = returned {
        let ordering = ordering_of(&answer);
        if ordering != Ordering::Equal {
            return Step::Done(ctx.prelude.ordering(ordering));
        }
    }
    while let Some(item) = work.pop() {
        let (a, b) = match item {
            Order::Decided(ordering) => {
                if ordering != Ordering::Equal {
                    return Step::Done(ctx.prelude.ordering(ordering));
                }
                continue;
            }
            Order::Pair(a, b) => (a, b),
        };
        let children = |a: &[Value],
                        b: &[Value],
                        after: Ordering,
                        work: &mut Vec<Order>| {
            work.push(Order::Decided(after));
            work.extend(
                a.iter()
                    .cloned()
                    .zip(b.iter().cloned())
                    .rev()
                    .map(|(a, b)| Order::Pair(a, b)),
            );
        };
        match (&a, &b) {
            (Value::Record(x), Value::Record(y)) => {
                if written_row(ctx, ctx.natives.members.cmp, &a).is_some() {
                    return member_call(
                        ctx,
                        ctx.natives.members.cmp,
                        &a,
                        alloc::vec![b],
                    );
                }
                match x.constructor.cmp(&y.constructor) {
                    Ordering::Equal => {
                        children(&x.fields, &y.fields, Ordering::Equal, work);
                    }
                    decided => {
                        return Step::Done(ctx.prelude.ordering(decided))
                    }
                }
            }
            (Value::Seq(x), Value::Seq(y))
            | (Value::Tuple(x), Value::Tuple(y)) => {
                children(x, y, x.len().cmp(&y.len()), work);
            }
            (Value::Packed(x), Value::Packed(y)) => match x
                .meaning
                .cmp(&y.meaning)
            {
                Ordering::Equal => {
                    work.push(Order::Pair(x.value.clone(), y.value.clone()));
                }
                decided => return Step::Done(ctx.prelude.ordering(decided)),
            },
            _ => {
                let decided = structural::cmp(&a, &b);
                if decided != Ordering::Equal {
                    return Step::Done(ctx.prelude.ordering(decided));
                }
            }
        }
    }
    Step::Done(ctx.prelude.ordering(Ordering::Equal))
}

fn show_step(
    work: &mut Vec<Piece>,
    out: &mut String,
    argument: &mut bool,
    ctx: &Ctx<'_>,
    returned: Option<Value>,
) -> Step {
    if let Some(answer) = returned {
        let Value::Text(shown) = answer else {
            unreachable!("show gives text")
        };
        if *argument && shown.contains(' ') {
            out.push('(');
            out.push_str(&shown);
            out.push(')');
        } else {
            out.push_str(&shown);
        }
    }
    while let Some(piece) = work.pop() {
        let (value, is_argument) = match piece {
            Piece::Text(text) => {
                out.push_str(&text);
                continue;
            }
            Piece::Value(value, is_argument) => (value, is_argument),
        };
        match &value {
            Value::Record(record) => {
                if written_row(ctx, ctx.natives.members.show, &value).is_some()
                {
                    *argument = is_argument;
                    return member_call(
                        ctx,
                        ctx.natives.members.show,
                        &value,
                        Vec::new(),
                    );
                }
                let parenthesize = is_argument && !record.fields.is_empty();
                if parenthesize {
                    work.push(Piece::Text(String::from(")")));
                }
                for field in record.fields.iter().rev() {
                    work.push(Piece::Value(field.clone(), true));
                    work.push(Piece::Text(String::from(" ")));
                }
                let meaning = &ctx.tables.meanings[record.meaning];
                let label = meaning
                    .constructors
                    .get(record.constructor)
                    .map_or(&meaning.label, |constructor| &constructor.label);
                work.push(Piece::Text(label.clone()));
                if parenthesize {
                    work.push(Piece::Text(String::from("(")));
                }
            }
            Value::Seq(parts) | Value::Tuple(parts) => {
                let (open, close) = if matches!(value, Value::Seq(_)) {
                    ("[", "]")
                } else {
                    ("(", ")")
                };
                work.push(Piece::Text(String::from(close)));
                for (index, part) in parts.iter().enumerate().rev() {
                    work.push(Piece::Value(part.clone(), false));
                    if index > 0 {
                        work.push(Piece::Text(String::from(", ")));
                    }
                }
                work.push(Piece::Text(String::from(open)));
            }
            Value::Packed(packed) => {
                work.push(Piece::Value(packed.value.clone(), is_argument));
            }
            Value::Int(_) | Value::Big(_) => out.push_str(&arith::show(&value)),
            Value::F32(_) | Value::F64(_) => out.push_str(&float::show(&value)),
            Value::Text(_) => out.push_str(&text::show(&value)),
            Value::Unit | Value::Function(_) | Value::Meaning(_) => {
                out.push_str(&structural::show(&value, ctx.tables));
            }
        }
    }
    Step::Done(Value::text(out))
}
