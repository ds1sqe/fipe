//! Syntax-only parsing of lexer output, with owned nodes and diagnostics.

use crate::{
    ast::{self, Entry, ExprId, ExprKind, Module, Name, PatternId},
    lex::Lexed,
    token::{Keyword, Kind, Span, Symbol, Token},
};
use alloc::{vec, vec::Vec};
use core::fmt;

mod expression;
mod item;
mod pattern;
mod types;

/// Maximum nested parser frames, independent of the later execution budget.
pub const MAX_NESTING: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: Span,
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Lexical(crate::lex::Reason),
    InvalidTokenStream,
    Expected(Expected),
    OperatorInArguments,
    ChainedComparison,
    FlagValueMustTouch,
    ArmsNeedGroup,
    MixedArmsAndLines,
    InvalidRestPattern,
    /// A list element ended at a line break that no comma carries over.
    CommaBeforeLineBreak,
    /// A record literal in an `if` or `with` head, where `{` is the body.
    RecordInHead,
    NestingLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    Expression,
    Pattern,
    Type,
    Name,
    UnqualifiedName,
    NonnegativeInteger,
    LineEnd,
    Symbol(Symbol),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lexical(reason) => reason.fmt(f),
            Self::InvalidTokenStream => f.write_str("invalid token stream"),
            Self::Expected(expected) => write!(f, "expected {expected:?}"),
            Self::OperatorInArguments => {
                f.write_str("group an operator expression used as an argument")
            }
            Self::ChainedComparison => f.write_str("group chained comparisons"),
            Self::FlagValueMustTouch => {
                f.write_str("flag, equals and attached value must touch")
            }
            Self::ArmsNeedGroup => f.write_str("bracket the arm function"),
            Self::MixedArmsAndLines => {
                f.write_str("an arm list may contain only arms")
            }
            Self::InvalidRestPattern => {
                f.write_str("record rest must be the final pattern field")
            }
            Self::CommaBeforeLineBreak => {
                f.write_str("expected `,` before the line break")
            }
            Self::RecordInHead => {
                f.write_str("group a record used in an `if` or `with` head")
            }
            Self::NestingLimit => f.write_str("parser nesting limit exceeded"),
        }
    }
}

/// Parse a complete compilation unit without resolving names or types.
///
/// # Errors
/// Returns lexical or syntax diagnostics; a failed unit has no returned AST.
pub fn parse(input: Lexed) -> Result<Module, Vec<Error>> {
    if !input.errors.is_empty() {
        return Err(input
            .errors
            .into_iter()
            .map(|error| Error {
                span: error.span,
                reason: Reason::Lexical(error.reason),
            })
            .collect());
    }
    let end = validate(&input.tokens).map_err(|error| vec![error])?;
    Parser {
        tokens: input.tokens,
        position: 0,
        previous: Span::default(),
        depth: 0,
        module: Module {
            span: Span { start: 0, end },
            ..Module::default()
        },
        errors: Vec::new(),
    }
    .file()
}

fn validate(tokens: &[Token]) -> Result<usize, Error> {
    let mut end = 0;
    for (index, token) in tokens.iter().enumerate() {
        let span = token.span;
        let valid = span.start >= end && span.end >= span.start;
        let eof = token.kind == Kind::Eof;
        let width = match token.kind {
            Kind::Symbol(symbol) => Some(symbol.spelling().len()),
            Kind::Newline => Some(1),
            Kind::Eof => Some(0),
            _ => None,
        };
        if !valid
            || eof != (index + 1 == tokens.len())
            || width.is_some_and(|width| span.end - span.start != width)
            || (!eof && span.start == span.end)
        {
            return Err(Error {
                span,
                reason: Reason::InvalidTokenStream,
            });
        }
        end = span.end;
    }
    if tokens.is_empty() {
        return Err(Error {
            span: Span::default(),
            reason: Reason::InvalidTokenStream,
        });
    }
    Ok(end)
}

/// Where a line is being parsed; newlines are handled the same everywhere.
/// Only the arm rule and the `if`/`with` body rule depend on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// A file, block or group line: an unbracketed arm list is allowed.
    Line,
    /// A list element or an arm body: an arm list needs its own brackets.
    Element,
    /// An `if` or `with` head: the first top-level `{` is the body.
    Head,
}

impl Context {
    const fn arms(self) -> bool {
        matches!(self, Self::Line)
    }

    const fn body(self) -> bool {
        matches!(self, Self::Head)
    }
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    previous: Span,
    depth: usize,
    module: Module,
    errors: Vec<Error>,
}

impl Parser {
    fn file(mut self) -> Result<Module, Vec<Error>> {
        self.newlines();
        while !matches!(self.peek(), Kind::Eof) {
            match self.entry() {
                Ok(entry) => self.module.entries.push(entry),
                Err(error) => {
                    self.errors.push(error);
                    self.recover(None);
                }
            }
            self.newlines();
        }
        if self.errors.is_empty() {
            Ok(self.module)
        } else {
            Err(self.errors)
        }
    }

    /// A file entry: an item, a line, or `pub` before a `type`, a `trait`
    /// or a binding.
    fn entry(&mut self) -> Result<Entry, Error> {
        let public = self.keyword(Keyword::Pub);
        let entry = if self.item_start() {
            Entry::Item(self.item(public)?)
        } else if public {
            if !self.binding_ahead() {
                return Err(self.expected(Expected::Name));
            }
            Entry::Line(self.binding(Context::Line, true)?)
        } else {
            Entry::Line(self.line(Context::Line)?)
        };
        self.line_end(None)?;
        Ok(entry)
    }

    /// Whether this line is a binding: a depth-zero `=` before its first
    /// `=>`, pipe, comma or newline, not counting an `=` after a flag.
    fn binding_ahead(&mut self) -> bool {
        self.peek();
        let mut depth = 0_usize;
        let mut after_flag = false;
        for token in &self.tokens[self.position..] {
            match token.kind {
                Kind::Symbol(
                    Symbol::LParen
                    | Symbol::LBracket
                    | Symbol::LBrace
                    | Symbol::Less,
                ) => depth += 1,
                Kind::Symbol(
                    Symbol::RParen
                    | Symbol::RBracket
                    | Symbol::RBrace
                    | Symbol::Greater,
                ) => match depth.checked_sub(1) {
                    Some(less) => depth = less,
                    None => return false,
                },
                Kind::Symbol(Symbol::Assign) if depth == 0 => {
                    return !after_flag;
                }
                Kind::Symbol(Symbol::FatArrow | Symbol::Comma)
                    if depth == 0 =>
                {
                    return false;
                }
                Kind::Symbol(symbol)
                    if depth == 0 && symbol.continues_line() =>
                {
                    return false;
                }
                Kind::Newline if depth == 0 => return false,
                Kind::Eof => return false,
                _ => {}
            }
            after_flag = matches!(token.kind, Kind::Flag(_));
        }
        false
    }

    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.depth == MAX_NESTING {
            return Err(self.error(Reason::NestingLimit));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    /// The current token, after folding a newline that a leading operator
    /// on the next line carries over. A blank line never folds.
    fn peek(&mut self) -> &Kind {
        if self.tokens[self.position].kind == Kind::Newline
            && self.leading(self.position + 1)
        {
            self.position += 1;
        }
        &self.tokens[self.position].kind
    }

    fn leading(&self, index: usize) -> bool {
        let Some(token) = self.tokens.get(index) else {
            return false;
        };
        match token.kind {
            Kind::Symbol(Symbol::Slash) => {
                self.tokens.get(index + 1).is_some_and(|next| {
                    expression::operand_start(&next.kind, Context::Line)
                })
            }
            Kind::Symbol(symbol) => {
                symbol.continues_line()
                    || matches!(
                        symbol,
                        Symbol::Plus
                            | Symbol::Minus
                            | Symbol::Star
                            | Symbol::Percent
                            | Symbol::Eq
                            | Symbol::NotEq
                            | Symbol::Less
                            | Symbol::LessEq
                            | Symbol::Greater
                            | Symbol::GreaterEq
                            | Symbol::FatArrow
                    )
            }
            _ => false,
        }
    }

    /// After a trailing operator: one newline is carried over, never two.
    fn skip_newline(&mut self) {
        if self.tokens[self.position].kind == Kind::Newline {
            self.position += 1;
        }
    }

    fn here(&mut self) -> Span {
        self.peek();
        self.tokens[self.position].span
    }

    fn bump(&mut self) -> Token {
        self.peek();
        let token = &mut self.tokens[self.position];
        let span = token.span;
        let kind = core::mem::replace(&mut token.kind, Kind::Eof);
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
        self.previous = span;
        Token { kind, span }
    }

    fn newlines(&mut self) {
        while self.tokens[self.position].kind == Kind::Newline {
            self.position += 1;
        }
    }

    fn at(&mut self, symbol: Symbol) -> bool {
        *self.peek() == Kind::Symbol(symbol)
    }

    fn eat(&mut self, symbol: Symbol) -> bool {
        if self.at(symbol) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn keyword(&mut self, keyword: Keyword) -> bool {
        if *self.peek() == Kind::Kw(keyword) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: Symbol) -> Result<Span, Error> {
        if self.eat(symbol) {
            Ok(self.previous)
        } else {
            Err(self.expected(Expected::Symbol(symbol)))
        }
    }

    fn error(&mut self, reason: Reason) -> Error {
        Error {
            span: self.here(),
            reason,
        }
    }

    fn expected(&mut self, expected: Expected) -> Error {
        self.error(Reason::Expected(expected))
    }

    const fn since(&self, start: usize) -> Span {
        Span {
            start,
            end: self.previous.end,
        }
    }

    /// A comma list after its opener: newlines may follow the opener or a
    /// comma or precede the closer; anywhere else they need a comma first.
    fn list<T>(
        &mut self,
        close: Symbol,
        mut item: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let mut items = Vec::new();
        loop {
            self.newlines();
            if self.at(close) {
                break;
            }
            items.push(item(self)?);
            if self.eat(Symbol::Comma) {
                continue;
            }
            if self.at(close) {
                break;
            }
            if self.tokens[self.position].kind == Kind::Newline {
                let newline = self.tokens[self.position].span;
                let mut index = self.position;
                while self.tokens[index].kind == Kind::Newline {
                    index += 1;
                }
                if self.tokens[index].kind == Kind::Symbol(close) {
                    self.position = index;
                    break;
                }
                return Err(Error {
                    span: newline,
                    reason: Reason::CommaBeforeLineBreak,
                });
            }
            return Err(self.expected(Expected::Symbol(close)));
        }
        self.expect(close)?;
        Ok(items)
    }

    fn name(&mut self, single: bool) -> Result<Name, Error> {
        if !matches!(self.peek(), Kind::Name(parts) if !parts.is_empty()) {
            return Err(self.expected(Expected::Name));
        }
        let token = self.bump();
        let Kind::Name(segments) = token.kind else {
            unreachable!("name kind was checked before advancing")
        };
        if single && segments.len() != 1 {
            return Err(Error {
                span: token.span,
                reason: Reason::Expected(Expected::UnqualifiedName),
            });
        }
        Ok(Name {
            segments,
            span: token.span,
        })
    }

    fn nonnegative(&mut self) -> Result<(u64, Span), Error> {
        if let Kind::Int {
            negative,
            magnitude,
        } = *self.peek()
        {
            if !negative || magnitude == 0 {
                let span = self.bump().span;
                return Ok((magnitude, span));
            }
        }
        Err(self.expected(Expected::NonnegativeInteger))
    }

    fn expression(&mut self, kind: ExprKind, span: Span) -> ExprId {
        let id = ExprId(self.module.expressions.len());
        self.module.expressions.push(ast::Expr { kind, span });
        id
    }

    fn expr_span(&self, id: ExprId) -> Span {
        self.module.expressions[id.0].span
    }

    fn pattern_span(&self, id: PatternId) -> Span {
        self.module.patterns[id.0].span
    }

    fn line_end(&mut self, close: Option<Symbol>) -> Result<(), Error> {
        match self.peek() {
            Kind::Newline | Kind::Eof => Ok(()),
            Kind::Symbol(symbol) if Some(*symbol) == close => Ok(()),
            _ => Err(self.expected(Expected::LineEnd)),
        }
    }

    fn recover(&mut self, close: Option<Symbol>) {
        loop {
            match self.peek() {
                Kind::Newline | Kind::Eof => break,
                Kind::Symbol(symbol) if Some(*symbol) == close => break,
                _ => {
                    self.bump();
                }
            }
        }
        self.newlines();
    }
}
