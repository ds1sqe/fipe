//! The closed cast table, consulted only at known places.
use super::{meaning, prelude, Expected, Infer, Reason};
use crate::{
    ast::ExprId,
    resolve::Origin,
    token::Span,
    typed::{Cast, CastRow},
    types::Type,
};

#[derive(Clone, Copy)]
enum Meaning {
    Number,
    Read,
    Text,
    Host,
    Bytes,
    Packed,
    String,
    Show,
}

// Exact bytes precede the overlapping Show row. No row feeds another.
const TABLE: [(CastRow, Meaning, Meaning); 6] = [
    (CastRow::Number, Meaning::Number, Meaning::Number),
    (CastRow::Read, Meaning::Read, Meaning::String),
    (CastRow::Bytes, Meaning::Bytes, Meaning::String),
    (CastRow::Show, Meaning::Text, Meaning::Show),
    (CastRow::Codec, Meaning::Host, Meaning::Bytes),
    (CastRow::Pack, Meaning::Packed, Meaning::Packed),
];

/// The literal an expression is, through any groups around it: a literal
/// is checked at compile time however it is parenthesized.
pub(super) fn grouped_literal(
    module: &crate::ast::Module,
    mut id: ExprId,
) -> Option<&crate::ast::Literal> {
    loop {
        match &module.expressions[id.0].kind {
            crate::ast::ExprKind::Literal(literal) => return Some(literal),
            crate::ast::ExprKind::Group(parts) if parts.len() == 1 => {
                id = parts[0];
            }
            _ => return None,
        }
    }
}

impl Infer<'_> {
    fn cast_meaning(&mut self, ty: &Type, class: Meaning) -> bool {
        let ty = meaning(ty);
        match class {
            Meaning::Number => ["Integer", "F32", "F64"]
                .iter()
                .any(|n| super::is_prelude(ty, n)),
            Meaning::Read => self.has_claim(ty, &prelude("Read")),
            Meaning::String => super::is_prelude(ty, "String"),
            Meaning::Text => {
                super::is_prelude(ty, "String")
                    || self.cast_meaning(ty, Meaning::Bytes)
            }
            Meaning::Bytes => {
                matches!(ty, Type::Named { name, arguments } if name.origin == Origin::Prelude && name.name == "Vec" && arguments.len() == 1 && (super::is_prelude(&arguments[0], "U8") || matches!(&arguments[0], Type::Limited { name, .. } if super::is_prelude(name, "U8"))))
            }
            Meaning::Show => self.has_claim(ty, &prelude("Show")),
            Meaning::Host => {
                matches!(ty, Type::Named { name, .. } if matches!(name.origin, Origin::Host(_) | Origin::Commands))
            }
            Meaning::Packed => self.packed(ty),
        }
    }

    fn cast_target(&mut self, target: &Type) -> bool {
        if matches!(meaning(target), Type::Var(_) | Type::Rigid(_)) {
            return false;
        }
        TABLE
            .iter()
            .any(|(_, to, _)| self.cast_meaning(target, *to))
    }

    pub(super) fn cast_pair(
        &mut self,
        target: &Type,
        source: &Type,
        span: Span,
        other: Span,
    ) -> Option<CastRow> {
        if meaning(target) == meaning(source) {
            return None;
        }
        if self.cast_meaning(target, Meaning::Text)
            && super::outcome::family(meaning(source)).is_some()
        {
            self.error(
                span,
                Reason::OutcomeText {
                    family: source.clone(),
                },
            );
            return None;
        }
        for (row, to, from) in TABLE {
            if !self.cast_meaning(target, to) {
                continue;
            }
            if row == CastRow::Pack {
                let errors = self.state.errors.len();
                self.pack(target, source, span, other);
                return (self.state.errors.len() == errors).then_some(row);
            }
            if self.cast_meaning(source, from) {
                if row == CastRow::Show {
                    self.require_claim(source.clone(), prelude("Show"), span);
                    self.dots();
                }
                return Some(row);
            }
        }
        None
    }

    pub(super) fn record_cast(
        &mut self,
        id: ExprId,
        row: CastRow,
        from: Type,
        to: Type,
    ) {
        self.set_cast(
            id.0,
            Cast {
                operation: None,
                row,
                from,
                to,
            },
        );
    }

    pub(super) fn record_cast_operations(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let casts: alloc::vec::Vec<_> = self
            .state
            .casts
            .iter()
            .enumerate()
            .filter_map(|(index, cast)| {
                Some((index, (**cast.as_ref()?).clone()))
            })
            .collect();
        for (index, cast) in casts {
            let (head, name, result, receiverless) = match cast.row {
                CastRow::Read => (
                    self.apply(meaning(&cast.to)),
                    "read",
                    super::prelude_of(
                        "Result",
                        alloc::vec![
                            self.apply(meaning(&cast.to)),
                            prelude("ParseError")
                        ],
                    ),
                    true,
                ),
                CastRow::Show => (
                    self.apply(meaning(&cast.from)),
                    "show",
                    prelude("String"),
                    false,
                ),
                _ => continue,
            };
            let span = self.span(ExprId(index));
            let callee = self.cast_operation(&head, name, receiverless, span);
            let expected = Type::function(
                if receiverless {
                    prelude("String")
                } else {
                    head.clone()
                },
                result,
            );
            let operation = if let Some(callee) = callee {
                self.unify(&callee.whole(), &expected, span, span);
                crate::typed::Comparison {
                    selected: callee
                        .selected
                        .unwrap_or_else(|| crate::typed::Selected::Member {
                            head: head.clone(),
                            name: name.into(),
                            argument: None,
                        })
                        .applied(&self.state.subst),
                    meanings: callee
                        .meanings
                        .iter()
                        .map(|ty| self.apply(ty))
                        .collect(),
                    ty: self.apply(&expected),
                    self_slot: callee.self_slot.unwrap_or(0),
                }
            } else {
                crate::typed::Comparison {
                    selected: if receiverless {
                        crate::typed::Selected::Static {
                            head,
                            name: name.into(),
                        }
                    } else {
                        crate::typed::Selected::Member {
                            head,
                            name: name.into(),
                            argument: None,
                        }
                    },
                    meanings: alloc::vec::Vec::new(),
                    ty: expected,
                    self_slot: 0,
                }
            };
            self.state.casts[index]
                .as_mut()
                .expect("recorded cast")
                .operation = Some(operation);
        }
    }

    fn cast_operation(
        &mut self,
        head: &Type,
        name: &str,
        receiverless: bool,
        span: Span,
    ) -> Option<super::Callee> {
        // The lookup selects at the cast's span; the expression's own
        // selection stands.
        let lookup = self.snapshot();
        let callee = if receiverless {
            if let Type::Named { name: owner, .. } = head {
                Some(self.type_member(
                    &crate::resolve::Target::Type {
                        origin: owner.origin.clone(),
                        name: owner.name.clone(),
                    },
                    name,
                    span,
                ))
            } else {
                None
            }
        } else {
            self.self_function(name, head, span)
        };
        self.keep_except(lookup, super::trial::Field::Selection);
        callee
    }

    fn same_local_meaning(&self, id: ExprId, target: &Type) -> bool {
        let Some(crate::resolve::Target::Local { site }) =
            self.resolved.expr(id)
        else {
            return false;
        };
        self.state
            .locals
            .get(&(site.start, site.end))
            .or_else(|| {
                self.frame
                    .and_then(|f| f.locals.get(&(site.start, site.end)))
            })
            .is_some_and(|local| {
                local.scheme.variables.is_empty()
                    && meaning(&self.apply(&local.scheme.ty)) == meaning(target)
            })
    }

    pub(super) fn expr(
        &mut self,
        id: ExprId,
        expected: Option<&Expected>,
    ) -> Type {
        let Some(expected) = expected.filter(|e| e.cast || e.spot) else {
            return self.check_expr(id, expected);
        };
        let target = self.apply(&expected.ty);
        let literal = grouped_literal(self.module, id);
        if self.same_local_meaning(id, &target)
            || !self.cast_target(&target)
            || (matches!(
                literal,
                Some(
                    crate::ast::Literal::Int { .. }
                        | crate::ast::Literal::Float(_)
                )
            ) && self.cast_meaning(&target, Meaning::Number))
        {
            return self.check_expr(id, Some(expected));
        }
        if self.packed(&target) {
            let source = self.check_expr(id, None);
            self.default_entering(&source);
            let source = self.apply(&source);
            if let Some(row) =
                self.cast_pair(&target, &source, self.span(id), expected.span)
            {
                self.record_cast(id, row, source, target.clone());
                self.state.entries.push((
                    id,
                    target.clone(),
                    expected.span,
                    expected.place_kind.clone(),
                    false,
                ));
            }
            return target;
        }
        let errors = self.state.errors.len();
        let first = self.snapshot();
        let ty = self.check_expr(id, Some(expected));
        self.dots();
        if self.state.errors.len() == errors
            || self.state.errors[errors..]
                .iter()
                .any(|e| super::bind::decided_literal(&e.reason))
        {
            self.keep(first);
            return ty;
        }
        self.restore(first);
        let second = self.snapshot();
        let source = self.check_expr(id, None);
        self.default_entering(&source);
        self.dots();
        let source = self.apply(&source);
        let errors = self.state.errors.len();
        if let Some(row) =
            self.cast_pair(&target, &source, self.span(id), expected.span)
        {
            // Numeric literal exactness is decided by the existing literal checker.
            if row == CastRow::Number {
                if let Some(form) = literal.and_then(super::Lit::of) {
                    self.check_literal(
                        form,
                        &target,
                        self.span(id),
                        expected.span,
                    );
                }
            }
            self.record_cast(id, row, source, target.clone());
            self.state.entries.push((
                id,
                target.clone(),
                expected.span,
                expected.place_kind.clone(),
                false,
            ));
            self.keep(second);
            return target;
        }
        if self.state.errors[errors..]
            .iter()
            .any(|e| matches!(e.reason, Reason::OutcomeText { .. }))
        {
            self.keep(second);
            return target;
        }
        // Neither reading fits: the first, against the place, reports.
        self.restore(second);
        let ty = self.check_expr(id, Some(expected));
        self.dots();
        ty
    }
}
