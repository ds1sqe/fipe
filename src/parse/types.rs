use super::{Error, Expected, Parser, Reason};
use crate::{
    ast::{ExprId, ExprKind, Name, Type, TypeId, TypeKind},
    token::{Kind, Span, Symbol},
};
use alloc::vec::Vec;

impl Parser {
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
