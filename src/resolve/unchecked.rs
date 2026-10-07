//! Forms the parser accepts and the checker does not check yet: each is
//! one check error that names it, until its stage of the type model.

use crate::{
    ast::{ExprKind, Literal, Module, PatternKind, TypeKind},
    token::{Affixed, Span, Written},
};
use alloc::vec::Vec;
use core::fmt;

/// A form that is parsed and not yet checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// A type parameter on a program's own type, `type Pair<A, B>`.
    TypeParameter,
    /// `(a, b)` or `A * B`.
    Tuple,
    /// A program's prefix or suffix on a number or quote; `b"…"` is
    /// checked as bytes.
    Affix,
    /// A number argument, `Bits<8>`, or a number parameter, `[T; n]`.
    Number,
    /// `a ++ b` on arrays.
    Concat,
}

impl fmt::Display for Unchecked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TypeParameter => "a type parameter of a program's type",
            Self::Tuple => "a tuple",
            Self::Affix => "a prefix or suffix",
            Self::Number => "a number in a type",
            Self::Concat => "`++`",
        })
    }
}

/// The bytes of `b"…"`, which the checker reads as `Vec<U8>` until the
/// prelude claims the prefix (open.md L1).
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
    for expr in &module.expressions {
        match &expr.kind {
            ExprKind::Tuple(_) => found.push((expr.span, Unchecked::Tuple)),
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
            TypeKind::Number(_) if !lengths.contains(&index) => {
                Unchecked::Number
            }
            TypeKind::Named { .. } if lengths.contains(&index) => {
                Unchecked::Number
            }
            _ => continue,
        };
        found.push((ty.span, form));
    }
    found
}
