//! Forms the parser accepts and the checker does not check yet: each is
//! one check error that names it, until its stage of the type model.

use crate::{
    ast::{
        BinaryOp, ConstraintKind, Entry, ExprKind, ImplMember, ItemKind,
        Literal, Module, PatternKind, TypeKind,
    },
    token::{Affixed, Span, Written},
};
use alloc::vec::Vec;
use core::fmt;

/// A form that is parsed and not yet checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// `type N: Base`, or limits after `where`.
    LimitedType,
    /// A type parameter on a program's own type, `type Pair<A, B>`.
    TypeParameter,
    /// `(a, b)` or `A * B`.
    Tuple,
    /// A type's name taking a payload in a pattern: `U8 b`.
    GatePattern,
    /// A program's prefix or suffix on a number or quote; `b"…"` is
    /// checked as bytes.
    Affix,
    /// `A & B`.
    Both,
    /// `A | B`.
    Either,
    /// `!A`.
    Not,
    /// A number argument, `Bits<8>`, or a number parameter, `[T; n]`.
    Number,
    /// `a ++ b`.
    Concat,
    /// A member constraint, `b.age: a => …`, in a declaration.
    MemberConstraint,
    /// An impl member with no value, in a declaration.
    Supplied,
}

impl fmt::Display for Unchecked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LimitedType => "a limited type",
            Self::TypeParameter => "a type parameter of a program's type",
            Self::Tuple => "a tuple",
            Self::GatePattern => "a gate pattern",
            Self::Affix => "a prefix or suffix",
            Self::Both => "the type sign `&`",
            Self::Either => "the type sign `|`",
            Self::Not => "the type sign `!`",
            Self::Number => "a number in a type",
            Self::Concat => "`++`",
            Self::MemberConstraint => "a member constraint",
            Self::Supplied => "a member with no value",
        })
    }
}

/// The bytes of `b"…"`, which the checker reads as `Vec<U8>` until the
/// prelude claims the prefix.
pub fn bytes(affixed: &Affixed) -> Option<&[u8]> {
    match &affixed.form {
        Written::Text { value, raw: false }
            if affixed.prefix == "b" && affixed.suffix.is_empty() =>
        {
            Some(value)
        }
        _ => None,
    }
}

/// Every unchecked form of a declaration module, in arena order. A
/// declaration module has no gate patterns: its patterns are not
/// resolved.
pub fn scan(module: &Module) -> Vec<(Span, Unchecked)> {
    let mut found = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Type {
                name, base, limits, ..
            } if base.is_some() || !limits.is_empty() => {
                found.push((name.span, Unchecked::LimitedType));
            }
            ItemKind::Impl { members, .. } => {
                for member in members {
                    if let ImplMember::Named {
                        name, value: None, ..
                    } = member
                    {
                        found.push((name.span, Unchecked::Supplied));
                    }
                }
            }
            _ => {}
        }
    }
    for expr in &module.expressions {
        match &expr.kind {
            ExprKind::Tuple(_) => found.push((expr.span, Unchecked::Tuple)),
            ExprKind::Binary {
                op: BinaryOp::Concat,
                operator,
                ..
            } => found.push((*operator, Unchecked::Concat)),
            ExprKind::Literal(Literal::Affixed(affixed))
                if bytes(affixed).is_none() =>
            {
                found.push((expr.span, Unchecked::Affix));
            }
            _ => {}
        }
    }
    for pattern in &module.patterns {
        match &pattern.kind {
            PatternKind::Tuple(_) => {
                found.push((pattern.span, Unchecked::Tuple));
            }
            PatternKind::Literal(Literal::Affixed(affixed))
                if bytes(affixed).is_none() =>
            {
                found.push((pattern.span, Unchecked::Affix));
            }
            _ => {}
        }
    }
    let lengths: Vec<usize> = module
        .types
        .iter()
        .filter_map(|ty| match ty.kind {
            TypeKind::Array { length, .. } => Some(length.0),
            _ => None,
        })
        .collect();
    for (index, ty) in module.types.iter().enumerate() {
        let form = match &ty.kind {
            TypeKind::Tuple(_) => Unchecked::Tuple,
            TypeKind::Both(_) => Unchecked::Both,
            TypeKind::Either(_) => Unchecked::Either,
            TypeKind::Not(_) => Unchecked::Not,
            TypeKind::Number(_) if !lengths.contains(&index) => {
                Unchecked::Number
            }
            TypeKind::Named { .. } if lengths.contains(&index) => {
                Unchecked::Number
            }
            TypeKind::Qualified { constraints, .. } => {
                for constraint in constraints {
                    if matches!(constraint.kind, ConstraintKind::Member { .. })
                    {
                        found.push((
                            constraint.span,
                            Unchecked::MemberConstraint,
                        ));
                    }
                }
                continue;
            }
            _ => continue,
        };
        found.push((ty.span, form));
    }
    found
}
