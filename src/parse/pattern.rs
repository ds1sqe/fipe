use super::{
    expression::literal, Context, Error, Expected, Parser, Reason, MAX_NESTING,
};
use crate::{
    ast::{
        Arm, ExprId, ExprKind, Literal, Pattern, PatternField, PatternId,
        PatternKind,
    },
    token::{Keyword, Kind, Span, Symbol},
};
use alloc::{vec, vec::Vec};

impl Parser {
    /// The span of a top-level `=>` before this line ends, if any.
    pub(super) fn arm_arrow(&mut self, context: Context) -> Option<Span> {
        self.peek();
        if context.body() {
            return None;
        }
        let mut depth = 0_usize;
        for (index, token) in self.tokens.iter().enumerate().skip(self.position)
        {
            match token.kind {
                Kind::Symbol(Symbol::FatArrow) if depth == 0 => {
                    return Some(token.span);
                }
                Kind::Symbol(
                    Symbol::LParen | Symbol::LBracket | Symbol::LBrace,
                ) => {
                    depth += 1;
                    // Deeper patterns cannot pass the parser's own bound.
                    // Stop lookahead before recovery repeats a long suffix.
                    if depth > MAX_NESTING {
                        return None;
                    }
                }
                Kind::Symbol(
                    Symbol::RParen | Symbol::RBracket | Symbol::RBrace,
                ) => {
                    depth = depth.checked_sub(1)?;
                }
                // A head has no depth-zero `:`; one starts an annotation,
                // whose `=>` introduces constraints, not an arm.
                Kind::Symbol(
                    Symbol::Comma | Symbol::Assign | Symbol::Colon,
                ) if depth == 0 => {
                    return None;
                }
                Kind::Symbol(symbol)
                    if depth == 0 && symbol.continues_line() =>
                {
                    return None;
                }
                // A leading `=>` on the next line continues this one.
                Kind::Newline if depth == 0 => {
                    let next =
                        self.tokens.get(index + 1).map(|next| &next.kind);
                    if next != Some(&Kind::Symbol(Symbol::FatArrow)) {
                        return None;
                    }
                }
                Kind::Eof => return None,
                _ => {}
            }
        }
        None
    }

    /// An unbracketed arm list at a stage; a trailing comma carries one
    /// newline over, like any other trailing operator.
    pub(super) fn arms(&mut self) -> Result<ExprId, Error> {
        let start = self.here().start;
        let mut arms = vec![self.arm()?];
        while self.eat(Symbol::Comma) {
            self.skip_newline();
            if self.arm_arrow(Context::Line).is_none() {
                break;
            }
            arms.push(self.arm()?);
        }
        Ok(self.expression(ExprKind::Arms(arms), self.since(start)))
    }

    /// A bracketed arm list after its opener: a comma list of arms.
    pub(super) fn arm_list(
        &mut self,
        start: usize,
        close: Symbol,
    ) -> Result<ExprId, Error> {
        let arms = self.list(close, |parser| {
            if parser.arm_arrow(Context::Line).is_none() {
                return Err(parser.error(Reason::MixedArmsAndLines));
            }
            parser.arm()
        })?;
        Ok(self.expression(ExprKind::Arms(arms), self.since(start)))
    }

    /// One arm: a head of one or more patterns, a guard, `=>` and a body.
    pub(super) fn arm(&mut self) -> Result<Arm, Error> {
        let mut patterns = vec![self.head_pattern()?];
        while !self.at(Symbol::FatArrow)
            && *self.peek() != Kind::Kw(Keyword::If)
        {
            patterns.push(self.head_pattern()?);
        }
        let guard = if self.keyword(Keyword::If) {
            Some(self.stage(Context::Head)?)
        } else {
            None
        };
        self.expect(Symbol::FatArrow)?;
        self.skip_newline();
        // An arm body is a non-arm line; a nested arm list needs brackets.
        let body = self.line(Context::Element)?;
        let start = self.pattern_span(patterns[0]).start;
        Ok(Arm {
            patterns,
            guard,
            body,
            span: Span {
                start,
                end: self.expr_span(body).end,
            },
        })
    }

    /// A head atom: `(name = operand)` carries a default; otherwise a
    /// pattern, where an uppercase name takes one payload.
    fn head_pattern(&mut self) -> Result<PatternId, Error> {
        self.peek();
        let defaulted = self.tokens[self.position].kind
            == Kind::Symbol(Symbol::LParen)
            && matches!(
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                Some(Kind::Name(_))
            )
            && self.tokens.get(self.position + 2).map(|token| &token.kind)
                == Some(&Kind::Symbol(Symbol::Assign));
        if !defaulted {
            return self.pattern(Context::Element);
        }
        let start = self.bump().span.start;
        let name = self.name(true)?;
        self.expect(Symbol::Assign)?;
        let value = self.operand(Context::Element)?;
        self.expect(Symbol::RParen)?;
        Ok(self.add_pattern(
            PatternKind::Default { name, value },
            self.since(start),
        ))
    }

    pub(super) fn pattern(
        &mut self,
        context: Context,
    ) -> Result<PatternId, Error> {
        self.nested(|parser| parser.pattern_inner(context))
    }

    fn pattern_inner(&mut self, context: Context) -> Result<PatternId, Error> {
        let start = self.here().start;
        let tuple = self.tuple_at(self.position + 1);
        let kind = match self.peek() {
            Kind::Name(_) => return self.name_pattern(context),
            Kind::Symbol(Symbol::LParen) if tuple => {
                self.bump();
                let parts =
                    self.tuple(|parser| parser.pattern(Context::Element))?;
                PatternKind::Tuple(parts)
            }
            Kind::Symbol(Symbol::LParen) => {
                self.bump();
                let kind = if self.at(Symbol::RParen) {
                    PatternKind::Literal(Literal::Unit)
                } else {
                    PatternKind::Group(self.pattern(Context::Element)?)
                };
                self.expect(Symbol::RParen)?;
                kind
            }
            Kind::Symbol(Symbol::LBracket) => return self.bracket_pattern(),
            _ => {
                if !pattern_start(self.peek()) {
                    return Err(self.expected(Expected::Pattern));
                }
                let token = self.bump();
                PatternKind::Literal(literal(token.kind).ok_or(Error {
                    span: token.span,
                    reason: Reason::Expected(Expected::Pattern),
                })?)
            }
        };
        Ok(self.add_pattern(kind, self.since(start)))
    }

    fn name_pattern(&mut self, context: Context) -> Result<PatternId, Error> {
        let name = self.name(false)?;
        let start = name.span.start;
        let mut span = name.span;
        let kind = if name.segments == ["_"] {
            PatternKind::Wildcard
        } else if self.eat(Symbol::LBrace) {
            let (fields, rest) = self.pattern_fields()?;
            span = self.since(start);
            PatternKind::Record { name, fields, rest }
        } else if name.is_uppercase() && pattern_start(self.peek()) {
            // The case rule: a constructor takes the pattern after it.
            let payload = self.pattern(context)?;
            span.end = self.pattern_span(payload).end;
            PatternKind::Constructor { name, payload }
        } else {
            PatternKind::Name(name)
        };
        Ok(self.add_pattern(kind, span))
    }

    /// `[`: an outcome pattern when `:` or `!` follows, otherwise a list.
    fn bracket_pattern(&mut self) -> Result<PatternId, Error> {
        let start = self.bump().span.start;
        let good = match self.tokens[self.position].kind {
            Kind::Symbol(Symbol::Colon) => Some(true),
            Kind::Symbol(Symbol::Not) => Some(false),
            _ => None,
        };
        if let Some(good) = good {
            self.bump();
            let payload = if self.at(Symbol::RBracket) {
                None
            } else {
                Some(self.pattern(Context::Element)?)
            };
            self.expect(Symbol::RBracket)?;
            return Ok(self.add_pattern(
                PatternKind::Outcome { good, payload },
                self.since(start),
            ));
        }
        let elements = self.list(Symbol::RBracket, |parser| {
            parser.pattern(Context::Element)
        })?;
        Ok(self.add_pattern(PatternKind::List(elements), self.since(start)))
    }

    fn pattern_fields(
        &mut self,
    ) -> Result<(Vec<PatternField>, Option<Span>), Error> {
        let mut rest = None;
        let fields = self.list(Symbol::RBrace, |parser| {
            if rest.is_some() {
                return Err(parser.error(Reason::InvalidRestPattern));
            }
            if matches!(parser.peek(), Kind::Word(word) if word == "..") {
                rest = Some(parser.bump().span);
                return Ok(None);
            }
            parser.pattern_field().map(Some)
        })?;
        Ok((fields.into_iter().flatten().collect(), rest))
    }

    fn pattern_field(&mut self) -> Result<PatternField, Error> {
        let name = self.name(true)?;
        let pattern = if self.eat(Symbol::Colon) {
            Some(self.pattern(Context::Element)?)
        } else {
            None
        };
        let span = Span {
            start: name.span.start,
            end: pattern.map_or(name.span.end, |id| self.pattern_span(id).end),
        };
        Ok(PatternField {
            name,
            pattern,
            span,
        })
    }

    fn add_pattern(&mut self, kind: PatternKind, span: Span) -> PatternId {
        let id = PatternId(self.module.patterns.len());
        self.module.patterns.push(Pattern { kind, span });
        id
    }
}

const fn pattern_start(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Name(_)
            | Kind::Int { .. }
            | Kind::Float(_)
            | Kind::Str { .. }
            | Kind::Affixed(_)
            | Kind::Symbol(Symbol::LParen | Symbol::LBracket)
    )
}
