//! Outcomes (types.md §Outcome types; evaluation.md §Marked pipes,
//! §Fallback and sequencing): brackets take their family from the
//! expected type, `|:` and `|!` hand the selected side's payload to the
//! stage, except `|!` on `Maybe`, whose stage is a plain value; `||` and
//! `&&` work on `Bool`, `Maybe` and `Result` only.

use super::{prelude_of, Expected, Infer, Reason};
use crate::{ast::ExprId, resolve::Origin, token::Span, types::Type};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Maybe,
    Result,
    Either,
}

/// The outcome family of a type, with its arguments.
pub(super) fn family(ty: &Type) -> Option<(Family, &[Type])> {
    let Type::Named { name, arguments } = super::meaning(ty) else {
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
        (good, has_payload, pattern): (bool, bool, bool),
        span: Span,
        other: Span,
    ) -> Option<Type> {
        if matches!(applied, Type::Var(_)) {
            let side = self.fresh();
            self.state.families.push(Pending {
                ty: applied.clone(),
                side: side.clone(),
                good,
                has_payload,
                pattern,
                span,
                other,
            });
            return Some(side);
        }
        match family(applied) {
            None | Some((Family::Either, _)) => {
                let ty = applied.clone();
                self.error(span, Reason::NoBracketForm { ty });
                None
            }
            Some((Family::Maybe, _)) if !good && has_payload => {
                self.error(span, Reason::NothingPayload);
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
        let ty = expected.map_or_else(|| self.fresh(), |e| e.ty.clone());
        let head = self.apply(&ty);
        let other = expected.map_or(span, |e| e.span);
        let side = self.bracket_side(
            &head,
            (good, payload.is_some(), false),
            span,
            other,
        );
        match (payload, &side) {
            (Some(payload), Some(ty)) => {
                self.expr(
                    payload,
                    Some(
                        &Expected::new(ty.clone(), span)
                            .at(crate::typed::Place::Payload),
                    ),
                );
            }
            (Some(payload), None) => {
                self.expr(payload, None);
            }
            // A deferred family is checked when it is known.
            (None, Some(ty)) if !matches!(head, Type::Var(_)) => {
                self.payload_present(good, ty, span);
            }
            (None, _) => {}
        }
        head
    }

    /// A bracket without a payload where the side's payload is not `()`.
    fn payload_present(&mut self, good: bool, side: &Type, span: Span) {
        match self.apply(side) {
            Type::Unit => {}
            Type::Var(_) => self.unify(side, &Type::Unit, span, span),
            ty => self.error(span, Reason::MissingPayload { good, ty }),
        }
    }

    /// `left |: right` and `left |! right`, the left already typed.
    pub(super) fn marked(
        &mut self,
        good: bool,
        (left, left_ty): (ExprId, Type),
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        let Some((family, arguments)) = family(&applied) else {
            self.not_outcome(&applied, operator, Reason::NeedsOutcome);
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

    /// A left that is no outcome: unknown while it is a variable, unless
    /// the variable is a literal's, which no outcome type is.
    fn not_outcome(&mut self, applied: &Type, operator: Span, known: Reason) {
        let reason =
            if matches!(applied, Type::Var(_)) && !self.numeric_var(applied) {
                Reason::UnknownFamily
            } else {
                known
            };
        self.error(operator, reason);
    }

    /// The marked-pipe result: a return of the same family is used
    /// directly, and preferred when both readings unify; a plain one is
    /// wrapped in the selected side. `Maybe` discards a plain `|!`
    /// result.
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
        let site = (span.start, span.end);
        if matches!(applied, Type::Var(_)) && !self.numeric_var(&applied) {
            // Which reading holds waits for the result: a stage whose
            // result a literal's meaning fixes later is plain.
            let payload = self.fresh();
            let rebuilt = with_side(family, arguments, good, payload.clone());
            self.state.rebuilding.push(Rebuilding {
                site,
                family,
                good,
                result,
                rebuilt: rebuilt.clone(),
                payload,
                span,
                other,
            });
            self.state.rebuild_types.insert(site, rebuilt.clone());
            return rebuilt;
        }
        let same =
            super::outcome::family(&applied).is_some_and(|(f, _)| f == family);
        let reading = Self::reading(family, good, same);
        let payload = self.fresh();
        let rebuilt = with_side(family, arguments, good, payload.clone());
        self.settle_rebuild(
            site,
            reading,
            (&rebuilt, &payload, &result),
            span,
            other,
        );
        self.state.rebuild_types.insert(site, rebuilt.clone());
        rebuilt
    }

    /// A return of the same family is used directly; a plain one is
    /// wrapped, or discarded by `Maybe`'s `|!`.
    const fn reading(
        family: Family,
        good: bool,
        same: bool,
    ) -> crate::typed::Rebuild {
        if same {
            crate::typed::Rebuild::Direct
        } else if matches!(family, Family::Maybe) && !good {
            crate::typed::Rebuild::Discard
        } else {
            crate::typed::Rebuild::Wrap
        }
    }

    /// Record a rebuild and unify its result with the rebuilt type, whole
    /// or as the selected payload.
    fn settle_rebuild(
        &mut self,
        site: (usize, usize),
        reading: crate::typed::Rebuild,
        (rebuilt, payload, result): (&Type, &Type, &Type),
        span: Span,
        other: Span,
    ) {
        self.insert(|s| &mut s.rebuilds, site, reading);
        match reading {
            crate::typed::Rebuild::Direct => {
                self.unify(rebuilt, result, span, other);
            }
            crate::typed::Rebuild::Wrap => {
                self.unify(payload, result, span, other);
            }
            crate::typed::Rebuild::Discard => {}
        }
    }

    /// Decide the rebuilds whose stage result is now known.
    pub(super) fn solve_rebuilds(&mut self, finish: bool) {
        if self.state.rebuilding.is_empty() {
            return;
        }
        self.journal_list(|s| &mut s.rebuilding);
        let pending = core::mem::take(&mut self.state.rebuilding);
        for owed in pending {
            let applied = self.apply(&owed.result);
            let unknown = matches!(applied, Type::Var(_));
            let numeric = unknown && self.numeric_var(&applied);
            if unknown && !numeric && !finish {
                self.state.rebuilding.push(owed);
                continue;
            }
            // E1 prefers the family reading while both unify.
            let same = if unknown {
                !numeric
            } else {
                family(&applied).is_some_and(|(f, _)| f == owed.family)
            };
            let reading = Self::reading(owed.family, owed.good, same);
            self.settle_rebuild(
                owed.site,
                reading,
                (&owed.rebuilt, &owed.payload, &owed.result),
                owed.span,
                owed.other,
            );
        }
    }

    /// Decide every pending rebuild, the unknown ones by the family
    /// reading, then resolve what that fixed.
    pub(super) fn finish_rebuilds(&mut self) {
        if self.state.rebuilding.is_empty() {
            return;
        }
        self.solve_rebuilds(true);
        self.dots();
    }

    /// `left || right`: `Bool`, or the fallback of a `Maybe` or
    /// `Result`: the same family first, else its good payload. The left
    /// is already typed.
    pub(super) fn or(
        &mut self,
        (left, left_ty): (ExprId, Type),
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        if super::is_prelude(&applied, "Bool") {
            self.insert(
                |s| &mut s.modes,
                (operator.start, operator.end),
                crate::typed::Mode::Bool,
            );
            self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
            return applied;
        }
        let Some((family @ (Family::Maybe | Family::Result), arguments)) =
            family(&applied)
        else {
            self.not_outcome(&applied, operator, Reason::NeedsMaybeOrResult);
            self.expr(right, None);
            return self.fresh();
        };
        let good = payload(family, arguments, true);
        let before = self.snapshot();
        let errors = self.state.errors.len();
        self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
        self.dots();
        if self.state.errors.len() == errors {
            self.keep(before);
            self.insert(
                |s| &mut s.modes,
                (operator.start, operator.end),
                crate::typed::Mode::Family,
            );
            return applied;
        }
        self.restore(before);
        let second = self.snapshot();
        self.insert(
            |s| &mut s.modes,
            (operator.start, operator.end),
            crate::typed::Mode::Payload,
        );
        let errors = self.state.errors.len();
        self.expr(
            right,
            Some(
                &Expected::new(good.clone(), left_span)
                    .at(crate::typed::Place::Payload),
            ),
        );
        if self.state.errors.len() == errors {
            self.keep(second);
            return good;
        }
        // A right of another family is a clash of families, not a
        // mismatch inside the payload.
        let right_ty =
            self.state.types[right.0].clone().map(|t| self.apply(&t));
        let other = right_ty
            .as_ref()
            .and_then(|t| self::family(t).map(|(f, _)| f));
        if other.is_some_and(|other| other != family) {
            self.restore(second);
            let right_ty = self.expr(right, None);
            let right_ty = self.apply(&right_ty);
            self.error(
                operator,
                Reason::FamilyClash {
                    left: applied.clone(),
                    right: right_ty,
                },
            );
            return self.fresh();
        }
        self.keep(second);
        good
    }

    /// `left && right` is `left |: (_ => right)`, the left already
    /// typed.
    pub(super) fn and(
        &mut self,
        (left, left_ty): (ExprId, Type),
        right: ExprId,
        operator: Span,
    ) -> Type {
        let left_span = self.span(left);
        let applied = self.apply(&left_ty);
        if super::is_prelude(&applied, "Bool") {
            self.insert(
                |s| &mut s.modes,
                (operator.start, operator.end),
                crate::typed::Mode::Bool,
            );
            self.expr(right, Some(&Expected::new(applied.clone(), left_span)));
            return applied;
        }
        if !matches!(
            family(&applied),
            Some((Family::Maybe | Family::Result, _))
        ) {
            self.not_outcome(&applied, operator, Reason::NeedsMaybeOrResult);
            self.expr(right, None);
            return self.fresh();
        }
        let result = self.expr(right, None);
        self.insert(
            |s| &mut s.modes,
            (operator.start, operator.end),
            crate::typed::Mode::Family,
        );
        self.rebuild(&applied, true, result, self.span(right), left_span)
    }
}

#[derive(Clone)]
pub(super) struct Pending {
    ty: Type,
    side: Type,
    good: bool,
    has_payload: bool,
    /// A pattern's bracket matches its side whole; a value's bracket
    /// without a payload needs the side to be `()`.
    pattern: bool,
    span: Span,
    other: Span,
}

/// A marked-pipe rebuild waiting for its stage's result type.
#[derive(Clone)]
pub(super) struct Rebuilding {
    site: (usize, usize),
    family: Family,
    good: bool,
    result: Type,
    /// The outcome rebuilt, its selected side's payload `payload`.
    rebuilt: Type,
    payload: Type,
    span: Span,
    other: Span,
}

impl Infer<'_> {
    pub(super) fn solve_families(&mut self, finish: bool) {
        self.journal_list(|s| &mut s.families);
        let pending = core::mem::take(&mut self.state.families);
        for owed in pending {
            let ty = self.apply(&owed.ty);
            if matches!(ty, Type::Var(_)) {
                if finish {
                    self.error(owed.span, Reason::UnknownFamily);
                } else {
                    self.state.families.push(owed);
                }
            } else if let Some(side) = self.bracket_side(
                &ty,
                (owed.good, owed.has_payload, owed.pattern),
                owed.span,
                owed.other,
            ) {
                if owed.has_payload || owed.pattern {
                    self.unify(&side, &owed.side, owed.span, owed.other);
                } else {
                    self.payload_present(owed.good, &side, owed.span);
                }
            }
        }
    }
}
