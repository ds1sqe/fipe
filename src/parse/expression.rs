use super::{Context, Error, Expected, Parser, Reason};
use crate::{
    ast::{
        Argument, ArgumentKind, BinaryOp, CallStyle, ExprId, ExprKind,
        FieldValue, Literal, Name, PipeOp,
    },
    token::{Keyword, Kind, Span, Symbol},
};
use alloc::{string::String, vec::Vec};

impl Parser {
    pub(super) fn line(&mut self, context: Context) -> Result<ExprId, Error> {
        self.nested(|parser| {
            if matches!(parser.peek(), Kind::Doc(_)) {
                return Err(parser.error(Reason::UnattachedDoc));
            }
            if *parser.peek() == Kind::Kw(Keyword::With) {
                parser.scoped(context)
            } else if parser.binding_ahead() {
                parser.binding(context, false)
            } else {
                parser.pipeline(context)
            }
        })
    }

    fn documented_line(&mut self, context: Context) -> Result<ExprId, Error> {
        let doc = self.docs(false)?;
        if doc.is_some() && !self.binding_ahead() {
            return Err(self.error(Reason::UnattachedDoc));
        }
        let id = self.line(context)?;
        if let Some(doc) = doc {
            self.attach_doc(id, doc)?;
        }
        Ok(id)
    }

    /// `pattern [: type] = line`; `=` always makes a new binding.
    pub(super) fn binding(
        &mut self,
        context: Context,
        public: bool,
    ) -> Result<ExprId, Error> {
        let pattern = self.pattern(context)?;
        let start = self.pattern_span(pattern).start;
        let annotation = if self.eat(Symbol::Colon) {
            Some(self.annotation()?)
        } else {
            None
        };
        self.expect(Symbol::Assign)?;
        self.skip_newline();
        if let Some(span) = self.assign_ahead() {
            return Err(Error {
                span,
                reason: Reason::BindingValue,
            });
        }
        let value = self.line(context)?;
        Ok(self.expression(
            ExprKind::Bind {
                doc: None,
                public,
                pattern,
                annotation,
                value,
            },
            Span {
                start,
                end: self.expr_span(value).end,
            },
        ))
    }

    fn scoped(&mut self, context: Context) -> Result<ExprId, Error> {
        let start = self.bump().span.start;
        let key = self.name(true)?;
        let value = self.operand(Context::Head)?;
        let body = self.block(context)?;
        Ok(self
            .expression(ExprKind::With { key, value, body }, self.since(start)))
    }

    pub(super) fn pipeline(
        &mut self,
        context: Context,
    ) -> Result<ExprId, Error> {
        let mut left = self.stage(context)?;
        while let Some(op) = pipe(self.peek()) {
            let operator = self.bump().span;
            let right = self.stage(context)?;
            let span = Span {
                start: self.expr_span(left).start,
                end: self.expr_span(right).end,
            };
            left = self.expression(
                ExprKind::Pipe {
                    op,
                    operator,
                    left,
                    right,
                },
                span,
            );
        }
        Ok(left)
    }

    pub(super) fn stage(&mut self, context: Context) -> Result<ExprId, Error> {
        if *self.peek() == Kind::Kw(Keyword::If) {
            return self.conditional(context);
        }
        if let Some(span) = self.arm_arrow(context) {
            if !context.arms() {
                return Err(Error {
                    span,
                    reason: Reason::ArmsNeedGroup,
                });
            }
            return self.arms();
        }
        let left = self.unary(context)?;
        if self.binary().is_some() {
            return self.binary_tail(left, 1, context);
        }
        self.space_call(left, context)
    }

    fn space_call(
        &mut self,
        callee: ExprId,
        context: Context,
    ) -> Result<ExprId, Error> {
        let start = self.expr_span(callee).start;
        let mut end = self.expr_span(callee).end;
        let mut arguments = Vec::new();
        loop {
            if self.binary().is_some() {
                return Err(self.error(Reason::OperatorInArguments));
            }
            if !argument_start(self.peek(), context) || self.here().start <= end
            {
                break;
            }
            let argument = self.argument(context, false)?;
            end = argument.span.end;
            arguments.push(argument);
        }
        if arguments.is_empty() {
            Ok(callee)
        } else {
            Ok(self.expression(
                ExprKind::Call {
                    callee,
                    arguments,
                    style: CallStyle::Space,
                },
                Span { start, end },
            ))
        }
    }

    fn binary(&mut self) -> Option<(BinaryOp, u8)> {
        let result = binary(self.peek())?;
        if result.0 == BinaryOp::Divide {
            // A slash is division only when an operand follows on this line;
            // otherwise it is the root path, as in `ls /`.
            let next = &self.tokens[self.position + 1].kind;
            if !operand_start(next, Context::Line)
                && *next != Kind::Symbol(Symbol::Not)
            {
                return None;
            }
        }
        Some(result)
    }

    fn binary_tail(
        &mut self,
        mut left: ExprId,
        minimum: u8,
        context: Context,
    ) -> Result<ExprId, Error> {
        let mut previous = None;
        while let Some((op, precedence)) = self.binary() {
            if precedence < minimum {
                break;
            }
            if precedence <= 2 && previous == Some(precedence) {
                return Err(self.error(Reason::ChainedComparison));
            }
            let operator = self.bump().span;
            self.skip_newline();
            let right = self.unary(context)?;
            let right = self.binary_tail(right, precedence + 1, context)?;
            let span = Span {
                start: self.expr_span(left).start,
                end: self.expr_span(right).end,
            };
            left = self.expression(
                ExprKind::Binary {
                    op,
                    operator,
                    left,
                    right,
                },
                span,
            );
            previous = Some(precedence);
        }
        Ok(left)
    }

    fn unary(&mut self, context: Context) -> Result<ExprId, Error> {
        let mut operators = Vec::new();
        while self.eat(Symbol::Not) {
            operators.push(self.previous);
            if operators.len() + self.depth >= super::MAX_NESTING {
                return Err(self.error(Reason::NestingLimit));
            }
        }
        let mut value = self.operand(context)?;
        for operator in operators.into_iter().rev() {
            let span = Span {
                start: operator.start,
                end: self.expr_span(value).end,
            };
            value = self.expression(ExprKind::Not { operator, value }, span);
        }
        Ok(value)
    }

    pub(super) fn operand(
        &mut self,
        context: Context,
    ) -> Result<ExprId, Error> {
        let mut value = self.nested(|parser| parser.primary(context))?;
        loop {
            let span = self.expr_span(value);
            if self.here().start != span.end {
                break;
            }
            match self.peek() {
                Kind::Symbol(Symbol::LParen) => value = self.call(value)?,
                Kind::Selector(_) => {
                    let token = self.bump();
                    let Kind::Selector(segments) = token.kind else {
                        unreachable!("selector was checked before advancing")
                    };
                    value = self.expression(
                        ExprKind::Field {
                            value,
                            field: Name {
                                segments,
                                span: token.span,
                            },
                        },
                        Span {
                            start: span.start,
                            end: token.span.end,
                        },
                    );
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn primary(&mut self, context: Context) -> Result<ExprId, Error> {
        match self.peek() {
            Kind::Symbol(Symbol::LParen) => return self.group(),
            Kind::Symbol(Symbol::LBracket) => return self.bracket(),
            Kind::Symbol(Symbol::LBrace) if !context.body() => {
                return self.block(context);
            }
            Kind::Name(_) => {
                let name = self.name(false)?;
                // In an `if` or `with` head the first top-level `{` is
                // the body, so no record opens there (02-syntax §Heads).
                if name.is_uppercase()
                    && self.at(Symbol::LBrace)
                    && !context.body()
                {
                    return self.record(name);
                }
                if self.at(Symbol::Less) && self.here().start == name.span.end {
                    return self.instance(name);
                }
                let span = name.span;
                return Ok(self.expression(ExprKind::Name(name), span));
            }
            kind if !operand_start(kind, context) => {
                return Err(self.expected(Expected::Expression));
            }
            _ => {}
        }
        let token = self.bump();
        let kind = match token.kind {
            Kind::Word(text) => ExprKind::Word(text),
            Kind::Path(text) => ExprKind::Path(text),
            Kind::Selector(segments) => ExprKind::Selector(Name {
                segments,
                span: token.span,
            }),
            Kind::Symbol(Symbol::Slash) => ExprKind::Path(String::from("/")),
            kind => ExprKind::Literal(literal(kind).ok_or(Error {
                span: token.span,
                reason: Reason::Expected(Expected::Expression),
            })?),
        };
        Ok(self.expression(kind, token.span))
    }

    fn group(&mut self) -> Result<ExprId, Error> {
        let start = self.bump().span.start;
        self.newlines();
        if self.at(Symbol::RParen) {
            self.bump();
            return Ok(self.expression(
                ExprKind::Literal(Literal::Unit),
                self.since(start),
            ));
        }
        if self.arm_arrow(Context::Line).is_some() {
            return self.arm_list(start, Symbol::RParen);
        }
        if self.tuple_at(self.position) {
            let parts =
                self.tuple(|parser| parser.documented_line(Context::Element))?;
            return Ok(
                self.expression(ExprKind::Tuple(parts), self.since(start))
            );
        }
        let lines = self.lines(Symbol::RParen)?;
        Ok(self.expression(ExprKind::Group(lines), self.since(start)))
    }

    /// `[`: an outcome bracket when `:` or `!` follows, otherwise a list.
    fn bracket(&mut self) -> Result<ExprId, Error> {
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
                Some(self.line(Context::Element)?)
            };
            self.expect(Symbol::RBracket)?;
            return Ok(self.expression(
                ExprKind::Outcome { good, payload },
                self.since(start),
            ));
        }
        let elements = self.list(Symbol::RBracket, |parser| {
            parser.documented_line(Context::Element)
        })?;
        Ok(self.expression(ExprKind::List(elements), self.since(start)))
    }

    /// Raw construction after an uppercase name; `{ x }` puns the field
    /// `x` with the value named `x`.
    fn record(&mut self, name: Name) -> Result<ExprId, Error> {
        let start = name.span.start;
        self.expect(Symbol::LBrace)?;
        let fields = self.list(Symbol::RBrace, |parser| {
            let name = parser.name(true)?;
            let value = if parser.eat(Symbol::Colon) {
                parser.line(Context::Element)?
            } else {
                parser.expression(ExprKind::Name(name.clone()), name.span)
            };
            let span = Span {
                start: name.span.start,
                end: parser.expr_span(value).end,
            };
            Ok(FieldValue { name, value, span })
        })?;
        Ok(self
            .expression(ExprKind::Record { name, fields }, self.since(start)))
    }

    fn call(&mut self, callee: ExprId) -> Result<ExprId, Error> {
        self.expect(Symbol::LParen)?;
        let arguments = self.list(Symbol::RParen, |parser| {
            parser.argument(Context::Element, true)
        })?;
        let span = self.since(self.expr_span(callee).start);
        Ok(self.expression(
            ExprKind::Call {
                callee,
                arguments,
                style: CallStyle::Parenthesized,
            },
            span,
        ))
    }

    fn argument(
        &mut self,
        context: Context,
        full: bool,
    ) -> Result<Argument, Error> {
        if matches!(self.peek(), Kind::Flag(_)) {
            return self.flag(context);
        }
        let value = if full {
            self.documented_line(context)?
        } else {
            self.operand(context)?
        };
        Ok(Argument {
            kind: ArgumentKind::Positional(value),
            span: self.expr_span(value),
        })
    }

    fn flag(&mut self, context: Context) -> Result<Argument, Error> {
        let token = self.bump();
        let Kind::Flag(name) = token.kind else {
            unreachable!("flag was checked before advancing")
        };
        let mut end = token.span.end;
        let value = if self.eat(Symbol::Assign) {
            let equals = self.previous;
            if equals.start != end || self.here().start != equals.end {
                return Err(Error {
                    span: equals,
                    reason: Reason::FlagValueMustTouch,
                });
            }
            let value = self.operand(context)?;
            end = self.expr_span(value).end;
            Some(value)
        } else {
            None
        };
        Ok(Argument {
            kind: ArgumentKind::Flag { name, value },
            span: Span {
                start: token.span.start,
                end,
            },
        })
    }

    fn conditional(&mut self, context: Context) -> Result<ExprId, Error> {
        self.nested(|parser| parser.if_expression(context))
    }

    fn if_expression(&mut self, context: Context) -> Result<ExprId, Error> {
        let start = self.bump().span.start;
        let condition = self.stage(Context::Head)?;
        let then_body = self.block(context)?;
        let otherwise = self.tokens[self.position..]
            .iter()
            .position(|token| token.kind != Kind::Newline);
        let has_else = otherwise.is_some_and(|offset| {
            self.tokens[self.position + offset].kind == Kind::Kw(Keyword::Else)
        });
        let else_body = if has_else {
            self.newlines();
            self.bump();
            if *self.peek() == Kind::Kw(Keyword::If) {
                Some(self.conditional(context)?)
            } else {
                Some(self.block(context)?)
            }
        } else {
            None
        };
        Ok(self.expression(
            ExprKind::If {
                condition,
                then_body,
                else_body,
            },
            self.since(start),
        ))
    }

    /// A braced body: an arm list when its first line has a top-level `=>`,
    /// otherwise a block of lines.
    pub(super) fn block(&mut self, _context: Context) -> Result<ExprId, Error> {
        self.nested(|parser| {
            let start = parser.expect(Symbol::LBrace)?.start;
            parser.newlines();
            if parser.arm_arrow(Context::Line).is_some() {
                return parser.arm_list(start, Symbol::RBrace);
            }
            let lines = parser.lines(Symbol::RBrace)?;
            Ok(parser.expression(ExprKind::Block(lines), parser.since(start)))
        })
    }

    /// Lines up to a closing bracket, each ended by a newline. Errors in
    /// one line are recorded and parsing resumes at the next line.
    fn lines(&mut self, close: Symbol) -> Result<Vec<ExprId>, Error> {
        let mut lines = Vec::new();
        while !self.at(close) && *self.peek() != Kind::Eof {
            let result = self.documented_line(Context::Line).and_then(|line| {
                // A stray closer inside the bracket wants this one.
                if matches!(
                    self.peek(),
                    Kind::Symbol(
                        Symbol::RParen | Symbol::RBracket | Symbol::RBrace
                    )
                ) && !self.at(close)
                {
                    return Err(self.expected(Expected::Symbol(close)));
                }
                self.line_end(Some(close))?;
                Ok(line)
            });
            match result {
                Ok(line) => lines.push(line),
                Err(error) => {
                    self.errors.push(error);
                    self.recover(Some(close));
                }
            }
            self.newlines();
        }
        self.expect(close)?;
        Ok(lines)
    }
}

pub(super) fn literal(kind: Kind) -> Option<Literal> {
    Some(match kind {
        Kind::Int {
            negative,
            magnitude,
            radix,
            digits,
        } => Literal::Int {
            negative,
            magnitude,
            radix,
            digits,
        },
        Kind::Float(text) => Literal::Float(text),
        Kind::Str { value, raw } => Literal::Str { value, raw },
        Kind::Affixed(affixed) => Literal::Affixed(affixed),
        _ => return None,
    })
}

pub(super) const fn operand_start(kind: &Kind, context: Context) -> bool {
    match kind {
        Kind::Name(_)
        | Kind::Word(_)
        | Kind::Path(_)
        | Kind::Selector(_)
        | Kind::Int { .. }
        | Kind::Float(_)
        | Kind::Str { .. }
        | Kind::Affixed(_)
        | Kind::Symbol(Symbol::LParen | Symbol::LBracket | Symbol::Slash) => {
            true
        }
        Kind::Symbol(Symbol::LBrace) => !context.body(),
        _ => false,
    }
}

const fn argument_start(kind: &Kind, context: Context) -> bool {
    matches!(kind, Kind::Flag(_)) || operand_start(kind, context)
}

const fn pipe(kind: &Kind) -> Option<PipeOp> {
    Some(match kind {
        Kind::Symbol(Symbol::Pipe) => PipeOp::Plain,
        Kind::Symbol(Symbol::PipeGood) => PipeOp::Good,
        Kind::Symbol(Symbol::PipeBad) => PipeOp::Bad,
        Kind::Symbol(Symbol::Or) => PipeOp::Or,
        Kind::Symbol(Symbol::And) => PipeOp::And,
        _ => return None,
    })
}

const fn binary(kind: &Kind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        Kind::Symbol(Symbol::Eq) => (BinaryOp::Equal, 1),
        Kind::Symbol(Symbol::NotEq) => (BinaryOp::NotEqual, 1),
        Kind::Symbol(Symbol::Less) => (BinaryOp::Less, 2),
        Kind::Symbol(Symbol::LessEq) => (BinaryOp::LessEqual, 2),
        Kind::Symbol(Symbol::Greater) => (BinaryOp::Greater, 2),
        Kind::Symbol(Symbol::GreaterEq) => (BinaryOp::GreaterEqual, 2),
        Kind::Symbol(Symbol::Plus) => (BinaryOp::Add, 3),
        Kind::Symbol(Symbol::Minus) => (BinaryOp::Subtract, 3),
        Kind::Symbol(Symbol::Concat) => (BinaryOp::Concat, 3),
        Kind::Symbol(Symbol::Star) => (BinaryOp::Multiply, 4),
        Kind::Symbol(Symbol::Slash) => (BinaryOp::Divide, 4),
        Kind::Symbol(Symbol::Percent) => (BinaryOp::Remainder, 4),
        _ => return None,
    })
}
