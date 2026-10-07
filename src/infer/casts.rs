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
        self.state.casts[id.0] = Some(Cast { row, from, to });
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
        if !self.cast_target(&target) {
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
            }
            return target;
        }
        // ponytail: snapshots cost O(state); use an undo log if place checking dominates.
        let before = self.state.clone();
        let ty = self.check_expr(id, Some(expected));
        self.dots();
        if self.state.errors.len() == before.errors.len()
            || self.state.errors[before.errors.len()..]
                .iter()
                .any(|e| matches!(e.reason, Reason::OutOfRange))
        {
            return ty;
        }
        let failed = core::mem::replace(&mut self.state, before);
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
                if let crate::ast::ExprKind::Literal(literal) =
                    &self.module.expressions[id.0].kind
                {
                    let form = match literal {
                        crate::ast::Literal::Int {
                            negative,
                            magnitude,
                        } => Some(super::Lit::Int {
                            negative: *negative,
                            magnitude: magnitude.clone(),
                        }),
                        crate::ast::Literal::Float(_) => {
                            Some(super::Lit::Float)
                        }
                        _ => None,
                    };
                    if let Some(form) = form {
                        self.check_literal(
                            form,
                            &target,
                            self.span(id),
                            expected.span,
                        );
                    }
                }
            }
            self.record_cast(id, row, source, target.clone());
            self.state.entries.push((
                id,
                target.clone(),
                expected.span,
                expected.place_kind.clone(),
            ));
            return target;
        }
        if self.state.errors[errors..]
            .iter()
            .any(|e| matches!(e.reason, Reason::OutcomeText { .. }))
        {
            return target;
        }
        self.state = failed;
        ty
    }
}
