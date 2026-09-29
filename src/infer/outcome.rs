//! Outcomes (types.md §Outcome types; evaluation.md §Marked pipes,
//! §Fallback and sequencing): brackets take their family from the
//! expected type, `|:` and `|!` hand the selected side's payload to the
//! stage, except `|!` on `Maybe`, whose stage is a plain value; `||` and
//! `&&` work on `Bool`, `Maybe` and `Result` only.

use super::{prelude_of, Expected, Infer, Reason};
use crate::{ast::ExprId, resolve::Origin, token::Span, types::Type};
use alloc::vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Maybe,
    Result,
    Either,
}

/// The outcome family of a type, with its arguments.
pub(super) fn family(ty: &Type) -> Option<(Family, &[Type])> {
    let Type::Named { name, arguments } = ty else {
        return None;
    };
    if name.origin != Origin::Prelude {
        return None;
    }
    let family = match (name.name.as_str(), arguments.len()) {
        ("Maybe", 1) => Family::Maybe,
        ("Result", 2) => Family::Result,
        ("Either", 2) => Family::Either,
        _ => return None,
    };
    Some((family, arguments))
}

/// A side's payload; `Maybe`'s bad side carries none, typed unit for
/// the bracket `[!]`.
fn payload(family: Family, arguments: &[Type], good: bool) -> Type {
    match (family, good) {
        (Family::Maybe, false) => Type::Unit,
        (Family::Maybe | Family::Result, true) | (Family::Either, false) => {
            arguments[0].clone()
        }
        (Family::Result, false) | (Family::Either, true) => {
            arguments[1].clone()
        }
    }
}

/// `family<…>` with the selected side's payload replaced.
fn with_side(family: Family, arguments: &[Type], good: bool, ty: Type) -> Type {
    let mut arguments = arguments.to_vec();
    match (family, good) {
        (Family::Maybe, false) => {}
        (Family::Maybe | Family::Result, true) | (Family::Either, false) => {
            arguments[0] = ty;
        }
        (Family::Result, false) | (Family::Either, true) => arguments[1] = ty,
    }
    let name = match family {
        Family::Maybe => "Maybe",
        Family::Result => "Result",
        Family::Either => "Either",
    };
    prelude_of(name, arguments)
}

impl Infer<'_> {
    /// The payload type a bracket binds against `applied`; `None` after
    /// reporting why there is none.
    pub(super) fn bracket_side(
        &mut self,
        applied: &Type,
        good: bool,
        has_payload: bool,
        span: Span,
        other: Span,
    ) -> Option<Type> {
        match family(applied) {
            None => {
                let reason = if matches!(applied, Type::Var(_)) {
                    Reason::UnknownFamily
                } else {
                    Reason::NeedsMaybeOrResult
                };
                self.error(span, reason);
                None
            }
            Some((Family::Either, _)) => {
                self.error(span, Reason::NeedsMaybeOrResult);
                None
            }
            Some((Family::Maybe, arguments)) if !good && has_payload => {
                let actual = prelude_of(
                    "Result",
                    vec![arguments[0].clone(), self.fresh()],
                );
                let expected = applied.clone();
                self.error(
                    span,
                    Reason::Mismatch {
                        expected,
                        actual,
                        other,
                    },
                );
                None
            }
            Some((family, arguments)) => Some(payload(family, arguments, good)),
        }
    }

    /// `[:e]`, `[!e]` as a value: the family is the expected type's.
    pub(super) fn bracket(
        &mut self,
        good: bool,
        payload: Option<ExprId>,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        let applied = expected.map(|e| (self.apply(&e.ty), e.span));
        let side = if let Some((ty, other)) = &applied {
            self.bracket_side(ty, good, payload.is_some(), span, *other)
        } else {
            self.error(span, Reason::UnknownFamily);
            None
        };
        match (payload, &side) {
            (Some(payload), Some(ty)) => {
                self.expr(payload, Some(&Expected::new(ty.clone(), span)));
            }
            (Some(payload), None) => {
                self.expr(payload, None);
            }
            (None, Some(ty)) => self.unify(ty, &Type::Unit, span, span),
            (None, None) => {}
        }
        match (side, applied) {
            (Some(_), Some((ty, _))) => ty,
            _ => self.fresh(),
        }
    }

    /// `left |: right` and `left |! right`.
    pub(super) fn marked(
        &mut self,
        good: bool,
        left: ExprId,
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_ty = self.expr(left, None);
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        let Some((family, arguments)) = family(&applied) else {
            self.not_outcome(&applied, operator);
            let input = self.fresh();
            self.stage(right, input, left_span, None);
            return self.fresh();
        };
        if family == Family::Maybe && !good {
            return self.miss(&applied, right, left_span);
        }
        let input = payload(family, arguments, good);
        let result = self.stage(right, input, left_span, None);
        self.rebuild(&applied, good, result, self.span(right), left_span)
    }

    /// `maybe |! stage`: the stage is a value evaluated on the miss; a
    /// stage with an open spot is an error.
    fn miss(&mut self, left: &Type, right: ExprId, left_span: Span) -> Type {
        let span = self.span(right);
        let result = self.expr(right, None);
        if matches!(self.apply(&result), Type::Function { .. }) {
            self.error(span, Reason::OpenSpot);
            return left.clone();
        }
        self.rebuild(left, false, result, span, left_span)
    }

    fn not_outcome(&mut self, applied: &Type, operator: Span) {
        let reason = if matches!(applied, Type::Var(_)) {
            Reason::UnknownFamily
        } else {
            Reason::NeedsMaybeOrResult
        };
        self.error(operator, reason);
    }

    /// The marked-pipe result: a return of the same family is used
    /// directly, and preferred when both readings unify; a plain one is
    /// wrapped in the selected side. `Maybe` discards a plain `|!`
    /// result; `Result` needs it to implement `Err`.
    pub(super) fn rebuild(
        &mut self,
        left: &Type,
        good: bool,
        result: Type,
        span: Span,
        other: Span,
    ) -> Type {
        let Some((family, arguments)) = family(left) else {
            return result;
        };
        let applied = self.apply(&result);
        let same = match &applied {
            Type::Var(_) => !self.numeric_var(&applied),

            ty => super::outcome::family(ty).is_some_and(|(f, _)| f == family),
        };
        if same {
            let fresh = self.fresh();
            let shape = with_side(family, arguments, good, fresh);
            self.unify(&shape, &result, span, other);
            return shape;
        }
        if family == Family::Result && !good {
            self.obligation("Err", result.clone(), span);
        }
        with_side(family, arguments, good, result)
    }

    /// `left || right`: `Bool`, or the fallback of a `Maybe` or
    /// `Result`: the same family first, else its good payload.
    pub(super) fn or(
        &mut self,
        left: ExprId,
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_ty = self.expr(left, None);
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        if super::is_prelude(&applied, "Bool") {
            self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
            return applied;
        }
        let Some((family @ (Family::Maybe | Family::Result), arguments)) =
            family(&applied)
        else {
            self.not_outcome(&applied, operator);
            self.expr(right, None);
            return self.fresh();
        };
        let good = payload(family, arguments, true);
        let before = self.state.clone();
        self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
        if self.state.errors.len() == before.errors.len() {
            return applied;
        }
        self.state = before;
        self.expr(right, Some(&Expected::new(good.clone(), left_span)));
        good
    }

    /// `left && right` is `left |: (_ => right)`.
    pub(super) fn and(
        &mut self,
        left: ExprId,
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_ty = self.expr(left, None);
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        if super::is_prelude(&applied, "Bool") {
            self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
            return applied;
        }
        if !matches!(
            family(&applied),
            Some((Family::Maybe | Family::Result, _))
        ) {
            self.not_outcome(&applied, operator);
            self.expr(right, None);
            return self.fresh();
        }
        let result = self.expr(right, None);
        self.rebuild(&applied, true, result, self.span(right), left_span)
    }
}
