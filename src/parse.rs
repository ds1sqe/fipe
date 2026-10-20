//! Syntax-only parsing of lexer output, with owned nodes and diagnostics.

use crate::{
    ast::{self, Entry, ExprId, ExprKind, Module, Name, PatternId},
    lex::Lexed,
    token::{Keyword, Kind, Magnitude, Span, Symbol, Token},
};
use alloc::{vec, vec::Vec};
use core::fmt;

mod expression;
mod item;
mod pattern;
mod types;

/// Maximum nested parser frames, independent of the later execution budget.
pub const MAX_NESTING: usize = 64;

/// Maximum depth of an expression's tree, chains included: `a + b + c`
/// is three deep. Every later stage handles the deepest expression
/// admitted on a 2 MiB stack, in debug and release builds.
pub const MAX_DEPTH: usize = 256;

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
    /// An operand of an operator followed by an argument: an application
    /// used as an operand is grouped.
    ApplicationInOperator,
    ChainedComparison,
    FlagValueMustTouch,
    ArmsNeedGroup,
    MixedArmsAndLines,
    InvalidRestPattern,
    /// A list element ended at a line break that no comma carries over.
    CommaBeforeLineBreak,
    NestingLimit,
    /// An expression deeper than [`MAX_DEPTH`].
    DepthLimit,
    /// A second top-level `=`: a binding's value is not a binding.
    BindingValue,
    /// `|!` in a type, where either and not are written `| !`.
    EitherNot,
    /// An outer doc has no eligible owner, or changes form.
    UnattachedDoc,
    /// Module docs belong before the first module entry only.
    InnerDocPosition,
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
            Self::ApplicationInOperator => f.write_str(
                "group an application used as an operand, as `n * (f (n - 1))`",
            ),
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
            Self::NestingLimit => f.write_str("parser nesting limit exceeded"),
            Self::DepthLimit => f.write_str(
                "expression too deep: bind part of it to a name first",
            ),
            Self::BindingValue => {
                f.write_str("a binding's value is not a binding")
            }
            Self::UnattachedDoc => f.write_str("doc has no owner"),
            Self::InnerDocPosition => {
                f.write_str("module doc must precede all entries")
            }
            Self::EitherNot => f.write_str("in a type, write `| !`"),
        }
    }
}

/// Parse a complete compilation unit without resolving names or types.
///
/// # Errors
/// Returns lexical or syntax diagnostics; a failed unit has no returned AST.
pub fn parse(input: Lexed) -> Result<Module, Vec<Error>> {
    if !input.errors.is_empty() {
        return Err(lexical(input.errors));
    }
    let module = parse_partial(input);
    if module.errors.is_empty() {
        Ok(module)
    } else {
        Err(module.errors)
    }
}

/// Parse what can be read, keeping each unreadable entry as an error node.
///
/// Every entry the parser reads is in `entries`, every other is a span
/// in `malformed`, and `errors` holds the diagnostics, lexical ones
/// first. An entry holding a lexical error is malformed. An invalid
/// token stream yields an empty module with that one error.
#[must_use]
pub fn parse_partial(input: Lexed) -> Module {
    let lexed = lexical(input.errors);
    let mut module = match validate(&input.tokens).and_then(|end| {
        validate_comments(&input.comments, &input.tokens, end).map(|()| end)
    }) {
        Err(error) => Module {
            errors: vec![error],
            ..Module::default()
        },
        Ok(end) => read(input.tokens, input.comments, end),
    };
    if !lexed.is_empty() {
        let mut kept = Vec::new();
        for entry in core::mem::take(&mut module.entries) {
            let span = entry_span(&module, &entry);
            if lexed.iter().any(|error| {
                span.start <= error.span.start && error.span.start < span.end
            }) {
                module.malformed.push(span);
            } else {
                kept.push(entry);
            }
        }
        module.entries = kept;
        module.malformed.sort_by_key(|span| span.start);
        let parsed = core::mem::take(&mut module.errors);
        module.errors = lexed.into_iter().chain(parsed).collect();
    }
    module
}

fn lexical(errors: Vec<crate::lex::LexError>) -> Vec<Error> {
    errors
        .into_iter()
        .map(|error| Error {
            span: error.span,
            reason: Reason::Lexical(error.reason),
        })
        .collect()
}

/// The span of an entry: its item's, or its line's expression.
fn entry_span(module: &Module, entry: &Entry) -> Span {
    match entry {
        Entry::Item(item) => item.span,
        Entry::Line(id) => {
            module.expr(*id).map_or_else(Span::default, |e| e.span)
        }
    }
}

fn read(tokens: Vec<Token>, comments: Vec<Span>, end: usize) -> Module {
    Parser {
        tokens,
        position: 0,
        previous: Span::default(),
        depth: 0,
        depths: Vec::new(),
        too_deep: None,
        module: Module {
            span: Span { start: 0, end },
            comments,
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

fn validate_comments(
    comments: &[Span],
    tokens: &[Token],
    eof: usize,
) -> Result<(), Error> {
    let mut end = 0;
    let mut token = 0;
    for &span in comments {
        while token < tokens.len() && tokens[token].span.end <= span.start {
            token += 1;
        }
        if span.start < end
            || span.end <= span.start
            || span.end > eof
            || tokens.get(token).is_some_and(|t| t.span.start < span.end)
        {
            return Err(Error {
                span,
                reason: Reason::InvalidTokenStream,
            });
        }
        end = span.end;
    }
    Ok(())
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
    /// Each expression's tree depth, by its index.
    depths: Vec<usize>,
    /// Where the entry being parsed first passed [`MAX_DEPTH`].
    too_deep: Option<Span>,
    module: Module,
    errors: Vec<Error>,
}

impl Parser {
    fn file(mut self) -> Module {
        self.newlines();
        if matches!(self.peek(), Kind::InnerDoc(_)) {
            match self.docs(true) {
                Ok(doc) => self.module.doc = doc,
                Err(error) => self.errors.push(error),
            }
        }
        while !matches!(self.peek(), Kind::Eof) {
            let start = self.here().start;
            let recorded = self.errors.len();
            let entry = self.entry().and_then(|entry| {
                self.too_deep.map_or(Ok(entry), |span| {
                    Err(Error {
                        span,
                        reason: Reason::DepthLimit,
                    })
                })
            });
            self.too_deep = None;
            match entry {
                Ok(entry) if self.errors.len() == recorded => {
                    self.module.entries.push(entry);
                }
                // An error recovered inside the entry makes it malformed.
                Ok(_) => self.malformed(start),
                Err(error) => {
                    // An error recovered inside the entry is its cause;
                    // what fails after it is a cascade.
                    if self.errors.len() == recorded {
                        self.errors.push(error);
                    }
                    self.recover(None);
                    self.malformed(start);
                }
            }
            self.newlines();
        }
        self.module.errors = self.errors;
        self.module
    }

    /// Record the entry from `start` to the last token read as an error
    /// node.
    fn malformed(&mut self, start: usize) {
        let end = self.previous.end.max(start);
        self.module.malformed.push(Span { start, end });
    }

    /// A file entry: an item, a binding, a signature-only declaration
    /// (`name: type` with no `=`), or a line; `pub` may precede a `type`,
    /// a `trait`, a binding or a declaration.
    fn entry(&mut self) -> Result<Entry, Error> {
        let doc_span = self.here();
        let doc = self.docs(false)?;
        if doc.is_some() && matches!(self.peek(), Kind::Kw(Keyword::Use)) {
            return Err(Error {
                span: doc_span,
                reason: Reason::UnattachedDoc,
            });
        }
        if doc.is_some()
            && !(self.item_start()
                || self.binding_ahead()
                || self.colon_follows()
                || matches!(self.peek(), Kind::Kw(Keyword::Pub)))
        {
            return Err(self.error(Reason::UnattachedDoc));
        }
        let public = self.keyword(Keyword::Pub);
        let mut entry = if self.item_start() {
            Entry::Item(self.item(public)?)
        } else if self.binding_ahead() {
            Entry::Line(self.binding(Context::Line, public)?)
        } else if self.colon_follows() {
            Entry::Item(self.declare(public)?)
        } else if public {
            return Err(self.expected(Expected::Name));
        } else {
            Entry::Line(self.line(Context::Line)?)
        };
        if let Some(doc) = doc {
            match &mut entry {
                Entry::Item(item)
                    if !matches!(item.kind, ast::ItemKind::Use { .. }) =>
                {
                    item.doc = Some(doc);
                }
                Entry::Line(id) => self.attach_doc(*id, doc)?,
                Entry::Item(_) => return Err(self.error(Reason::UnattachedDoc)),
            }
        }
        self.line_end(None)?;
        Ok(entry)
    }

    fn docs(
        &mut self,
        inner: bool,
    ) -> Result<Option<alloc::string::String>, Error> {
        if matches!(self.peek(), Kind::InnerDoc(_)) && !inner {
            return Err(self.error(Reason::InnerDocPosition));
        }
        let mut text = None;
        let mut paragraph = false;
        loop {
            let same = matches!(
                (self.peek(), inner),
                (Kind::Doc(_), false) | (Kind::InnerDoc(_), true)
            );
            if !same {
                break;
            }
            let token = self.bump();
            let (Kind::Doc(line) | Kind::InnerDoc(line)) = token.kind else {
                unreachable!()
            };
            let first = text.is_none();
            let joined = text.get_or_insert_with(alloc::string::String::new);
            if !first {
                joined.push('\n');
                if paragraph {
                    joined.push('\n');
                }
            }
            joined.push_str(&line);
            let before = self.position;
            self.newlines();
            paragraph = self.position - before > 1;
        }
        if text.is_some()
            && matches!((self.peek(), inner), (Kind::InnerDoc(_), false))
        {
            return Err(self.error(Reason::UnattachedDoc));
        }
        if !inner
            && text.is_some()
            && matches!(
                self.peek(),
                Kind::Eof
                    | Kind::Symbol(
                        Symbol::RBrace | Symbol::RParen | Symbol::RBracket
                    )
            )
        {
            return Err(self.error(Reason::UnattachedDoc));
        }
        Ok(text)
    }

    fn attach_doc(
        &mut self,
        id: ExprId,
        text: alloc::string::String,
    ) -> Result<(), Error> {
        if let ExprKind::Bind { doc, .. } =
            &mut self.module.expressions[id.0].kind
        {
            *doc = Some(text);
            Ok(())
        } else {
            Err(self.error(Reason::UnattachedDoc))
        }
    }

    /// Whether this line is a binding: a depth-zero `=` before its first
    /// `=>`, pipe, comma or newline, not counting an `=` after a flag; a
    /// leading `=` on the next line counts. A `=>` after a depth-zero `:`
    /// belongs to the annotation's constraints.
    fn binding_ahead(&mut self) -> bool {
        self.assign_ahead().is_some()
    }

    /// The `=` that makes this line a binding, per [`Self::binding_ahead`].
    fn assign_ahead(&mut self) -> Option<Span> {
        self.peek();
        let mut depth = 0_usize;
        let mut after_flag = false;
        let mut annotated = false;
        let tokens = &self.tokens[self.position..];
        for (index, token) in tokens.iter().enumerate() {
            match token.kind {
                Kind::Symbol(Symbol::Colon) if depth == 0 => annotated = true,
                // Inside an annotation, `,` separates constraints, `=>`
                // ends them and `|` joins types; none ends the line.
                Kind::Symbol(Symbol::FatArrow | Symbol::Comma)
                    if depth == 0 && annotated => {}
                Kind::Symbol(symbol)
                    if depth == 0 && annotated && symbol.continues_line() => {}
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
                ) => depth = depth.checked_sub(1)?,
                Kind::Symbol(Symbol::Assign) if depth == 0 => {
                    return (!after_flag).then_some(token.span);
                }
                Kind::Symbol(Symbol::FatArrow | Symbol::Comma)
                    if depth == 0 =>
                {
                    return None;
                }
                Kind::Symbol(symbol)
                    if depth == 0 && symbol.continues_line() =>
                {
                    return None;
                }
                Kind::Newline if depth == 0 => {
                    let next = tokens.get(index + 1).map(|next| &next.kind);
                    let continued_type = annotated
                        && (self.leading(self.position + index + 1)
                            || index > 0
                                && matches!(
                                    tokens[index - 1].kind,
                                    Kind::Symbol(
                                        Symbol::Arrow
                                            | Symbol::FatArrow
                                            | Symbol::Ampersand
                                            | Symbol::Pipe
                                            | Symbol::Star
                                            | Symbol::Comma
                                    )
                                ));
                    if next != Some(&Kind::Symbol(Symbol::Assign))
                        && !continued_type
                    {
                        return None;
                    }
                }
                Kind::Eof => return None,
                _ => {}
            }
            after_flag = matches!(token.kind, Kind::Flag(_));
        }
        None
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
                            | Symbol::Concat
                            | Symbol::Eq
                            | Symbol::NotEq
                            | Symbol::Less
                            | Symbol::LessEq
                            | Symbol::Greater
                            | Symbol::GreaterEq
                            | Symbol::FatArrow
                            | Symbol::Assign
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
            self.before_closer(close)?;
            break;
        }
        self.expect(close)?;
        Ok(items)
    }

    /// Only newlines before `close`, which are skipped; any other line
    /// break in a list needs a comma first.
    fn before_closer(&mut self, close: Symbol) -> Result<(), Error> {
        if self.tokens[self.position].kind != Kind::Newline {
            return Err(self.expected(Expected::Symbol(close)));
        }
        let newline = self.tokens[self.position].span;
        let mut index = self.position;
        while self.tokens[index].kind == Kind::Newline {
            index += 1;
        }
        if self.tokens[index].kind != Kind::Symbol(close) {
            return Err(Error {
                span: newline,
                reason: Reason::CommaBeforeLineBreak,
            });
        }
        self.position = index;
        Ok(())
    }

    /// Whether the `( )` whose contents start at token `from` holds a
    /// depth-zero comma before any depth-zero `=>`: a tuple. A `<`
    /// touching a name opens type arguments, whose commas are theirs.
    fn tuple_at(&self, from: usize) -> bool {
        let mut depth = 0_usize;
        let mut angles = 0_usize;
        let mut previous: Option<&Token> = None;
        for token in self.tokens.iter().skip(from) {
            let top = depth == 0 && angles == 0;
            match token.kind {
                Kind::Symbol(Symbol::Comma) if top => return true,
                Kind::Symbol(Symbol::FatArrow) if top => return false,
                Kind::Symbol(
                    Symbol::LParen | Symbol::LBracket | Symbol::LBrace,
                ) => depth += 1,
                Kind::Symbol(Symbol::Less)
                    if previous.is_some_and(|p| {
                        matches!(p.kind, Kind::Name(_))
                            && p.span.end == token.span.start
                    }) =>
                {
                    angles += 1;
                }
                // Outside type arguments `>` compares.
                Kind::Symbol(Symbol::Greater) => {
                    angles = angles.saturating_sub(1);
                }
                Kind::Symbol(
                    Symbol::RParen | Symbol::RBracket | Symbol::RBrace,
                ) => match depth.checked_sub(1) {
                    Some(less) => depth = less,
                    None => return false,
                },
                Kind::Eof => return false,
                _ => {}
            }
            previous = Some(token);
        }
        false
    }

    /// The parts of a tuple after its opener: two or more, comma
    /// separated, with a list's newlines and trailing comma.
    fn tuple<T>(
        &mut self,
        mut part: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let mut parts = Vec::new();
        loop {
            self.newlines();
            if parts.len() >= 2 && self.at(Symbol::RParen) {
                break;
            }
            parts.push(part(self)?);
            if !self.eat(Symbol::Comma) {
                break;
            }
        }
        if !self.at(Symbol::RParen) {
            self.before_closer(Symbol::RParen)?;
        }
        self.expect(Symbol::RParen)?;
        Ok(parts)
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

    fn nonnegative(&mut self) -> Result<(Magnitude, Span), Error> {
        if matches!(
            self.peek(),
            Kind::Int { negative, magnitude, .. }
                if !negative || magnitude.is_zero()
        ) {
            let token = self.bump();
            if let Kind::Int { magnitude, .. } = token.kind {
                return Ok((magnitude, token.span));
            }
        }
        Err(self.expected(Expected::NonnegativeInteger))
    }

    fn expression(&mut self, kind: ExprKind, span: Span) -> ExprId {
        let depth = 1 + self.below(&kind);
        if depth > MAX_DEPTH && self.too_deep.is_none() {
            self.too_deep = Some(match kind {
                ExprKind::Binary { operator, .. }
                | ExprKind::Pipe { operator, .. } => operator,
                _ => span,
            });
        }
        let id = ExprId(self.module.expressions.len());
        self.module.expressions.push(ast::Expr { kind, span });
        self.depths.push(depth);
        id
    }

    /// The depth of the deepest expression directly under `kind`.
    fn below(&self, kind: &ExprKind) -> usize {
        let depth = |id: &ExprId| self.depths[id.0];
        let deepest = |ids: &[ExprId]| ids.iter().map(depth).max();
        match kind {
            ExprKind::Literal(_)
            | ExprKind::Name(_)
            | ExprKind::Word(_)
            | ExprKind::Path(_)
            | ExprKind::Selector(_)
            | ExprKind::Instance { .. } => None,
            ExprKind::Group(ids)
            | ExprKind::Tuple(ids)
            | ExprKind::List(ids)
            | ExprKind::Block(ids) => deepest(ids),
            ExprKind::Outcome { payload, .. } => payload.as_ref().map(depth),
            ExprKind::Record { fields, .. } => {
                fields.iter().map(|field| depth(&field.value)).max()
            }
            ExprKind::Call {
                callee, arguments, ..
            } => arguments
                .iter()
                .filter_map(|argument| match &argument.kind {
                    ast::ArgumentKind::Positional(value)
                    | ast::ArgumentKind::Flag {
                        value: Some(value), ..
                    } => Some(depth(value)),
                    ast::ArgumentKind::Flag { value: None, .. } => None,
                })
                .chain([depth(callee)])
                .max(),
            ExprKind::Field { value, .. }
            | ExprKind::Not { value, .. }
            | ExprKind::Bind { value, .. } => Some(depth(value)),
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipe { left, right, .. } => {
                Some(depth(left).max(depth(right)))
            }
            ExprKind::With { value, body, .. } => {
                Some(depth(value).max(depth(body)))
            }
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => [Some(condition), Some(then_body), else_body.as_ref()]
                .into_iter()
                .flatten()
                .map(depth)
                .max(),
            ExprKind::Arms(arms) => arms
                .iter()
                .flat_map(|arm| {
                    arm.guard
                        .iter()
                        .chain([&arm.body])
                        .map(depth)
                        .chain(self.default_depths(&arm.patterns))
                })
                .max(),
        }
        .unwrap_or(0)
    }

    /// The depths of the defaults written in an arm head's patterns.
    fn default_depths(&self, patterns: &[PatternId]) -> Vec<usize> {
        let mut depths = Vec::new();
        let mut pending: Vec<PatternId> = patterns.to_vec();
        while let Some(id) = pending.pop() {
            match &self.module.patterns[id.0].kind {
                ast::PatternKind::Default { value, .. } => {
                    depths.push(self.depths[value.0]);
                }
                ast::PatternKind::Group(inner) => pending.push(*inner),
                ast::PatternKind::Tuple(parts) => pending.extend(parts),
                _ => {}
            }
        }
        depths
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

    /// Skip to the end of the line, or inside a bracket to its `close`
    /// outside any bracket opened while skipping, so the closer found is
    /// the enclosing one.
    fn recover(&mut self, close: Option<Symbol>) {
        let balance = close.is_some();
        let mut depth = 0_usize;
        loop {
            match self.peek() {
                Kind::Newline if depth == 0 => break,
                Kind::Eof => break,
                Kind::Symbol(symbol)
                    if depth == 0 && Some(*symbol) == close =>
                {
                    break;
                }
                Kind::Symbol(
                    Symbol::LParen | Symbol::LBracket | Symbol::LBrace,
                ) if balance => {
                    depth += 1;
                    self.bump();
                }
                Kind::Symbol(
                    Symbol::RParen | Symbol::RBracket | Symbol::RBrace,
                ) if balance => {
                    depth = depth.saturating_sub(1);
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        self.newlines();
    }
}
