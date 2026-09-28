use super::{Context, Error, Expected, Parser, Reason};
use crate::{
    ast::{Constraint, ExprId, ExprKind, Name, Type, TypeId, TypeKind},
    token::{Kind, Span, Symbol},
};
use alloc::vec::Vec;

impl Parser {
    /// The type of an annotation: `Show a, Num b => type`, or a type.
    /// Constraints live only here, at the top; a nested type has none.
    pub(super) fn annotation(&mut self) -> Result<TypeId, Error> {
        if !self.constraints_ahead() {
            return self.type_syntax();
        }
        let start = self.here().start;
        let mut constraints = Vec::new();
        loop {
            let class = self.type_atom()?;
            let variable = self.name(true)?;
            let span = Span {
                start: self.module.types[class.0].span.start,
                end: variable.span.end,
            };
            constraints.push(Constraint {
                class,
                variable,
                span,
            });
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
    /// An annotation ends at `=`, `:`, a newline, a pipe, a brace or an
    /// unmatched closer, so a later member's `=>` is never taken.
    fn constraints_ahead(&mut self) -> bool {
        self.peek();
        let mut depth = 0_usize;
        for token in &self.tokens[self.position..] {
            match token.kind {
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
                        Symbol::Assign
                            | Symbol::Colon
                            | Symbol::LBrace
                            | Symbol::RBrace
                    );
                    if ends || symbol.continues_line() {
                        return false;
                    }
                }
                Kind::Newline | Kind::Eof if depth == 0 => return false,
                _ => {}
            }
        }
        false
    }

    /// A type: an atom, or `atom -> type`, which nests to the right.
    pub(super) fn type_syntax(&mut self) -> Result<TypeId, Error> {
        self.nested(|parser| {
            let start = parser.here().start;
            let parameter = parser.type_atom()?;
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
                let (length, length_span) = self.nonnegative()?;
                self.expect(Symbol::RBracket)?;
                TypeKind::Array {
                    element,
                    length,
                    length_span,
                }
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

    /// The list after an opening `<`; at least one type, closed by `>`.
    pub(super) fn type_arguments(&mut self) -> Result<Vec<TypeId>, Error> {
        let arguments = self.list(Symbol::Greater, Self::type_syntax)?;
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
