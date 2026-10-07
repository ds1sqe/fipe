use super::{form, ir, Index, Origin, Scope, Source, Type, Vec};

/// A literal's form; a fraction the checker read as an integer is handed
/// over as that integer.
fn literal_form(
    source: &Source<'_>,
    id: crate::ast::ExprId,
    literal: &crate::ast::Literal,
) -> ir::Form {
    let integer = source.typed.expr(id).is_some_and(|ty| {
        matches!(crate::infer::meaning(ty), Type::Named { name, .. }
            if name.origin == Origin::Prelude && name.name == "Integer")
    });
    let whole = match literal {
        crate::ast::Literal::Float(text) if integer => {
            crate::token::Magnitude::whole_of_fraction(text)
        }
        _ => None,
    };
    whole.map_or_else(
        || form(literal),
        |(negative, magnitude)| ir::Form::Whole {
            negative,
            digits: alloc::format!("{magnitude}").len(),
            magnitude,
            radix: 10,
        },
    )
}

impl Index {
    pub(super) fn expression(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let span = expression.span;
        let site = (span.start, span.end);
        match &expression.kind {
            E::Literal(literal) => {
                let reader = source
                    .typed
                    .selected
                    .contains_key(&site)
                    .then(|| self.chosen(source, span, None));
                ir::Kind::Literal {
                    form: literal_form(source, id, literal),
                    reader,
                }
            }
            E::Word(text) | E::Path(text) => ir::Kind::Literal {
                form: ir::Form::Text(text.clone()),
                reader: None,
            },
            E::Group(parts) | E::Block(parts) => {
                self.sequence(source, scope, parts)
            }
            E::Tuple(parts) => ir::Kind::Tuple(
                parts.iter().map(|id| scope.expression(*id)).collect(),
            ),
            E::List(parts) => ir::Kind::Array(
                parts.iter().map(|id| scope.expression(*id)).collect(),
            ),
            E::Bind { pattern, value, .. } => ir::Kind::Bind {
                pattern: self.pattern(source, scope, *pattern),
                value: scope.expression(*value),
            },
            E::If { .. } => self.conditional(source, scope, id),
            E::Arms(arms) => self.function(source, scope, arms),
            E::With { .. } => self.with(source, scope, id),
            E::Selector(_) => self.selector(source, scope, id),
            E::Name(name)
                if source.resolved.expr(id)
                    == Some(&crate::resolve::Target::Word) =>
            {
                ir::Kind::Literal {
                    form: ir::Form::Text(name.segments.join(".")),
                    reader: None,
                }
            }
            E::Name(_) | E::Instance { .. } => self.name(source, scope, id),
            E::Call {
                callee, arguments, ..
            } => self.call_expression(source, scope, span, *callee, arguments),
            E::Not { .. } | E::Binary { .. } => {
                self.operator(source, scope, id)
            }
            E::Field { value, .. } => {
                let receiver = scope.expression(*value);
                scope.receivers.borrow_mut().insert(span, receiver);
                if let Some(bound) = source.typed.bound.get(&site) {
                    let callee = self.chosen(source, span, Some(receiver));
                    self.call(
                        source,
                        scope,
                        span,
                        callee,
                        bound,
                        Some(receiver),
                    )
                } else {
                    self.chosen_kind(source, span, Some(receiver))
                }
            }
            E::Pipe { .. } => Self::pipe(source, scope, id),
            E::Outcome { .. } => self.outcome(source, scope, id),
            E::Record { .. } => {
                let callee = self.chosen(source, span, None);
                self.call(
                    source,
                    scope,
                    span,
                    callee,
                    &source.typed.bound[&site],
                    None,
                )
            }
        }
    }

    fn call_expression(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        span: crate::token::Span,
        callee: crate::ast::ExprId,
        arguments: &[crate::ast::Argument],
    ) -> ir::Kind {
        let site = (span.start, span.end);

        if let Some(phase) =
            crate::resolve::timing(source.resolved.expr(callee))
        {
            let [crate::ast::Argument {
                kind: crate::ast::ArgumentKind::Positional(value),
                ..
            }] = arguments
            else {
                unreachable!("checked timing operand")
            };
            return ir::Kind::Timing {
                phase: if phase == crate::typed::Phase::Compile {
                    ir::Phase::Compile
                } else {
                    ir::Phase::Init
                },
                value: scope.expression(*value),
            };
        }
        let callee = if source.typed.conversions.contains_key(&site) {
            self.chosen(source, span, None)
        } else {
            scope.expression(callee)
        };
        self.call(
            source,
            scope,
            span,
            callee,
            &source.typed.bound[&site],
            None,
        )
    }

    pub(super) fn name(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let span = expression.span;
        let site = (span.start, span.end);
        match &expression.kind {
            E::Name(_) | E::Instance { .. } => {
                if let Some(kind) = self.member_name(source, scope, id) {
                    return kind;
                }
                if let Some(crate::resolve::Target::Local { site: binding }) =
                    source.resolved.expr(id)
                {
                    let slot = scope.slots[binding];
                    let signature = self.tables.borrow().slots[slot].clone();
                    let definition = signature.definition;
                    let arguments = source
                        .typed
                        .meaning_arguments
                        .get(&site)
                        .map_or(&[][..], Vec::as_slice);
                    let meanings = signature
                        .hidden
                        .iter()
                        .map(|hidden| {
                            let index = signature
                                .parameters
                                .iter()
                                .position(|parameter| parameter == hidden)
                                .expect("hidden parameter");
                            self.ty(&source.home, &arguments[index])
                        })
                        .collect();
                    let kind = if let Some(index) = definition {
                        ir::Kind::Definition { index, meanings }
                    } else {
                        ir::Kind::Local { slot, meanings }
                    };
                    if let Some(bound) =
                        source.typed.bound.get(&site).filter(|bound| {
                            !bound.defaults.is_empty()
                                || !bound.supplies.is_empty()
                        })
                    {
                        let callee = self.node(
                            &source.home,
                            span,
                            &source.typed.callee_types[&site],
                            kind,
                        );
                        return self
                            .call(source, scope, span, callee, bound, None);
                    }
                    return kind;
                }
                if let Some(bound) = source.typed.bound.get(&site) {
                    let chosen = self.chosen(source, span, None);
                    self.call(source, scope, span, chosen, bound, None)
                } else {
                    self.chosen_kind(source, span, None)
                }
            }
            _ => unreachable!("name expression"),
        }
    }

    fn member_name(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> Option<ir::Kind> {
        let span = source.module.expressions[id.0].span;
        let site = (span.start, span.end);
        if let Some(steps) = source.typed.accesses.get(&site) {
            if let Some(crate::resolve::Target::Members { head, .. }) =
                source.resolved.expr(id)
            {
                let mut receiver =
                    self.member_root(source, scope, head, &steps[0].head, span);
                let outer = source
                    .typed
                    .bound
                    .get(&site)
                    .filter(|bound| !bound.spots.is_empty());
                for (index, step) in steps.iter().enumerate() {
                    let last = index + 1 == steps.len();
                    scope.receivers.borrow_mut().insert(span, receiver);
                    let ty = if last {
                        source
                            .typed
                            .callee_types
                            .get(&site)
                            .unwrap_or(&step.callee)
                    } else {
                        &step.callee
                    };
                    let kind = self.selected_kind(
                        source,
                        &step.selected,
                        &step.meanings,
                        ty,
                        Some(receiver),
                    );
                    let (kind, ty) = match &step.bound {
                        Some(bound) => {
                            let callee =
                                self.node(&source.home, span, ty, kind);
                            let kind = self.call(
                                source,
                                scope,
                                span,
                                callee,
                                bound,
                                Some(receiver),
                            );
                            (kind, &step.result)
                        }
                        None => (kind, ty),
                    };
                    // The last step is the expression's own node; an
                    // earlier one is the receiver of the next.
                    if !last {
                        receiver = self.node(&source.home, span, ty, kind);
                        continue;
                    }
                    let Some(bound) = outer else {
                        return Some(kind);
                    };
                    let callee = self.node(&source.home, span, ty, kind);
                    return Some(self.call(
                        source,
                        scope,
                        span,
                        callee,
                        bound,
                        scope.receivers.borrow().get(&span).copied(),
                    ));
                }
            }
        }
        None
    }

    fn member_root(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        head: &crate::resolve::Target,
        ty: &Type,
        span: crate::token::Span,
    ) -> usize {
        let kind = match head {
            crate::resolve::Target::Local { site } => ir::Kind::Local {
                slot: scope.slots[site],
                meanings: Vec::new(),
            },
            crate::resolve::Target::Value { origin, name } => {
                let origin = super::home_origin(&source.home, origin);
                let index =
                    self.definitions.iter().find_map(|((home, _), index)| {
                        (*home == origin
                            && self.tables.borrow().definitions[*index].label
                                == *name)
                            .then_some(*index)
                    });
                let Some(index) = index else {
                    let module = super::owner(
                        source.declarations,
                        source.files,
                        &origin,
                    );
                    let site = module
                        .entries
                        .iter()
                        .find_map(|entry| {
                            let crate::ast::Entry::Item(item) = entry else {
                                return None;
                            };
                            let crate::ast::ItemKind::Declare {
                                name: declared,
                                ..
                            } = &item.kind
                            else {
                                return None;
                            };
                            (declared.segments.join(".") == *name)
                                .then_some(declared.span)
                        })
                        .expect("declared member receiver");
                    return self.selected(
                        source,
                        span,
                        &crate::typed::Selected::Declared { origin, site },
                        &[],
                        ty,
                        None,
                    );
                };
                ir::Kind::Definition {
                    index,
                    meanings: Vec::new(),
                }
            }
            crate::resolve::Target::TypeMember { .. } => {
                let root = &source.typed.roots[&(span.start, span.end)];
                return self.selected(
                    source,
                    span,
                    &root.selected,
                    &root.meanings,
                    ty,
                    None,
                );
            }
            _ => unreachable!("member root selection"),
        };
        self.node(&source.home, span, ty, kind)
    }

    pub(super) fn selector(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let span = expression.span;
        let site = (span.start, span.end);
        let ty = source.typed.expr(id).expect("checked expression");
        match &expression.kind {
            E::Selector(_) => {
                let Type::Function { parameter, result } = ty else {
                    unreachable!("selector function")
                };
                let input = self.ty(&source.home, parameter);
                let slot = self.tables.borrow().slots.len();
                self.tables.borrow_mut().slots.push(ir::Slot {
                    ty: input,
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    definition: None,
                });
                let mut receiver = self.node(
                    &source.home,
                    span,
                    parameter,
                    ir::Kind::Local {
                        slot,
                        meanings: Vec::new(),
                    },
                );
                let steps = &source.typed.accesses[&site];
                for (index, step) in steps.iter().enumerate() {
                    let callee = self.selected(
                        source,
                        span,
                        &step.selected,
                        &step.meanings,
                        if index + 1 == steps.len() {
                            source
                                .typed
                                .callee_types
                                .get(&site)
                                .unwrap_or(&step.callee)
                        } else {
                            &step.callee
                        },
                        Some(receiver),
                    );
                    let bound = step.bound.as_ref().or_else(|| {
                        (index + 1 == steps.len())
                            .then(|| source.typed.bound.get(&site))
                            .flatten()
                    });
                    receiver = bound
                        .filter(|bound| !bound.spots.is_empty())
                        .map_or(callee, |bound| {
                            let kind = self.call(
                                source,
                                scope,
                                span,
                                callee,
                                bound,
                                Some(receiver),
                            );
                            self.node(&source.home, span, result, kind)
                        });
                }
                ir::Kind::Function {
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    captures: Vec::new(),
                    arms: alloc::vec![ir::Arm {
                        span,
                        patterns: alloc::vec![ir::Pattern {
                            span,
                            kind: ir::PatternKind::Slot(slot)
                        }],
                        guard: None,
                        body: receiver
                    }],
                }
            }
            _ => unreachable!("selector expression"),
        }
    }

    pub(super) fn operator(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let span = expression.span;
        let ty = source.typed.expr(id).expect("checked expression");
        match &expression.kind {
            E::Not { operator, value }
            | E::Binary {
                operator,
                left: value,
                ..
            } => {
                let callee = self.chosen(
                    source,
                    *operator,
                    Some(scope.expression(*value)),
                );
                let call = self.call(
                    source,
                    scope,
                    *operator,
                    callee,
                    &source.typed.bound[&(operator.start, operator.end)],
                    Some(scope.expression(*value)),
                );
                if let E::Binary { op, .. } = expression.kind {
                    use crate::ast::BinaryOp;
                    let labels: &[&str] = match op {
                        BinaryOp::Less => &["Less"],
                        BinaryOp::LessEqual => &["Less", "Equal"],
                        BinaryOp::Greater => &["Greater"],
                        BinaryOp::GreaterEqual => &["Greater", "Equal"],
                        _ => &[],
                    };
                    if !labels.is_empty() {
                        let ordering = Type::named(
                            Origin::Prelude,
                            "Ordering",
                            Vec::new(),
                        );
                        let ir::Type::Meaning { index, .. } =
                            self.ty(&source.home, &ordering)
                        else {
                            unreachable!()
                        };
                        let accepted = labels
                            .iter()
                            .map(|label| self.constructor_index(index, label))
                            .collect();
                        let value =
                            self.node(&source.home, span, &ordering, call);
                        return ir::Kind::Ordering { value, accepted };
                    }
                    if op == BinaryOp::NotEqual {
                        let condition = self.node(&source.home, span, ty, call);
                        let then_body = self.boolean(source, span, false);
                        let else_body = self.boolean(source, span, true);
                        return ir::Kind::If {
                            condition,
                            then_body,
                            else_body,
                        };
                    }
                }
                call
            }
            _ => unreachable!("operator expression"),
        }
    }

    pub(super) fn pipe(
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::{ExprKind as E, PipeOp};
        let expression = &source.module.expressions[id.0];
        match &expression.kind {
            E::Pipe {
                op,
                operator,
                left,
                right,
            } => {
                let stage_span = source.module.expressions[right.0].span;
                let stage_site = (stage_span.start, stage_span.end);
                let left = scope.expression(*left);
                let right = scope.expression(*right);
                let site = (operator.start, operator.end);
                match op {
                    PipeOp::Plain => ir::Kind::Pipe { left, stage: right },
                    PipeOp::Good | PipeOp::Bad => ir::Kind::Marked {
                        good: *op == PipeOp::Good,
                        left,
                        stage: right,
                        rebuild: source.typed.rebuilds[&stage_site],
                    },
                    PipeOp::Or => ir::Kind::Or {
                        mode: source.typed.modes[&site],
                        left,
                        right,
                    },
                    PipeOp::And => ir::Kind::And {
                        mode: source.typed.modes[&site],
                        rebuild: source
                            .typed
                            .rebuilds
                            .get(&stage_site)
                            .copied(),
                        left,
                        right,
                    },
                }
            }
            _ => unreachable!("pipe expression"),
        }
    }

    pub(super) fn outcome(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let ty = source.typed.expr(id).expect("checked expression");
        match &expression.kind {
            E::Outcome { good, payload } => {
                let ir::Type::Meaning { index: meaning, .. } =
                    self.ty(&source.home, ty)
                else {
                    unreachable!("outcome meaning")
                };
                let label = if self.tables.borrow().meanings[meaning].label
                    == "Maybe"
                {
                    if *good {
                        "Just"
                    } else {
                        "Nothing"
                    }
                } else if *good {
                    "Ok"
                } else {
                    "Err"
                };
                let constructor = self.constructor_index(meaning, label);
                let fields = if let Some(payload) = payload {
                    alloc::vec![scope.expression(*payload)]
                } else if self.tables.borrow().meanings[meaning].constructors
                    [constructor]
                    .fields
                    .is_empty()
                {
                    Vec::new()
                } else {
                    alloc::vec![self.node(
                        &source.home,
                        expression.span,
                        &Type::Unit,
                        ir::Kind::Literal {
                            form: ir::Form::Unit,
                            reader: None
                        }
                    )]
                };
                ir::Kind::Construct {
                    meaning,
                    constructor,
                    fields,
                }
            }
            _ => unreachable!("outcome expression"),
        }
    }

    pub(super) fn with(
        &self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        match &expression.kind {
            E::With { key, value, body } => {
                let label = key.segments.join(".");
                let existing = self
                    .tables
                    .borrow()
                    .keys
                    .iter()
                    .position(|key| key == &label);
                let key = existing.unwrap_or_else(|| {
                    let index = self.tables.borrow().keys.len();
                    self.tables.borrow_mut().keys.push(label);
                    index
                });
                ir::Kind::With {
                    key,
                    value: scope.expression(*value),
                    body: scope.expression(*body),
                }
            }
            _ => unreachable!("with expression"),
        }
    }

    pub(super) fn conditional(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: crate::ast::ExprId,
    ) -> ir::Kind {
        use crate::ast::ExprKind as E;
        let expression = &source.module.expressions[id.0];
        let span = expression.span;
        match &expression.kind {
            E::If {
                condition,
                then_body,
                else_body,
            } => {
                let else_body = else_body.map_or_else(
                    || {
                        self.node(
                            &source.home,
                            span,
                            &Type::Unit,
                            ir::Kind::Literal {
                                form: ir::Form::Unit,
                                reader: None,
                            },
                        )
                    },
                    |id| scope.expression(id),
                );
                ir::Kind::If {
                    condition: scope.expression(*condition),
                    then_body: scope.expression(*then_body),
                    else_body,
                }
            }
            _ => unreachable!("conditional expression"),
        }
    }

    pub(super) fn function(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        arms: &[crate::ast::Arm],
    ) -> ir::Kind {
        let arms = arms
            .iter()
            .map(|arm| ir::Arm {
                span: arm.span,
                patterns: arm
                    .patterns
                    .iter()
                    .map(|pattern| self.pattern(source, scope, *pattern))
                    .collect(),
                guard: arm.guard.map(|guard| scope.expression(guard)),
                body: scope.expression(arm.body),
            })
            .collect();
        ir::Kind::Function {
            parameters: Vec::new(),
            hidden: Vec::new(),
            captures: Vec::new(),
            arms,
        }
    }

    pub(super) fn boolean(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        value: bool,
    ) -> usize {
        let ty = Type::named(Origin::Prelude, "Bool", Vec::new());
        let ir::Type::Meaning { index: meaning, .. } =
            self.ty(&source.home, &ty)
        else {
            unreachable!()
        };
        let constructor = self
            .constructor_index(meaning, if value { "True" } else { "False" });
        self.node(
            &source.home,
            span,
            &ty,
            ir::Kind::Construct {
                meaning,
                constructor,
                fields: Vec::new(),
            },
        )
    }

    /// The chosen callable's operation, for an expression that is that
    /// callable itself: its own node holds it, with no copy beside.
    pub(super) fn chosen_kind(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        receiver: Option<usize>,
    ) -> ir::Kind {
        let site = (span.start, span.end);
        if source.typed.derived_defaults.contains_key(&site)
            || source.typed.conversions.contains_key(&site)
        {
            let node = self.chosen(source, span, receiver);
            return self.tables.borrow().nodes[node].kind.clone();
        }
        self.selected_kind(
            source,
            source
                .typed
                .selected
                .get(&site)
                .expect("checked callable selection"),
            source
                .typed
                .meaning_arguments
                .get(&site)
                .map_or(&[], Vec::as_slice),
            &source.typed.callee_types[&site],
            receiver,
        )
    }

    pub(super) fn chosen(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        receiver: Option<usize>,
    ) -> usize {
        let site = (span.start, span.end);
        if let Some(value) = source.typed.derived_defaults.get(&site) {
            return self.default_node(source, span, value, &[], 0);
        }

        let conversion = source.typed.conversions.get(&site);
        let limited = conversion.is_some_and(|conversion| {
            matches!(
                conversion.mode,
                ir::ConversionMode::Check | ir::ConversionMode::TryCheck
            )
        });
        let callee = (!limited).then(|| {
            self.selected(
                source,
                span,
                source
                    .typed
                    .selected
                    .get(&site)
                    .expect("checked callable selection"),
                source
                    .typed
                    .meaning_arguments
                    .get(&site)
                    .map_or(&[], Vec::as_slice),
                &source.typed.callee_types[&site],
                receiver,
            )
        });
        conversion.map_or_else(
            || callee.expect("ordinary callable"),
            |conversion| {
                let from = self.ty(&source.home, &conversion.from);
                let to = self.ty(&source.home, &conversion.to);
                let parameter = self.predicate(
                    &source.home,
                    &conversion.parameter,
                    &mut Vec::new(),
                );
                let target = self.predicate(
                    &source.home,
                    &conversion.to,
                    &mut Vec::new(),
                );
                let kind = ir::Kind::Convert {
                    callee,
                    mode: conversion.mode,
                    from,
                    to,
                    parameter,
                    target,
                };
                self.node(
                    &source.home,
                    span,
                    &Type::function(
                        conversion.from.clone(),
                        conversion.result.clone(),
                    ),
                    kind,
                )
            },
        )
    }
}
