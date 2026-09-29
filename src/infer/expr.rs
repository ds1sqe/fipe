//! Expressions: literals, names, blocks, `if`, operators, lists,
//! records and bindings ([types.md] §Core judgments, §Fields, functions
//! and branches). Where an expected type is known, the expression is
//! checked against it.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/types.md

use super::{
    bind::{in_range, Part},
    is_prelude, prelude, prelude_of, Expected, Infer, Lit, Owed, Reason,
    NUMERIC,
};
use crate::{
    ast::{BinaryOp, ExprId, ExprKind, FieldValue, Literal, PipeOp},
    resolve::{Origin, Target},
    token::Span,
    types::{Obligation, Type},
};
use alloc::{string::String, vec::Vec};

impl Infer<'_> {
    /// Type an expression, checked against `expected` when given.
    pub(super) fn expr(
        &mut self,
        id: ExprId,
        expected: Option<&Expected>,
    ) -> Type {
        let ty = self.synth(id, expected);
        // A group, block, `with` or `if` checks its own last value.
        let checked = matches!(
            self.module.expressions[id.0].kind,
            ExprKind::Group(_)
                | ExprKind::Block(_)
                | ExprKind::With { .. }
                | ExprKind::If { .. }
        );
        if let Some(expected) = expected.filter(|_| !checked) {
            self.unify(&expected.ty, &ty, self.span(id), expected.span);
        }
        self.state.types[id.0] = Some(ty.clone());
        ty
    }

    fn synth(&mut self, id: ExprId, expected: Option<&Expected>) -> Type {
        let module = self.module;
        let span = self.span(id);
        match &module.expressions[id.0].kind {
            ExprKind::Literal(literal) => self.literal(literal, expected, span),
            ExprKind::Word(_) | ExprKind::Path(_) => self.word(expected),
            ExprKind::Name(_)
                if self.resolved.expr(id) == Some(&Target::Word) =>
            {
                self.word(expected)
            }
            ExprKind::Name(_)
            | ExprKind::Instance { .. }
            | ExprKind::Field { .. } => {
                let callee = self.callee(id);
                let expected = expected.map(|e| e.ty.clone());
                self.apply_callee(callee, &[], None, span, expected.as_ref())
            }
            ExprKind::Selector(name) => self.selector(name, expected, span),
            ExprKind::Group(lines) | ExprKind::Block(lines) => {
                self.block(lines, expected)
            }
            ExprKind::List(items) => self.list(items, expected, span),
            ExprKind::Outcome { good, payload } => {
                self.bracket(*good, *payload, expected, span)
            }
            ExprKind::Record { fields, .. } => self.record(id, fields, span),
            ExprKind::Call {
                callee, arguments, ..
            } => {
                let callee = self.callee(*callee);
                let expected = expected.map(|e| e.ty.clone());
                self.apply_callee(
                    callee,
                    arguments,
                    None,
                    span,
                    expected.as_ref(),
                )
            }
            ExprKind::Not { operator, value } => {
                let bool = Expected::new(prelude("Bool"), *operator);
                self.expr(*value, Some(&bool));
                prelude("Bool")
            }
            ExprKind::Binary { .. } | ExprKind::Pipe { .. } => {
                self.chain(id, expected)
            }
            ExprKind::Bind { .. } => {
                self.bind(id, false);
                Type::Unit
            }
            ExprKind::With { value, body, .. } => {
                self.expr(*value, None);
                self.expr(*body, expected)
            }
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.branch(*condition, *then_body, *else_body, expected, span)
            }
            ExprKind::Arms(arms) => self.arms(arms, expected),
        }
    }

    /// Integer literals default to `i64` and float literals to `f64`
    /// unless the expected type is numeric: any for an integer, a float
    /// for a float. An expected variable or rigid name is the literal's
    /// type, owing `Num` or `Float`; an integer out of its type's range
    /// is an error.
    pub(super) fn literal(
        &mut self,
        literal: &Literal,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        let expected = expected.map(|e| self.apply(&e.ty));
        let numeric = expected
            .clone()
            .filter(|ty| NUMERIC.iter().any(|name| is_prelude(ty, name)));
        let open =
            expected.filter(|ty| matches!(ty, Type::Var(_) | Type::Rigid(_)));
        let lit = match literal {
            Literal::Unit => return Type::Unit,
            Literal::Str { .. } => return prelude("String"),
            Literal::Bytes(_) => {
                return prelude_of("Vec", alloc::vec![prelude("u8")]);
            }
            Literal::Int {
                negative,
                magnitude,
            } => Lit::Int {
                negative: *negative,
                magnitude: *magnitude,
            },
            Literal::Float(_) => Lit::Float,
        };
        if let Some(ty) = open {
            let class = if matches!(lit, Lit::Float) {
                "Float"
            } else {
                "Num"
            };
            let obligation = Obligation::Class {
                class: String::from(class),
                ty: ty.clone(),
            };
            let mut owed = Owed::new(obligation, span);
            owed.literal = Some(lit);
            self.state.obligations.push(owed);
            return ty;
        }
        match lit {
            Lit::Int {
                negative,
                magnitude,
            } => {
                let ty = numeric.unwrap_or_else(|| prelude("i64"));
                if in_range(&ty, negative, magnitude) == Some(false) {
                    self.error(span, Reason::OutOfRange);
                }
                ty
            }
            Lit::Float => numeric
                .filter(|ty| is_prelude(ty, "f32") || is_prelude(ty, "f64"))
                .unwrap_or_else(|| prelude("f64")),
        }
    }

    /// A word or path is text: at a spot of type `String` or of a
    /// host-declared type implementing `Read` it takes that type,
    /// anywhere else it is `String` (types.md §Conversion).
    fn word(&self, expected: Option<&Expected>) -> Type {
        if let Some(expected) = expected.filter(|e| e.spot) {
            let ty = self.apply(&expected.ty);
            if self.reads(&ty) {
                return ty;
            }
        }
        prelude("String")
    }

    /// A block is its last line, or `()`; a refutable binding lifts the
    /// rest of the block into its outcome. Any other line's value is
    /// `()`, or discarded by `_ = e`.
    fn block(&mut self, lines: &[ExprId], expected: Option<&Expected>) -> Type {
        let module = self.module;
        let env = self.state.env.len();
        let mut lifts = Vec::new();
        let mut ty = Type::Unit;
        let mut checked = false;
        for (index, line) in lines.iter().enumerate() {
            let last = index + 1 == lines.len();
            if matches!(module.expressions[line.0].kind, ExprKind::Bind { .. })
            {
                if let Some(lift) = self.bind(*line, false) {
                    lifts.push(lift);
                }
                self.state.types[line.0] = Some(Type::Unit);
                ty = Type::Unit;
            } else {
                let expected = expected.filter(|_| last && lifts.is_empty());
                checked = expected.is_some();
                ty = self.expr(*line, expected);
                if !last {
                    self.discard(&ty, self.span(*line));
                }
            }
        }
        self.state.env.truncate(env);
        for lift in lifts.into_iter().rev() {
            ty = self.rebuild(&lift.value, true, ty, lift.span, lift.span);
        }
        if let Some(expected) = expected.filter(|_| !checked) {
            let span = lines.last().map_or(expected.span, |l| self.span(*l));
            self.unify(&expected.ty, &ty, span, expected.span);
        }
        ty
    }

    /// A line of the module.
    pub(super) fn line(&mut self, id: ExprId, file: bool) {
        let module = self.module;
        if matches!(module.expressions[id.0].kind, ExprKind::Bind { .. }) {
            self.bind(id, file);
            self.state.types[id.0] = Some(Type::Unit);
        } else {
            let mark = self.mark();
            let ty = self.expr(id, None);
            self.line_boundary(&ty, self.span(id), mark);
        }
    }

    /// A queried line: an expression is generalized like a binding's
    /// value, its variables and obligations kept in its scheme.
    pub(super) fn query_line(&mut self, id: ExprId) {
        if matches!(self.module.expressions[id.0].kind, ExprKind::Bind { .. }) {
            self.line(id, true);
            return;
        }
        let mark = self.mark();
        let ty = self.expr(id, None);
        let part = Part {
            ty,
            site: self.span(id),
            annotated: false,
            public: false,
        };
        let scheme = self.generalize(&[part], mark, &[]).remove(0);
        self.state.bindings[id.0] = Some(scheme);
    }

    fn branch(
        &mut self,
        condition: ExprId,
        then_body: ExprId,
        else_body: Option<ExprId>,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        self.expr(condition, Some(&Expected::new(prelude("Bool"), span)));
        let then_ty = self.expr(then_body, expected);
        if let Some(else_body) = else_body {
            let sibling = Expected::new(then_ty.clone(), self.span(then_body));
            self.expr(else_body, Some(&sibling));
            return then_ty;
        }
        self.unify(&Type::Unit, &then_ty, self.span(then_body), span);
        if let Some(expected) = expected {
            self.unify(&expected.ty, &Type::Unit, span, expected.span);
        }
        Type::Unit
    }

    /// Operators and pipes nested to the left, as a flat chain parses,
    /// typed from the innermost left operand outward without a frame
    /// per element. Each inner node is finished as [`Self::expr`]
    /// finishes it; `id` itself is finished by its caller.
    fn chain(&mut self, id: ExprId, expected: Option<&Expected>) -> Type {
        let module = self.module;
        // Each node, and whether `expected` reaches it: arithmetic
        // passes it to its left operand, anything else does not.
        let mut spine = Vec::new();
        let mut node = id;
        let mut reach = true;
        loop {
            match &module.expressions[node.0].kind {
                ExprKind::Binary { op, left, .. } => {
                    spine.push((node, reach));
                    reach &= class(*op) == "Num";
                    node = *left;
                }
                ExprKind::Pipe { left, .. } => {
                    spine.push((node, reach));
                    reach = false;
                    node = *left;
                }
                _ => break,
            }
        }
        let mut ty = self.expr(node, expected.filter(|_| reach));
        for (index, &(node, reach)) in spine.iter().enumerate().rev() {
            let expected = expected.filter(|_| reach);
            ty = match &module.expressions[node.0].kind {
                ExprKind::Binary {
                    op,
                    operator,
                    left,
                    right,
                } => self.binary(*op, *operator, (*left, ty), *right),
                ExprKind::Pipe {
                    op: PipeOp::Plain,
                    left,
                    right,
                    ..
                } => {
                    let expected = expected.map(|e| e.ty.clone());
                    let span = self.span(*left);
                    self.stage(*right, ty, span, expected.as_ref())
                }
                ExprKind::Pipe {
                    op,
                    operator,
                    left,
                    right,
                } => self.pipe(*op, (*left, ty), *right, *operator),
                _ => unreachable!("the spine holds operators and pipes"),
            };
            if index > 0 {
                if let Some(expected) = expected {
                    self.unify(
                        &expected.ty,
                        &ty,
                        self.span(node),
                        expected.span,
                    );
                }
                self.state.types[node.0] = Some(ty.clone());
            }
        }
        ty
    }

    /// Arithmetic needs one type and its `Num` obligation; comparison
    /// one type and `Eq` or `Ord`, giving `Bool`. The left operand is
    /// already typed.
    fn binary(
        &mut self,
        op: BinaryOp,
        operator: Span,
        (left, left_ty): (ExprId, Type),
        right: ExprId,
    ) -> Type {
        let class = class(op);
        let arithmetic = class == "Num";
        let sibling = Expected::new(left_ty.clone(), self.span(left));
        self.expr(right, Some(&sibling));
        self.obligation(class, left_ty.clone(), operator);
        if arithmetic {
            left_ty
        } else {
            prelude("Bool")
        }
    }

    /// A marked pipe, `||` or `&&` on its typed left; a plain pipe is a
    /// stage of [`Self::chain`].
    fn pipe(
        &mut self,
        op: PipeOp,
        left: (ExprId, Type),
        right: ExprId,
        operator: Span,
    ) -> Type {
        match op {
            PipeOp::Good | PipeOp::Bad => {
                self.marked(op == PipeOp::Good, left, right, operator)
            }
            PipeOp::Plain => unreachable!("a plain pipe is a stage"),
            PipeOp::Or => self.or(left, right, operator),
            PipeOp::And => self.and(left, right, operator),
        }
    }

    /// A list's elements share one type.
    fn list(
        &mut self,
        items: &[ExprId],
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        let element = match expected.map(|e| self.apply(&e.ty)) {
            Some(Type::Named { name, arguments })
                if name.origin == Origin::Prelude
                    && name.name == "Vec"
                    && arguments.len() == 1 =>
            {
                arguments[0].clone()
            }
            _ => self.fresh(),
        };
        let mut sibling = Expected::new(element.clone(), span);
        for (index, item) in items.iter().enumerate() {
            let ty = self.expr(*item, Some(&sibling));
            if index == 0 {
                sibling = Expected::new(ty, self.span(*item));
            }
        }
        prelude_of("Vec", alloc::vec![element])
    }

    /// A line whose value nothing uses: `()`, else an error.
    fn discard(&mut self, ty: &Type, span: Span) {
        match self.apply(ty) {
            Type::Unit => {}
            Type::Var(_) if !self.numeric_var(ty) => self.hint(&Type::Unit, ty),
            _ => self.error(span, Reason::Discarded),
        }
    }

    /// `T { field: value, … }`: each value checked against its field;
    /// a field with no default must be given.
    fn record(
        &mut self,
        id: ExprId,
        fields: &[FieldValue],
        span: Span,
    ) -> Type {
        let Some((ty, declared)) = self.record_shape(self.resolved.expr(id))
        else {
            for field in fields {
                self.expr(field.value, None);
            }
            return self.fresh();
        };
        for field in fields {
            let name = super::last(&field.name);
            let expected = declared
                .iter()
                .find(|f| f.name == name)
                .map(|f| Expected::new(f.ty.clone(), field.name.span));
            self.expr(field.value, expected.as_ref());
        }
        for field in declared {
            if !field.default
                && !fields.iter().any(|f| super::last(&f.name) == field.name)
            {
                let name = field.name;
                self.error(span, Reason::MissingField { name });
            }
        }
        ty
    }

    /// The type and fields a record form names: a record type, `Self`,
    /// or a sum constructor with fields.
    pub(super) fn record_shape(
        &mut self,
        target: Option<&Target>,
    ) -> Option<(Type, Vec<super::lower::Field>)> {
        let ty = match target? {
            Target::Type { origin, name } => {
                let count = self
                    .owner(origin)
                    .and_then(|m| super::lower::type_item(m, name))
                    .map_or(0, |(parameters, _)| parameters.len());
                let arguments = (0..count).map(|_| self.fresh()).collect();
                Type::named(origin.clone(), name, arguments)
            }
            Target::SelfType => self.self_type.clone()?,
            Target::Constructor { origin, ty, name } => {
                return Some(self.constructor_fields(origin, ty, name));
            }
            _ => return None,
        };
        let fields = self.fields(&ty)?;
        Some((ty, fields))
    }
}

/// The obligation an operator owes on its operands' type.
const fn class(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Remainder => "Num",
        BinaryOp::Equal | BinaryOp::NotEqual => "Eq",
        _ => "Ord",
    }
}
