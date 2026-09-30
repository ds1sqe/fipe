//! Source positions and the parser-facing lexical vocabulary.

use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;

mod display;
mod magnitude;

pub use magnitude::Magnitude;

/// Half-open byte offsets into one source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A borrowed source. File identity and diagnostic presentation are host-owned.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    text: &'a str,
}

impl<'a> Source<'a> {
    #[must_use]
    pub const fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// Exact spelling, or None for an invalid/out-of-bounds span.
    #[must_use]
    pub fn slice(self, span: Span) -> Option<&'a str> {
        self.text.get(span.start..span.end)
    }

    /// One-based line and Unicode scalar column, computed on demand.
    ///
    /// Offsets past EOF clamp to EOF. An offset inside a UTF-8 scalar clamps
    /// to that scalar's start. Only LF advances the line; TAB counts as one
    /// scalar. This is a diagnostic position, not a screen coordinate.
    #[must_use]
    pub fn line_col(self, offset: usize) -> (usize, usize) {
        let mut end = offset.min(self.text.len());
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        self.text[..end].chars().fold((1, 1), |(line, col), ch| {
            if ch == '\n' {
                (line + 1, 1)
            } else {
                (line, col + 1)
            }
        })
    }
}

/// A token owns its payload and retains its original source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}

/// Lexical meaning only: names, paths and words perform no lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Kw(Keyword),
    Symbol(Symbol),
    Name(Vec<String>),
    Word(String),
    Flag(String),
    Path(String),
    Selector(Vec<String>),
    /// Sign and exact magnitude; the lexer has no bound.
    Int {
        negative: bool,
        magnitude: Magnitude,
    },
    /// Exact decimal spelling; the checker chooses width and rounds once.
    Float(String),
    /// Valid UTF-8. The raw flag distinguishes single from decoded quotes.
    Str {
        value: String,
        raw: bool,
    },
    /// A number or quote with a program's prefix or suffix.
    Affixed(Box<Affixed>),
    Newline,
    Eof,
}

/// A written form with a prefix or suffix, each read with no type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affixed {
    /// A quote's prefix name, or the letter after a number's `0`; empty
    /// when not written.
    pub prefix: String,
    pub form: Written,
    /// Empty when not written.
    pub suffix: String,
}

/// What an [`Affixed`] form holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Written {
    /// Decimal, `0x`, `0o` or `0b`, decoded.
    Int {
        negative: bool,
        magnitude: Magnitude,
    },
    /// Exact spelling, sign included, suffix excluded.
    Float(String),
    /// The digits after a program's prefix, as written.
    Digits { negative: bool, digits: String },
    /// A quote's decoded bytes; whether they are UTF-8 is the reader's.
    Text { value: Vec<u8>, raw: bool },
}

/// Reserved words. Prelude constructors and host API names are not keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    If,
    Else,
    With,
    Type,
    Impl,
    Use,
    Pub,
}

impl Keyword {
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "if" => Self::If,
            "else" => Self::Else,
            "with" => Self::With,
            "type" => Self::Type,
            "impl" => Self::Impl,
            "use" => Self::Use,
            "pub" => Self::Pub,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Self::If => "if",
            Self::Else => "else",
            Self::With => "with",
            Self::Type => "type",
            Self::Impl => "impl",
            Self::Use => "use",
            Self::Pub => "pub",
        }
    }
}

/// Punctuation and operators, independent of their grammatical position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    /// `++`, a whole word.
    Concat,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    Assign,
    Eq,
    Not,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Arrow,
    FatArrow,
    Pipe,
    PipeGood,
    PipeBad,
    Or,
    And,
    /// `&`, which joins types.
    Ampersand,
}

impl Symbol {
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::Concat => "++",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::Comma => ",",
            Self::Semicolon => ";",
            Self::Colon => ":",
            Self::Assign => "=",
            Self::Eq => "==",
            Self::Not => "!",
            Self::NotEq => "!=",
            Self::Less => "<",
            Self::LessEq => "<=",
            Self::Greater => ">",
            Self::GreaterEq => ">=",
            Self::Arrow => "->",
            Self::FatArrow => "=>",
            Self::Pipe => "|",
            Self::PipeGood => "|:",
            Self::PipeBad => "|!",
            Self::Or => "||",
            Self::And => "&&",
            Self::Ampersand => "&",
        }
    }

    /// The five pipes: the lexer suppresses the first newline after them.
    #[must_use]
    pub const fn continues_line(self) -> bool {
        matches!(
            self,
            Self::Pipe | Self::PipeGood | Self::PipeBad | Self::Or | Self::And
        )
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.spelling())
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.spelling())
    }
}
