//! Linear UTF-8 scanning with byte spans and recoverable diagnostics.

use crate::token::{Kind, Span, Symbol, Token};
use alloc::{string::String, vec::Vec};
use core::{fmt, iter::Peekable, str::CharIndices};

mod quoted;
mod word;

/// Successful tokens and all lexical diagnostics, in source order.
///
/// A result always contains one final EOF token. A consumer must reject the
/// compilation unit if errors is nonempty; recovery is for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub errors: Vec<LexError>,
}

/// A check-time failure, not an execution Fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub span: Span,
    pub reason: Reason,
}

/// Stable diagnostic categories; hosts own presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    UnterminatedQuote,
    BadEscape,
    InvalidUtf8,
    Stray(char),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedQuote => f.write_str("unterminated quote"),
            Self::BadEscape => f.write_str("invalid escape"),
            Self::InvalidUtf8 => f.write_str("string is not valid UTF-8"),
            Self::Stray(ch) => write!(f, "unexpected character {ch:?}"),
        }
    }
}

/// Tokenize one UTF-8 source without performing lookup, parsing or I/O.
#[must_use]
pub fn lex(text: &str) -> Lexed {
    Scanner {
        text,
        chars: text.char_indices().peekable(),
        offset: 0,
        continuation: false,
        output: Lexed {
            tokens: Vec::new(),
            errors: Vec::new(),
        },
    }
    .run()
}

struct Scanner<'a> {
    text: &'a str,
    chars: Peekable<CharIndices<'a>>,
    offset: usize,
    continuation: bool,
    output: Lexed,
}

impl Scanner<'_> {
    fn run(mut self) -> Lexed {
        if self.peek() == Some('\u{feff}') {
            self.bump();
        }
        while let Some(ch) = self.peek() {
            let start = self.offset;
            match ch {
                '\n' => self.newline(start),
                ch if spacing(ch) => {
                    self.bump();
                }
                '#' => self.comment(),
                '\\' if self.join_ahead() => self.join(),
                _ => {
                    let kind = self.token();
                    self.finish(start, kind);
                }
            }
        }
        self.finish(self.offset, Ok(Kind::Eof));
        self.output
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|&(_, ch)| ch)
    }

    fn bump(&mut self) -> Option<char> {
        self.chars.next().map(|(offset, ch)| {
            self.offset = offset + ch.len_utf8();
            ch
        })
    }

    fn finish(&mut self, start: usize, kind: Result<Kind, Reason>) {
        let span = Span {
            start,
            end: self.offset,
        };
        self.continuation = matches!(
            kind,
            Ok(Kind::Symbol(symbol)) if symbol.continues_line()
        );
        match kind {
            Ok(kind) => self.output.tokens.push(Token { kind, span }),
            Err(reason) => self.output.errors.push(LexError { span, reason }),
        }
    }

    fn newline(&mut self, start: usize) {
        self.bump();
        if !self.continuation {
            self.finish(start, Ok(Kind::Newline));
        }
        self.continuation = false;
    }

    /// A backslash followed only by spacing before LF or EOF joins lines.
    fn join_ahead(&self) -> bool {
        self.join_at(self.offset)
    }

    fn join_at(&self, offset: usize) -> bool {
        self.text[offset..].strip_prefix('\\').is_some_and(|rest| {
            rest.chars().take_while(|&ch| ch != '\n').all(spacing)
        })
    }

    fn join(&mut self) {
        while self.peek().is_some_and(|ch| ch != '\n') {
            self.bump();
        }
        self.bump();
        // The joined newline is gone; a following blank line still ends a
        // pipe continuation, as it would without the join.
        self.continuation = false;
    }

    fn comment(&mut self) {
        while let Some(ch) = self.peek().filter(|&ch| ch != '\n') {
            let start = self.offset;
            self.bump();
            if invisible(ch) || (ch.is_control() && !spacing(ch)) {
                self.output.errors.push(LexError {
                    span: Span {
                        start,
                        end: self.offset,
                    },
                    reason: Reason::Stray(ch),
                });
            }
        }
    }

    fn token(&mut self) -> Result<Kind, Reason> {
        if matches!(self.peek(), Some('"' | '\'')) {
            return self.quoted(String::new());
        }
        if self.peek().is_some_and(boundary) {
            return self.symbol().map(Kind::Symbol);
        }
        self.word()
    }

    fn word(&mut self) -> Result<Kind, Reason> {
        let start = self.offset;
        let end = self.word_end(start);
        while self.offset < end {
            self.bump();
        }
        let word = &self.text[start..self.offset];
        if word == "-" && self.peek() == Some('>') {
            self.bump();
            return Ok(Kind::Symbol(Symbol::Arrow));
        }
        if matches!(self.peek(), Some('"' | '\'')) && word::prefix(word) {
            return self.quoted(String::from(word));
        }
        Ok(word::classify(word))
    }

    /// Where the word from `start` ends: at a boundary or a line join.
    fn word_end(&self, start: usize) -> usize {
        self.text[start..]
            .char_indices()
            .find(|&(at, ch)| {
                boundary(ch) || (ch == '\\' && self.join_at(start + at))
            })
            .map_or(self.text.len(), |(at, _)| start + at)
    }

    fn symbol(&mut self) -> Result<Symbol, Reason> {
        let ch = self.bump().expect("token is called only before EOF");
        let pair = match (ch, self.peek()) {
            ('|', Some(':')) => Some(Symbol::PipeGood),
            ('|', Some('!')) => Some(Symbol::PipeBad),
            ('|', Some('|')) => Some(Symbol::Or),
            ('&', Some('&')) => Some(Symbol::And),
            ('=', Some('>')) => Some(Symbol::FatArrow),
            ('=', Some('=')) => Some(Symbol::Eq),
            ('!', Some('=')) => Some(Symbol::NotEq),
            ('<', Some('=')) => Some(Symbol::LessEq),
            ('>', Some('=')) => Some(Symbol::GreaterEq),
            _ => None,
        };
        if let Some(pair) = pair {
            self.bump();
            return Ok(pair);
        }
        match ch {
            '(' => Ok(Symbol::LParen),
            ')' => Ok(Symbol::RParen),
            '{' => Ok(Symbol::LBrace),
            '}' => Ok(Symbol::RBrace),
            '[' => Ok(Symbol::LBracket),
            ']' => Ok(Symbol::RBracket),
            ',' => Ok(Symbol::Comma),
            ';' => Ok(Symbol::Semicolon),
            ':' => Ok(Symbol::Colon),
            '=' => Ok(Symbol::Assign),
            '!' => Ok(Symbol::Not),
            '<' => Ok(Symbol::Less),
            '>' => Ok(Symbol::Greater),
            '|' => Ok(Symbol::Pipe),
            '&' => Ok(Symbol::Ampersand),
            _ => Err(Reason::Stray(ch)),
        }
    }
}

const fn spacing(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
    )
}

const fn invisible(ch: char) -> bool {
    matches!(ch, '\u{feff}' | '\u{200b}')
}

const fn boundary(ch: char) -> bool {
    spacing(ch)
        || ch.is_control()
        || invisible(ch)
        || matches!(
            ch,
            '(' | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | ','
                | ';'
                | ':'
                | '='
                | '<'
                | '>'
                | '!'
                | '|'
                | '&'
                | '"'
                | '\''
                | '#'
        )
}
