//! Expressions: literals, names, blocks, `if`, operators, lists,
//! records and bindings ([types.md] §Core judgments, §Fields, functions
//! and branches). Where an expected type is known, the expression is
//! checked against it.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md

use super::{
    bind::Part, prelude, prelude_of, Expected, Infer, Lit, Owed, Reason,
};
use crate::{
    ast::{BinaryOp, ExprId, ExprKind, FieldValue, Literal, PipeOp},
    resolve::Target,
    token::Span,
    types::{Obligation, Type},
};
use alloc::{string::ToString, vec::Vec};

impl Infer<'_> {
    /// Type an expression, checked against `expected` when given.
    pub(super) fn check_expr(
        &mut self,
        id: ExprId,
        expected: Option<&Expected>,
    ) -> Type {
        if let Some(expected) = expected.filter(|e| e.spot) {
            self.argument_span(&expected.ty, self.span(id));
        }
        if let Some(expected) = expected {
            self.state.entries.push((
                id,
                expected.ty.clone(),
                expected.span,
                expected.place_kind.clone(),
                matches!(expected.ty, Type::Var(_)),
            ));
        }
        let ty = self.synth(id, expected);
        if self.state.subst.too_deep(&ty) {
            self.error(self.span(id), Reason::TypeDepth);
            return self.fresh();
        }
        // A group, block, `with` or `if` checks its own last value, an
        // operator chain each node the expected type reaches.
        let checked = matches!(
            self.module.expressions[id.0].kind,
            ExprKind::Group(_)
                | ExprKind::Block(_)
                | ExprKind::With { .. }
                | ExprKind::If { .. }
                | ExprKind::Binary { .. }
                | ExprKind::Pipe { .. }
        );
        if let Some(expected) = expected.filter(|_| !checked) {
            self.unify(&expected.ty, &ty, self.span(id), expected.span);
        }
        self.set_type(id.0, ty.clone());
        ty
    }

    /// A written call: a timing form's operand, a literal written with
    /// operands, or a call bound by the spot steps.
    fn call(
        &mut self,
        callee: ExprId,
        arguments: &[crate::ast::Argument],
        style: crate::ast::CallStyle,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        if crate::resolve::timing(self.resolved.expr(callee)).is_some() {
            if let [crate::ast::Argument {
                kind: crate::ast::ArgumentKind::Positional(value),
                ..
            }] = arguments
            {
                return self.expr(*value, expected);
            }
        }
        if matches!(
            self.module.expressions[callee.0].kind,
            ExprKind::Literal(_)
        ) {
            return self.literal_applied(callee, arguments);
        }
        let callee = self.callee(callee);
        let expected = expected.map(|e| e.ty.clone());
        self.written_call(
            callee,
            arguments,
            style,
            None,
            span,
            expected.as_ref(),
        )
    }

    /// A literal written as a callee, `2 ^ 10`: it takes no operands. The
    /// error is at the first operand, which is what the writer may have
    /// meant as an operator.
    fn literal_applied(
        &mut self,
        callee: ExprId,
        arguments: &[crate::ast::Argument],
    ) -> Type {
        self.expr(callee, None);
        if let Some(first) = arguments.first() {
            self.error(first.span, Reason::LiteralApplied);
        }
        for argument in arguments {
            match &argument.kind {
                crate::ast::ArgumentKind::Positional(value)
                | crate::ast::ArgumentKind::Flag {
                    value: Some(value), ..
                } => {
                    self.expr(*value, None);
                }
                crate::ast::ArgumentKind::Flag { value: None, .. } => {}
            }
        }
        self.fresh()
    }

    fn binding_value(&mut self, id: ExprId) -> Type {
        self.bind(id).map_or(Type::Unit, |lift| {
            self.rebuild(
                &self.apply(&lift.value),
                true,
                Type::Unit,
                lift.span,
                lift.span,
            )
        })
    }

    /// A packed place: an `A | B`, or a meaning of member limits, and a
    /// limited type narrowing either.
    pub(super) fn packed(&mut self, ty: &Type) -> bool {
        let ty = super::meaning(ty);
        matches!(ty, Type::Either(_))
            || !self
                .member_limits(ty, ty, Span::default(), &mut Vec::new())
                .is_empty()
    }

    pub(super) fn pack(
        &mut self,
        target: &Type,
        actual: &Type,
        span: Span,
        other: Span,
    ) {
        if target == actual {
            return;
        }
        // A limited packed type packs as its meaning; its tests are
        // entered afterwards, as any limit is.
        let target = super::meaning(target);
        if let Type::Either(parts) = target {
            if !parts
                .iter()
                .any(|p| super::meaning(p) == super::meaning(actual))
            {
                self.unify(target, actual, span, other);
            }
            return;
        }
        self.require_claim(actual.clone(), target.clone(), span);
        self.dots();
    }

    fn synth(&mut self, id: ExprId, expected: Option<&Expected>) -> Type {
        let module = self.module;
        let span = self.span(id);
        match &module.expressions[id.0].kind {
            ExprKind::Literal(literal) => self.literal(literal, expected, span),
            ExprKind::Word(_) | ExprKind::Path(_) => prelude("String"),
            ExprKind::Name(_)
                if self.resolved.expr(id) == Some(&Target::Word) =>
            {
                prelude("String")
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
            ExprKind::Tuple(parts) => {
                let target = expected.map(|e| self.apply(&e.ty));
                let types = parts
                    .iter()
                    .enumerate()
                    .map(|(index, part)| {
                        let place = match &target {
                            Some(Type::Tuple(types)) => {
                                types.get(index).map(|ty| {
                                    Expected::new(ty.clone(), span)
                                        .at(crate::typed::Place::Part(index))
                                })
                            }
                            _ => None,
                        };
                        self.expr(*part, place.as_ref())
                    })
                    .collect();
                Type::Tuple(types)
            }
            ExprKind::Outcome { good, payload } => {
                self.bracket(*good, *payload, expected, span)
            }
            ExprKind::Record { fields, .. } => self.record(id, fields, span),
            ExprKind::Call {
                callee,
                arguments,
                style,
            } => self.call(*callee, arguments, *style, expected, span),
            ExprKind::Not { operator, value } => {
                let ty = self.expr(*value, expected);
                self.require_claim(ty.clone(), prelude("Not"), *operator);
                self.record_not(*operator, &ty);
                ty
            }
            ExprKind::Binary { .. } | ExprKind::Pipe { .. } => {
                self.chain(id, expected)
            }
            ExprKind::Bind { .. } => self.binding_value(id),
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

    /// A written form stays open until its place or binding chooses a meaning.
    pub(super) fn literal(
        &mut self,
        literal: &Literal,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        if matches!(literal, Literal::Unit) {
            return Type::Unit;
        }
        let Some(lit) = Lit::of(literal) else {
            return self.fresh();
        };
        let ty = match expected {
            Some(e) => self.apply(&e.ty),
            None => self.fresh(),
        };
        self.select(
            (span.start, span.end),
            crate::typed::Selected::Static {
                head: ty.clone(),
                name: alloc::string::String::from("literal"),
            },
        );
        self.insert(
            |s| &mut s.callee_types,
            (span.start, span.end),
            Type::function(prelude("Form"), ty.clone()),
        );
        let obligation = Obligation::Claim {
            head: ty.clone(),
            class: prelude("Literal"),
            dot: span,
        };
        let mut owed = Owed::new(obligation, span);
        owed.literal = Some(lit);
        owed.literal_argument = expected.is_some_and(|e| e.spot);
        self.state.obligations.push(owed);
        ty
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
                if let Some(lift) = self.bind(*line) {
                    lifts.push(lift);
                }
                self.set_type(line.0, Type::Unit);
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
    pub(super) fn line(&mut self, id: ExprId) {
        let module = self.module;
        if matches!(module.expressions[id.0].kind, ExprKind::Bind { .. }) {
            if let Some(lift) = self.bind(id) {
                self.state.main_lifts.push(lift);
            }
            self.set_type(id.0, Type::Unit);
        } else {
            let mark = self.mark();
            self.expr(id, None);
            self.line_boundary(self.span(id), mark);
        }
    }

    /// A queried line: an expression is generalized like a binding's
    /// value, its variables and obligations kept in its scheme.
    pub(super) fn query_line(&mut self, id: ExprId) {
        if matches!(self.module.expressions[id.0].kind, ExprKind::Bind { .. }) {
            self.line(id);
            return;
        }
        let mark = self.mark();
        let ty = self.expr(id, None);
        let part = Part {
            ty,
            site: self.span(id),
            annotated: false,
        };
        let scheme = self.generalize(&[part], mark, &[]).remove(0);
        self.set_binding(id.0, scheme);
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
        if let Some(else_body) = else_body {
            let then_ty = self.expr(then_body, expected);
            let sibling = Expected::new(then_ty.clone(), self.span(then_body));
            self.expr(else_body, Some(&sibling));
            return then_ty;
        }
        // Without `else` the `if` is `()`, so its body must be too; a
        // body with a value says so, not that `()` lacks its claim.
        let then_ty = self.expr(then_body, None);
        let applied = self.apply(&then_ty);
        let valued = match applied {
            Type::Unit => false,
            Type::Var(_) => self.numeric_var(&applied),
            _ => true,
        };
        if valued {
            self.error(self.span(then_body), Reason::IfWithoutElse);
        } else {
            self.unify(&Type::Unit, &then_ty, self.span(then_body), span);
        }
        if let Some(expected) = expected {
            self.unify(&expected.ty, &Type::Unit, span, expected.span);
        }
        Type::Unit
    }

    /// Operators and pipes nested to the left, as a flat chain parses,
    /// typed from the innermost left operand outward without a frame
    /// per element. Each inner node is finished as [`Self::expr`]
    /// finishes it; `id` itself is finished by its caller, except that
    /// the chain checks every node the expected type reaches. An
    /// arithmetic node's type is its left operand's, which was checked,
    /// so it is not checked again.
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
                    reach &= is_arithmetic(*op);
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
        // A literal operand reads as the place's meaning: a width is a
        // limit, tested on the result at the place, never on an operand
        // (`04-members.md` §Number types).
        let operand = expected
            .filter(|_| {
                reach
                    && matches!(
                        module.expressions[node.0].kind,
                        ExprKind::Literal(_)
                    )
            })
            .map(|expected| {
                if spine.is_empty() {
                    return expected.clone();
                }
                let mut operand = expected.clone();
                operand.ty = super::meaning(&self.apply(&expected.ty)).clone();
                operand
            });
        let inner = operand.as_ref();
        let literal_run = matches!(module.expressions[node.0].kind, ExprKind::Literal(_))
            && spine.iter().all(|(id, _)| matches!(&module.expressions[id.0].kind,
                ExprKind::Binary { right, .. } if matches!(module.expressions[right.0].kind, ExprKind::Literal(_))));
        let mark = self.state.obligations.len();
        let mut ty = self.expr(node, inner);
        for (index, &(node, reach)) in spine.iter().enumerate().rev() {
            let expected = expected.filter(|_| reach);
            let kind = &module.expressions[node.0].kind;
            ty = match kind {
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
            if let Some(expected) = expected {
                self.unify(&expected.ty, &ty, self.span(node), expected.span);
            }
            if index > 0 {
                self.set_type(node.0, ty.clone());
            }
        }
        if literal_run {
            self.default_literals(mark, &[], &[]);
        }
        self.apply(&ty)
    }

    fn concat_list(
        &mut self,
        left_type: Type,
        operator: Span,
        right: ExprId,
    ) -> Type {
        let callee = self.members(
            left_type,
            &[alloc::string::String::from("concat")],
            operator,
        );
        let argument = crate::ast::Argument {
            kind: crate::ast::ArgumentKind::Positional(right),
            span: self.span(right),
        };
        self.apply_callee(callee, &[argument], None, operator, None)
    }
    fn binary(
        &mut self,
        op: BinaryOp,
        operator: Span,
        (left, left_ty): (ExprId, Type),
        right: ExprId,
    ) -> Type {
        let arithmetic = is_arithmetic(op);
        let literal_tie = [left, right].iter().any(|id| {
            matches!(self.module.expressions[id.0].kind, ExprKind::Literal(_))
        });
        let left_type = self.apply(&left_ty);
        if op == BinaryOp::Concat
            && matches!(
                self.module.expressions[right.0].kind,
                ExprKind::List(_)
            )
            && !matches!(
                super::meaning(&left_type),
                Type::Var(_) | Type::Rigid(_)
            )
        {
            return self.concat_list(left_type, operator, right);
        }
        let expected = (!arithmetic
            && op != BinaryOp::Concat
            && matches!(
                self.module.expressions[right.0].kind,
                ExprKind::List(_)
            ))
        .then(|| Expected::new(left_type, self.span(right)));
        let right_ty = self.expr(right, expected.as_ref());
        if (!arithmetic && op != BinaryOp::Concat) || literal_tie {
            let left_meaning = super::meaning(&self.apply(&left_ty)).clone();
            let right_meaning = super::meaning(&self.apply(&right_ty)).clone();
            self.unify(
                &left_meaning,
                &right_meaning,
                self.span(right),
                self.span(left),
            );
        }
        let head = super::meaning(&self.apply(&left_ty)).clone();
        let known = !matches!(head, Type::Var(_) | Type::Rigid(_));
        if known && literal_tie {
            let errors = self.state.errors.len();
            self.dots();
            if self.state.errors[errors..].iter().any(|error| {
                matches!(&error.reason, Reason::MissingClaim { claim, .. } if super::is_prelude(claim, "Literal"))
            }) {
                return self.fresh();
            }
        }
        let result = if arithmetic {
            if matches!(head, Type::Var(_) | Type::Rigid(_)) {
                head
            } else {
                self.fresh()
            }
        } else if op == BinaryOp::Concat {
            self.fresh()
        } else {
            prelude("Bool")
        };
        let name = operator_member(op);
        self.record_operation(
            operator,
            left_ty.clone(),
            name,
            Some(right_ty.clone()),
            if name == "cmp" {
                prelude("Ordering")
            } else {
                result.clone()
            },
        );
        if name == "concat" {
            self.require_member(
                left_ty,
                name,
                Type::function(right_ty, result.clone()),
                operator,
            );
        } else {
            let class = match name {
                "add" => "Add",
                "sub" => "Sub",
                "mul" => "Mul",
                "div" => "Div",
                "rem" => "Rem",
                "eq" => "Eq",
                _ => "Ord",
            };
            let args = if arithmetic {
                alloc::vec![
                    super::meaning(&self.apply(&right_ty)).clone(),
                    result.clone()
                ]
            } else {
                Vec::new()
            };
            self.require_claim(left_ty, prelude_of(class, args), operator);
        }
        if known {
            self.dots();
        }
        self.apply(&result)
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
        let target = expected.map(|e| self.apply(&e.ty));
        let mut length = Type::Number((items.len() as u64).into());
        let element = match target.as_ref().map(super::meaning) {
            Some(Type::Array { element, length: n }) => {
                self.unify(n, &length, span, expected.unwrap().span);
                length = (**n).clone();
                (**element).clone()
            }
            Some(head) if !matches!(head, Type::Var(_) | Type::Rigid(_)) => {
                let element = self
                    .claims(head, true)
                    .into_iter()
                    .find_map(|claim| {
                        if let Type::Named { name, arguments } = claim {
                            if name.origin == crate::resolve::Origin::Prelude
                                && name.name == "Sequence"
                            {
                                return arguments.into_iter().next();
                            }
                        }
                        None
                    })
                    .unwrap_or_else(|| self.fresh());
                let claim =
                    prelude_of("Sequence", alloc::vec![element.clone()]);
                self.require_claim(head.clone(), claim, span);
                self.dots();
                if let Some(Type::Array {
                    element: e,
                    length: n,
                }) = self.sequence_source(head, span)
                {
                    self.unify(&e, &element, span, span);
                    self.unify(&n, &length, span, span);
                    length = *n;
                }
                element
            }
            _ => self.fresh(),
        };
        let mut sibling = Expected::new(element.clone(), span)
            .at(crate::typed::Place::Element);
        sibling.cast = expected.is_some_and(|e| e.cast);
        for (index, item) in items.iter().enumerate() {
            sibling.span = self.span(*item);
            let ty = self.expr(*item, Some(&sibling));
            if index == 0 {
                let cast = sibling.cast;
                sibling = Expected::new(ty, self.span(*item))
                    .at(crate::typed::Place::Element);
                sibling.cast = cast;
            }
        }
        if let Some(target) = target {
            if !matches!(
                super::meaning(&target),
                Type::Var(_) | Type::Rigid(_) | Type::Array { .. }
            ) {
                return target;
            }
        }
        Type::Array {
            element: alloc::boxed::Box::new(element),
            length: alloc::boxed::Box::new(length),
        }
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
            let expected = declared.iter().find(|f| f.name == name).map(|f| {
                Expected::new(f.ty.clone(), field.name.span).at(
                    crate::typed::Place::Field {
                        owner: ty.to_string(),
                        name: name.clone(),
                    },
                )
            });
            self.expr(field.value, expected.as_ref());
        }
        self.record_fields(id, fields, &ty, span);
        for field in declared {
            if !field.default
                && !fields.iter().any(|f| super::last(&f.name) == field.name)
            {
                let name = field.name;
                let reason = if field.public {
                    Reason::MissingField { name }
                } else {
                    Reason::NoConstructor { field: name }
                };
                self.error(span, reason);
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
                    .and_then(|m| self.type_item(m, name))
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

const fn is_arithmetic(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder
    )
}

const fn operator_member(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Subtract => "sub",
        BinaryOp::Multiply => "mul",
        BinaryOp::Divide => "div",
        BinaryOp::Remainder => "rem",
        BinaryOp::Concat => "concat",
        BinaryOp::Equal | BinaryOp::NotEqual => "eq",
        _ => "cmp",
    }
}

impl Infer<'_> {
    fn record_not(&mut self, operator: Span, ty: &Type) {
        self.record_operation(operator, ty.clone(), "not", None, ty.clone());
    }
}
