use super::{Context, Error, Expected, Parser, Reason};
use crate::{
    ast::{
        Constraint, ConstraintKind, ExprId, ExprKind, Name, Type, TypeId,
        TypeKind,
    },
    token::{Kind, Span, Symbol},
};
use alloc::{vec, vec::Vec};

impl Parser {
    /// The type of an annotation: `Show a, b.age: c => type`, or a type.
    /// Constraints live only here, at the top; a nested type has none.
    pub(super) fn annotation(&mut self) -> Result<TypeId, Error> {
        if !self.constraints_ahead() {
            return self.type_syntax();
        }
        let start = self.here().start;
        let mut constraints = Vec::new();
        loop {
            let first = self.here().start;
            let kind = if self.colon_follows() {
                let member = self.name(false)?;
                self.expect(Symbol::Colon)?;
                let ty = self.type_syntax()?;
                ConstraintKind::Member { member, ty }
            } else {
                let class = self.type_atom()?;
                let variable = self.name(true)?;
                ConstraintKind::Named { class, variable }
            };
            let span = self.since(first);
            constraints.push(Constraint { kind, span });
            if !self.eat(Symbol::Comma) {
                break;
            }
        }
        self.expect(Symbol::FatArrow)?;
        self.skip_newline();
        let ty = self.type_syntax()?;
        Ok(self.add_type(
            TypeKind::Qualified { constraints, ty },
            self.since(start),
        ))
    }

    /// A depth-zero `=>` before this annotation ends: a constraint list.
    /// An annotation ends at `=`, `:`, a newline, a pipe other than `|`,
    /// a brace or an unmatched closer, so a later member's `=>` is never
    /// taken; a `:` after a qualified name is a member constraint's.
    fn constraints_ahead(&mut self) -> bool {
        self.peek();
        let mut depth = 0_usize;
        let tokens = &self.tokens[self.position..];
        for (index, token) in tokens.iter().enumerate() {
            match token.kind {
                Kind::Symbol(Symbol::Colon) if depth == 0 => {
                    let member = index > 0
                        && matches!(&tokens[index - 1].kind,
                            Kind::Name(parts) if parts.len() > 1);
                    if !member {
                        return false;
                    }
                }
                Kind::Symbol(
                    Symbol::LParen | Symbol::LBracket | Symbol::Less,
                ) => {
                    depth += 1;
                }
                Kind::Symbol(
                    Symbol::RParen | Symbol::RBracket | Symbol::Greater,
                ) => match depth.checked_sub(1) {
                    Some(less) => depth = less,
                    None => return false,
                },
                Kind::Symbol(symbol) if depth == 0 => {
                    if symbol == Symbol::FatArrow {
                        return true;
                    }
                    let ends = matches!(
                        symbol,
                        Symbol::Assign | Symbol::LBrace | Symbol::RBrace
                    );
                    if ends
                        || (symbol.continues_line() && symbol != Symbol::Pipe)
                    {
                        return false;
                    }
                }
                Kind::Newline | Kind::Eof if depth == 0 => return false,
                _ => {}
            }
        }
        false
    }

    /// A type: signs over atoms, then `-> type`, which nests to the
    /// right. `!` binds tightest, then `&`, `|` and `*`.
    pub(super) fn type_syntax(&mut self) -> Result<TypeId, Error> {
        self.nested(|parser| {
            let start = parser.here().start;
            let parameter = parser.type_chain(
                Symbol::Star,
                Self::type_term,
                TypeKind::Tuple,
            )?;
            if !parser.eat(Symbol::Arrow) {
                return Ok(parameter);
            }
            parser.skip_newline();
            let result = parser.type_syntax()?;
            Ok(parser.add_type(
                TypeKind::Function { parameter, result },
                parser.since(start),
            ))
        })
    }

    /// `A | B`; `|!` is the error that asks for `| !`.
    fn type_term(&mut self) -> Result<TypeId, Error> {
        let term =
            self.type_chain(Symbol::Pipe, Self::type_both, TypeKind::Either)?;
        if self.at(Symbol::PipeBad) {
            return Err(self.error(Reason::EitherNot));
        }
        Ok(term)
    }

    /// `A & B`, also a type parameter's bound.
    pub(super) fn type_both(&mut self) -> Result<TypeId, Error> {
        self.type_chain(Symbol::Ampersand, Self::type_not, TypeKind::Both)
    }

    fn type_not(&mut self) -> Result<TypeId, Error> {
        if !self.eat(Symbol::Not) {
            return self.type_atom();
        }
        let start = self.previous.start;
        let inner = self.type_atom()?;
        Ok(self.add_type(TypeKind::Not(inner), self.since(start)))
    }

    /// One part, or two or more joined by `sign` into one flat node.
    fn type_chain(
        &mut self,
        sign: Symbol,
        part: fn(&mut Self) -> Result<TypeId, Error>,
        node: fn(Vec<TypeId>) -> TypeKind,
    ) -> Result<TypeId, Error> {
        let start = self.here().start;
        let first = part(self)?;
        if !self.at(sign) {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat(sign) {
            self.skip_newline();
            parts.push(part(self)?);
        }
        Ok(self.add_type(node(parts), self.since(start)))
    }

    fn type_atom(&mut self) -> Result<TypeId, Error> {
        let start = self.here().start;
        let labeled = self.label_follows();
        let kind = match self.peek() {
            Kind::Name(_) => {
                let name = self.name(false)?;
                let arguments = if self.eat(Symbol::Less) {
                    self.type_arguments()?
                } else {
                    Vec::new()
                };
                TypeKind::Named { name, arguments }
            }
            Kind::Symbol(Symbol::LParen) if labeled => {
                self.bump();
                let name = self.name(true)?;
                self.expect(Symbol::Colon)?;
                let ty = self.type_syntax()?;
                let default = if self.eat(Symbol::Assign) {
                    Some(self.operand(Context::Element)?)
                } else {
                    None
                };
                self.expect(Symbol::RParen)?;
                TypeKind::Labeled { name, ty, default }
            }
            Kind::Symbol(Symbol::LParen) => {
                self.bump();
                if self.eat(Symbol::RParen) {
                    TypeKind::Unit
                } else {
                    let inner = self.type_syntax()?;
                    self.expect(Symbol::RParen)?;
                    return Ok(inner);
                }
            }
            Kind::Symbol(Symbol::LBracket) => {
                self.bump();
                let element = self.type_syntax()?;
                self.expect(Symbol::Semicolon)?;
                let length = if matches!(self.peek(), Kind::Name(_)) {
                    let name = self.name(true)?;
                    let span = name.span;
                    let arguments = Vec::new();
                    self.add_type(TypeKind::Named { name, arguments }, span)
                } else {
                    self.number()?
                };
                self.expect(Symbol::RBracket)?;
                TypeKind::Array { element, length }
            }
            _ => return Err(self.expected(Expected::Type)),
        };
        Ok(self.add_type(kind, self.since(start)))
    }

    /// `( NAME :` opens a labeled parameter rather than a grouped type.
    fn label_follows(&self) -> bool {
        matches!(
            self.tokens.get(self.position + 1).map(|token| &token.kind),
            Some(Kind::Name(_))
        ) && self
            .tokens
            .get(self.position + 2)
            .is_some_and(|token| token.kind == Kind::Symbol(Symbol::Colon))
    }

    /// A non-negative integer in a type.
    fn number(&mut self) -> Result<TypeId, Error> {
        let (value, span) = self.nonnegative()?;
        Ok(self.add_type(TypeKind::Number(value), span))
    }

    /// The list after an opening `<`; at least one type or number,
    /// closed by `>`.
    pub(super) fn type_arguments(&mut self) -> Result<Vec<TypeId>, Error> {
        let arguments = self.list(Symbol::Greater, |parser| {
            if matches!(parser.peek(), Kind::Int { .. }) {
                parser.number()
            } else {
                parser.type_syntax()
            }
        })?;
        if arguments.is_empty() {
            return Err(Error {
                span: self.previous,
                reason: Reason::Expected(Expected::Type),
            });
        }
        Ok(arguments)
    }

    /// `name<…>` in value position: the name and `<` touch.
    pub(super) fn instance(&mut self, name: Name) -> Result<ExprId, Error> {
        let start = name.span.start;
        self.expect(Symbol::Less)?;
        let arguments = self.type_arguments()?;
        Ok(self.expression(
            ExprKind::Instance { name, arguments },
            self.since(start),
        ))
    }

    pub(super) fn add_type(&mut self, kind: TypeKind, span: Span) -> TypeId {
        let id = TypeId(self.module.types.len());
        self.module.types.push(Type { kind, span });
        id
    }
}
