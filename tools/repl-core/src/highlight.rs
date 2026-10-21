//! Token classes for colouring a line while it is typed.
//!
//! The classes come from [`fipe::lex`]'s kinds and match the
//! tree-sitter grammar's highlight captures: `@keyword…`, `@string…`
//! (with words and paths), `@number…`, `@comment…` (with docs),
//! `@constructor` and `@type` (an uppercase name), and `@operator`
//! (with the pipes). Brackets, punctuation and lowercase names have no
//! class. A line is lexed alone, so a line inside a quote that spans
//! lines is classed as if it stood by itself.

use alloc::vec::Vec;
use core::ops::Range;
use fipe::{
    lex::lex,
    token::{Kind, Symbol, Written},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Keyword,
    String,
    Number,
    Comment,
    /// An uppercase name: a constructor or a type.
    Constructor,
    /// An operator or a pipe.
    Operator,
}

/// The classed parts of `text` as char ranges, in order and apart; text
/// in no class has no range.
#[must_use]
pub fn classes(text: &str) -> Vec<(Range<usize>, Class)> {
    let lexed = lex(text);
    let mut parts: Vec<(Range<usize>, Class)> = lexed
        .tokens
        .iter()
        .filter_map(|token| {
            Some((token.span.start..token.span.end, class(&token.kind)?))
        })
        .chain(
            lexed
                .comments
                .iter()
                .map(|span| (span.start..span.end, Class::Comment)),
        )
        // An unclosed quote is a lexer error; it is still a string.
        .chain(lexed.errors.iter().filter_map(|error| {
            text.get(error.span.start..)?
                .starts_with(['"', '\''])
                .then_some((error.span.start..error.span.end, Class::String))
        }))
        .collect();
    parts.sort_by_key(|(range, _)| range.start);
    let mut end = 0;
    parts.retain(|(range, _)| {
        let apart = range.start >= end && range.start < range.end;
        end = end.max(range.end);
        apart
    });
    let chars = |byte: usize| {
        text.get(..byte.min(text.len()))
            .map_or(0, |before| before.chars().count())
    };
    parts
        .into_iter()
        .map(|(range, class)| (chars(range.start)..chars(range.end), class))
        .collect()
}

fn class(kind: &Kind) -> Option<Class> {
    Some(match kind {
        Kind::Kw(_) => Class::Keyword,
        Kind::Str { .. } | Kind::Word(_) | Kind::Path(_) => Class::String,
        Kind::Int { .. } | Kind::Float(_) => Class::Number,
        Kind::Affixed(affixed) => match affixed.form {
            Written::Text { .. } => Class::String,
            _ => Class::Number,
        },
        Kind::Doc(_) | Kind::InnerDoc(_) => Class::Comment,
        Kind::Name(segments) => {
            let last = segments.last()?;
            if !last.starts_with(|c: char| c.is_ascii_uppercase()) {
                return None;
            }
            Class::Constructor
        }
        Kind::Symbol(symbol) => match symbol {
            Symbol::LParen
            | Symbol::RParen
            | Symbol::LBrace
            | Symbol::RBrace
            | Symbol::LBracket
            | Symbol::RBracket
            | Symbol::Comma
            | Symbol::Semicolon
            | Symbol::Colon => return None,
            _ => Class::Operator,
        },
        Kind::Flag(_) | Kind::Selector(_) | Kind::Newline | Kind::Eof => {
            return None
        }
    })
}
