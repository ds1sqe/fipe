use super::{Context, Error, Expected, Parser};
use crate::{
    ast::{
        ExprId, ExprKind, Field, ImplMember, Item, ItemKind, Member,
        TraitMember, TypeParameter,
    },
    token::{Keyword, Kind, Symbol},
};
use alloc::{vec, vec::Vec};

impl Parser {
    pub(super) fn item_start(&mut self) -> bool {
        matches!(
            self.peek(),
            Kind::Kw(
                Keyword::Use | Keyword::Type | Keyword::Trait | Keyword::Impl
            )
        )
    }

    pub(super) fn item(&mut self, public: bool) -> Result<Item, Error> {
        let start = self.here().start;
        let kind = match self.peek() {
            Kind::Kw(Keyword::Use) if !public => self.import()?,
            Kind::Kw(Keyword::Type) => self.type_item()?,
            Kind::Kw(Keyword::Trait) => self.trait_item()?,
            Kind::Kw(Keyword::Impl) if !public => self.impl_item()?,
            _ => return Err(self.expected(Expected::Name)),
        };
        Ok(Item {
            kind,
            public,
            span: self.since(start),
        })
    }

    /// `use x`, `use x.inc x.Point`, `use text 2`.
    fn import(&mut self) -> Result<ItemKind, Error> {
        self.bump();
        let mut names = Vec::new();
        loop {
            names.push(self.name(false)?);
            if !matches!(self.peek(), Kind::Name(_)) {
                break;
            }
        }
        let version = if matches!(self.peek(), Kind::Int { .. }) {
            Some(self.nonnegative()?.0)
        } else {
            None
        };
        Ok(ItemKind::Use { names, version })
    }

    /// `type Name<params> { members }`.
    fn type_item(&mut self) -> Result<ItemKind, Error> {
        self.bump();
        let name = self.name(true)?;
        let parameters = if self.eat(Symbol::Less) {
            self.list(Symbol::Greater, |parser| {
                let name = parser.name(true)?;
                let bound = if parser.eat(Symbol::Colon) {
                    Some(parser.name(false)?)
                } else {
                    None
                };
                let span = parser.since(name.span.start);
                Ok(TypeParameter { name, bound, span })
            })?
        } else {
            Vec::new()
        };
        self.expect(Symbol::LBrace)?;
        let members = self.list(Symbol::RBrace, Self::member)?;
        Ok(ItemKind::Type {
            name,
            parameters,
            members,
        })
    }

    /// A field when `pub` or `:` follows the name, else a constructor.
    fn member(&mut self) -> Result<Member, Error> {
        if self.at_keyword(Keyword::Pub) || self.colon_follows() {
            return self.field().map(Member::Field);
        }
        let name = self.name(true)?;
        let fields = if self.eat(Symbol::LParen) {
            self.list(Symbol::RParen, Self::field)?
        } else {
            Vec::new()
        };
        let span = self.since(name.span.start);
        Ok(Member::Constructor { name, fields, span })
    }

    /// `[pub] name: type [= operand]`.
    fn field(&mut self) -> Result<Field, Error> {
        let start = self.here().start;
        let public = self.keyword(Keyword::Pub);
        let name = self.name(true)?;
        self.expect(Symbol::Colon)?;
        let ty = self.type_syntax()?;
        let default = if self.eat(Symbol::Assign) {
            Some(self.operand(Context::Element)?)
        } else {
            None
        };
        Ok(Field {
            public,
            name,
            ty,
            default,
            span: self.since(start),
        })
    }

    /// `trait Name: Parent { name: type [= value], … }`.
    fn trait_item(&mut self) -> Result<ItemKind, Error> {
        self.bump();
        let name = self.name(true)?;
        let parent = if self.eat(Symbol::Colon) {
            Some(self.name(false)?)
        } else {
            None
        };
        self.expect(Symbol::LBrace)?;
        let members = self.list(Symbol::RBrace, |parser| {
            let name = parser.name(true)?;
            parser.expect(Symbol::Colon)?;
            let ty = parser.type_syntax()?;
            let default = if parser.eat(Symbol::Assign) {
                Some(parser.member_value()?)
            } else {
                None
            };
            let span = parser.since(name.span.start);
            Ok(TraitMember {
                name,
                ty,
                default,
                span,
            })
        })?;
        Ok(ItemKind::Trait {
            name,
            parent,
            members,
        })
    }

    /// `impl Target [: Trait] { members }`: named members, and bare arms
    /// that are the written constructor.
    fn impl_item(&mut self) -> Result<ItemKind, Error> {
        self.bump();
        let target = self.type_syntax()?;
        let implements = if self.eat(Symbol::Colon) {
            Some(self.type_syntax()?)
        } else {
            None
        };
        self.expect(Symbol::LBrace)?;
        let members = self.list(Symbol::RBrace, |parser| {
            if parser.arm_arrow(Context::Line).is_some() {
                return parser.arm().map(ImplMember::Constructor);
            }
            let start = parser.here().start;
            let public = parser.keyword(Keyword::Pub);
            let name = parser.name(true)?;
            let annotation = if parser.eat(Symbol::Colon) {
                Some(parser.type_syntax()?)
            } else {
                None
            };
            parser.expect(Symbol::Assign)?;
            let value = parser.member_value()?;
            Ok(ImplMember::Named {
                public,
                name,
                annotation,
                value,
                span: parser.since(start),
            })
        })?;
        Ok(ItemKind::Impl {
            target,
            implements,
            members,
        })
    }

    /// A member's value inside a comma list: one arm, or a non-arm line.
    /// Several arms take their own brackets, as in any list element.
    fn member_value(&mut self) -> Result<ExprId, Error> {
        if self.arm_arrow(Context::Line).is_some() {
            let arm = self.arm()?;
            let span = arm.span;
            return Ok(self.expression(ExprKind::Arms(vec![arm]), span));
        }
        self.line(Context::Element)
    }

    fn colon_follows(&mut self) -> bool {
        self.peek();
        matches!(self.tokens[self.position].kind, Kind::Name(_))
            && self
                .tokens
                .get(self.position + 1)
                .is_some_and(|token| token.kind == Kind::Symbol(Symbol::Colon))
    }

    fn at_keyword(&mut self, keyword: Keyword) -> bool {
        *self.peek() == Kind::Kw(keyword)
    }
}
