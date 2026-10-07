//! Forms the parser accepts and the checker does not check yet: each is
//! one check error that names it, until its stage of the type model.

use crate::{
    ast::{ExprKind, Literal, Module, PatternKind},
    token::{Affixed, Span, Written},
};
use alloc::vec::Vec;
use core::fmt;

/// A form that is parsed and not yet checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// A program's prefix or suffix on a number or quote; `b"…"` is
    /// checked as bytes.
    Affix,
}

impl fmt::Display for Unchecked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Affix => "a prefix or suffix",
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
            PatternKind::Literal(Literal::Affixed(affixed))
                if bytes(affixed).is_none() =>
            {
                found.push((pattern.span, Unchecked::Affix));
            }
            _ => {}
        }
    }
    found
}
