use super::{
    Error, ExprId, ExprKind, Module, Origin, Pass, Phase, Span, Target,
    TypeReason,
};
use crate::ast::{
    ArgumentKind, Entry, ImplMember, ItemKind, PatternKind, TypeId, TypeKind,
};
use alloc::{format, vec::Vec};

impl Pass<'_> {
    pub(super) fn expression_phase(&self, id: ExprId) -> Phase {
        let get = |id: ExprId| self.typed.known[id.0].phase;
        let expression = &self.module.expressions[id.0];
        let latest = |ids: &[ExprId]| {
            ids.iter()
                .map(|id| get(*id))
                .max()
                .unwrap_or(Phase::Compile)
        };
        match &expression.kind {
            ExprKind::Literal(_)
            | ExprKind::Word(_)
            | ExprKind::Path(_)
            | ExprKind::Selector(_)
            | ExprKind::Arms(_) => Phase::Compile,
            ExprKind::Name(_) | ExprKind::Instance { .. } => {
                self.target_phase(self.resolved.expr(id), expression.span)
            }
            ExprKind::Group(ids)
            | ExprKind::Block(ids)
            | ExprKind::Tuple(ids)
            | ExprKind::List(ids) => latest(ids),
            ExprKind::Bind { value, .. }
            | ExprKind::Not { value, .. }
            | ExprKind::Field { value, .. } => get(*value),
            ExprKind::Outcome { payload, .. } => {
                payload.map_or(Phase::Compile, get)
            }
            ExprKind::Record { fields, .. } => fields
                .iter()
                .map(|f| get(f.value))
                .max()
                .unwrap_or(Phase::Compile),
            ExprKind::With { value, body, .. } => get(*value)
                .max(get(*body))
                .max(self.site_phase(expression.span)),
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => get(*condition)
                .max(get(*then_body))
                .max(else_body.map_or(Phase::Compile, get)),
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipe { left, right, .. } => get(*left)
                .max(get(*right))
                .max(if self.pure_expression(id) {
                    Phase::Compile
                } else {
                    self.site_phase(expression.span)
                }),
            ExprKind::Call {
                callee, arguments, ..
            } => {
                let phase = arguments
                    .iter()
                    .filter_map(|a| match a.kind {
                        ArgumentKind::Positional(id)
                        | ArgumentKind::Flag {
                            value: Some(id), ..
                        } => Some(get(id)),
                        ArgumentKind::Flag { value: None, .. } => None,
                    })
                    .max()
                    .unwrap_or(Phase::Compile);
                phase
                    .max(
                        if crate::resolve::timing(self.resolved.expr(*callee))
                            .is_some()
                        {
                            Phase::Compile
                        } else {
                            get(*callee)
                        },
                    )
                    .max(if self.pure_expression(id) {
                        Phase::Compile
                    } else {
                        self.site_phase(expression.span)
                    })
            }
        }
    }

    pub(super) fn site_phase(&self, span: Span) -> Phase {
        if super::frequency(self.module, span) == super::Frequency::PerCall {
            return Phase::Eval;
        }
        if self.module.expressions.iter().any(|e| {
            matches!(e.kind, ExprKind::Bind { .. })
                && e.span.start <= span.start
                && span.end <= e.span.end
        }) {
            Phase::Init
        } else {
            Phase::Eval
        }
    }

    fn target_phase(&self, target: Option<&Target>, span: Span) -> Phase {
        match target {
            Some(
                Target::Local { .. }
                | Target::Value {
                    origin: Origin::File(_),
                    ..
                },
            ) => self.target(target).phase,
            Some(Target::Value { origin, name })
                if matches!(origin, Origin::Host(_) | Origin::Commands) =>
            {
                if self.owner(origin).and_then(|m| binding(m, name)).is_some() {
                    Phase::Compile
                } else {
                    self.site_phase(span)
                }
            }
            Some(Target::Members { head, .. }) => self
                .target_phase(Some(head), span)
                .max(if self.pure_target(target, &mut Vec::new()) {
                    Phase::Compile
                } else {
                    self.site_phase(span)
                }),
            Some(Target::TypeMember { .. }) => {
                if self.pure_target(target, &mut Vec::new()) {
                    Phase::Compile
                } else {
                    self.site_phase(span)
                }
            }
            Some(_) => Phase::Compile,
            None => Phase::Eval,
        }
    }

    pub(super) fn pure_expression(&self, id: ExprId) -> bool {
        let pure = |id: ExprId| self.typed.known[id.0].pure;
        match &self.module.expressions[id.0].kind {
            ExprKind::Name(_) | ExprKind::Instance { .. } => {
                self.pure_name(self.resolved.expr(id))
            }
            ExprKind::Call {
                callee, arguments, ..
            } => {
                (crate::resolve::timing(self.resolved.expr(*callee)).is_some()
                    || pure(*callee))
                    && arguments.iter().all(|a| match a.kind {
                        ArgumentKind::Positional(id)
                        | ArgumentKind::Flag {
                            value: Some(id), ..
                        } => pure(id),
                        ArgumentKind::Flag { value: None, .. } => true,
                    })
            }
            ExprKind::Binary {
                op, left, right, ..
            } => {
                pure(*left)
                    && pure(*right)
                    && self.pure_member_of(
                        operator_member(*op),
                        self.typed.expr(*left).and_then(type_head),
                        &mut Vec::new(),
                    )
            }
            ExprKind::Pipe { left, right, .. } => pure(*left) && pure(*right),
            ExprKind::Group(ids)
            | ExprKind::Block(ids)
            | ExprKind::List(ids)
            | ExprKind::Tuple(ids) => ids.iter().all(|id| pure(*id)),
            ExprKind::Bind { value, .. } | ExprKind::Not { value, .. } => {
                pure(*value)
            }
            ExprKind::Field { value, field } => {
                pure(*value)
                    && self.pure_member(
                        field
                            .segments
                            .last()
                            .map_or("", alloc::string::String::as_str),
                        &mut Vec::new(),
                    )
            }
            ExprKind::Selector(name) => self.pure_member(
                name.segments
                    .last()
                    .map_or("", alloc::string::String::as_str),
                &mut Vec::new(),
            ),
            ExprKind::Arms(arms) => arms
                .iter()
                .all(|a| pure(a.body) && a.guard.is_none_or(pure)),
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                pure(*condition)
                    && pure(*then_body)
                    && else_body.is_none_or(pure)
            }
            ExprKind::Record { fields, .. } => {
                fields.iter().all(|f| pure(f.value))
            }
            ExprKind::Outcome { payload, .. } => payload.is_none_or(pure),
            ExprKind::With { .. } => false,
            _ => true,
        }
    }

    fn pure_name(&self, target: Option<&Target>) -> bool {
        let Some(target) = target else {
            return true;
        };
        if let Some((_, pure)) =
            self.purity.borrow().iter().find(|(t, _)| t == target)
        {
            return *pure;
        }
        // Cache only a completed reachability walk: a back edge alone is not proof.
        let pure = self.pure_target(Some(target), &mut Vec::new());
        self.purity.borrow_mut().push((target.clone(), pure));
        pure
    }

    fn pure_tree(
        &self,
        module: &Module,
        resolved: &crate::resolve::Resolved,
        id: ExprId,
        visited: &mut Vec<Target>,
    ) -> bool {
        let span = module.expressions[id.0].span;
        resolved
            .expressions()
            .filter(|(id, _)| {
                let s = module.expressions[id.0].span;
                span.start <= s.start && s.end <= span.end
            })
            .all(|(_, target)| {
                self.pure_scoped(module, resolved, target, visited)
            })
            && module
                .expressions
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    span.start <= e.span.start && e.span.end <= span.end
                })
                .all(|(_, e)| {
                    if let ExprKind::Binary { op, left, .. } = e.kind {
                        let ty = self.expression_type(module, left);
                        self.pure_member_of(
                            operator_member(op),
                            ty.as_ref().and_then(type_head),
                            visited,
                        )
                    } else {
                        true
                    }
                })
    }

    fn expression_type(
        &self,
        module: &Module,
        id: ExprId,
    ) -> Option<super::Type> {
        if core::ptr::eq(module, self.module) {
            return self.typed.expr(id).cloned();
        }
        if let Some(line) = self.frame.and_then(|f| {
            f.lines
                .iter()
                .find(|l| core::ptr::eq(&raw const l.module, module))
        }) {
            return line.typed.expr(id).cloned();
        }
        let (stem, _) =
            self.files.iter().find(|(_, m)| core::ptr::eq(m, module))?;
        self.cache.borrow().get(stem)?.typed.expr(id).cloned()
    }

    fn pure_scoped(
        &self,
        module: &Module,
        resolved: &crate::resolve::Resolved,
        target: &Target,
        visited: &mut Vec<Target>,
    ) -> bool {
        if core::ptr::eq(module, self.module) {
            return self.pure_target(Some(target), visited);
        }
        let origin = self
            .files
            .iter()
            .find(|(_, m)| core::ptr::eq(m, module))
            .map(|(n, _)| Origin::File(n.clone()))
            .or_else(|| {
                self.frame
                    .and_then(|f| {
                        f.lines.iter().position(|l| {
                            core::ptr::eq(&raw const l.module, module)
                        })
                    })
                    .map(Origin::Line)
            })
            .unwrap_or(Origin::This);
        match target {
            Target::Value {
                origin: Origin::This,
                name,
            } => self.pure_target(
                Some(&Target::Value {
                    origin,
                    name: name.clone(),
                }),
                visited,
            ),
            Target::Local { site } => {
                if let Some(value) = binding_site(module, *site) {
                    let key = Target::Value {
                        origin,
                        name: format!("@{}:{}", site.start, site.end),
                    };
                    if visited.contains(&key) {
                        return true;
                    }
                    visited.push(key);
                    self.pure_tree(module, resolved, value, visited)
                } else if matches!(origin, Origin::Line(_)) {
                    self.pure_target(Some(target), visited)
                } else {
                    true
                }
            }
            Target::Members { head, members } => {
                self.pure_scoped(module, resolved, head, visited)
                    && members.iter().all(|m| self.pure_member(m, visited))
            }
            _ => self.pure_target(Some(target), visited),
        }
    }

    fn pure_target(
        &self,
        target: Option<&Target>,
        visited: &mut Vec<Target>,
    ) -> bool {
        let Some(target) = target else {
            return true;
        };
        if visited.contains(target) {
            return true;
        }
        visited.push(target.clone());
        match target {
            Target::Value {
                origin: Origin::Prelude,
                name,
            } => !matches!(
                name.as_str(),
                "log" | "sin" | "cos" | "atan2" | "hypot"
            ),
            Target::Local { site } => {
                if let Some((module, resolved, value)) =
                    core::iter::once((self.module, self.resolved))
                        .chain(self.frame.into_iter().flat_map(|f| {
                            f.lines.iter().map(|l| (&l.module, &l.resolved))
                        }))
                        .find_map(|(m, r)| {
                            binding_site(m, *site).map(|v| (m, r, v))
                        })
                {
                    self.pure_tree(module, resolved, value, visited)
                } else {
                    true
                }
            }
            Target::Value { origin, name } => {
                let Some(module) = self.owner(origin) else {
                    return false;
                };
                let Some(value) = binding(module, name) else {
                    return false;
                };
                if *origin == Origin::This {
                    return self.pure_tree(
                        module,
                        self.resolved,
                        value,
                        visited,
                    );
                }
                if let Origin::Line(index) = origin {
                    return self.frame.is_some_and(|f| {
                        self.pure_tree(
                            module,
                            &f.lines[*index].resolved,
                            value,
                            visited,
                        )
                    });
                }
                if let Origin::File(stem) = origin {
                    return self.cache.borrow().get(stem).is_some_and(|c| {
                        self.pure_tree(module, &c.resolved, value, visited)
                    });
                }
                // A supplied written value is a host constant.
                true
            }
            Target::Members { head, members } => {
                self.pure_target(Some(head), visited)
                    && members
                        .iter()
                        .all(|member| self.pure_member(member, visited))
            }
            Target::TypeMember { ty, member } => {
                let owner = if let Target::Type { origin, name } = &**ty {
                    Some(crate::types::TypeName {
                        origin: origin.clone(),
                        name: name.clone(),
                    })
                } else {
                    None
                };
                self.pure_member_of(member, owner.as_ref(), visited)
            }
            _ => true,
        }
    }

    fn pure_member(&self, name: &str, visited: &mut Vec<Target>) -> bool {
        self.pure_member_of(name, None, visited)
    }
    fn pure_member_of(
        &self,
        name: &str,
        owner: Option<&crate::types::TypeName>,
        visited: &mut Vec<Target>,
    ) -> bool {
        if owner.is_none_or(|n| n.origin == Origin::Prelude)
            && matches!(name, "sin" | "cos" | "atan2" | "hypot")
        {
            return false;
        }
        let key = Target::Value {
            origin: owner.map_or(Origin::This, |n| n.origin.clone()),
            name: format!(
                "@member:{}:{name}",
                owner.map_or("*", |n| n.name.as_str())
            ),
        };
        if visited.contains(&key) {
            return true;
        }
        visited.push(key);
        let modules = core::iter::once((self.module, Origin::This))
            .chain(self.frame.into_iter().flat_map(|f| {
                f.lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (&l.module, Origin::Line(i)))
            }))
            .chain(self.files.iter().map(|(n, m)| (m, Origin::File(n.clone()))))
            .chain(
                self.declarations
                    .modules
                    .iter()
                    .map(|m| (&m.module, Origin::Host(m.name.clone()))),
            );
        for (module, origin) in modules {
            for entry in &module.entries {
                let Entry::Item(item) = entry else {
                    continue;
                };
                let ItemKind::Impl {
                    target, members, ..
                } = &item.kind
                else {
                    continue;
                };
                if owner.is_some_and(|owner| {
                    self.type_name(module, &origin, *target).as_ref()
                        != Some(owner)
                }) {
                    continue;
                }
                for member in members {
                    if let ImplMember::Named { name: n, value, .. } = member {
                        if n.segments.last().is_none_or(|n| n != name) {
                            continue;
                        }
                        let Some(value) = value else {
                            return false;
                        };
                        if let Origin::File(stem) = &origin {
                            if self.cache.borrow().get(stem).is_some_and(|c| {
                                !self.pure_tree(
                                    module,
                                    &c.resolved,
                                    *value,
                                    visited,
                                )
                            }) {
                                return false;
                            }
                        }
                        let resolved = match &origin {
                            Origin::This => Some(self.resolved),
                            Origin::Line(i) => {
                                self.frame.map(|f| &f.lines[*i].resolved)
                            }
                            _ => None,
                        };
                        if resolved.is_some_and(|r| {
                            !self.pure_tree(module, r, *value, visited)
                        }) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    pub(super) fn demands(&mut self) {
        for (index, expression) in self.module.expressions.iter().enumerate() {
            if let ExprKind::Call {
                callee, arguments, ..
            } = &expression.kind
            {
                if let Some(demand) =
                    crate::resolve::timing(self.resolved.expr(*callee))
                {
                    if self.module.entries.iter().any(|e| matches!(e, Entry::Item(item) if matches!(item.kind, ItemKind::Type { .. }) && item.span.start <= expression.span.start && expression.span.end <= item.span.end)) { continue; }
                    if let [crate::ast::Argument {
                        kind: ArgumentKind::Positional(value),
                        ..
                    }] = arguments.as_slice()
                    {
                        self.demand(*value, demand, expression.span);
                        if demand == Phase::Compile {
                            self.typed.known[index].demanded = true;
                        }
                    }
                }
            }
            if let ExprKind::Bind {
                annotation: Some(annotation),
                value,
                ..
            } = expression.kind
            {
                for (demand, span) in self.type_timings(
                    self.module,
                    &Origin::This,
                    annotation,
                    &mut Vec::new(),
                ) {
                    self.demand(value, demand, span);
                    if demand == Phase::Compile {
                        self.typed.known[value.0].demanded = true;
                    }
                }
            }
        }
    }

    fn demand(&mut self, value: ExprId, demand: Phase, span: Span) {
        let phase = self.typed.known[value.0].phase;
        if demand == Phase::Compile && phase == Phase::Compile {
            let source = self.module.expressions[value.0].span;
            for (index, expression) in
                self.module.expressions.iter().enumerate()
            {
                if source.start <= expression.span.start
                    && expression.span.end <= source.end
                    && !self.module.expressions.iter().any(|arm| {
                        matches!(arm.kind, ExprKind::Arms(_))
                            && source.start <= arm.span.start
                            && arm.span.start <= expression.span.start
                            && expression.span.end <= arm.span.end
                            && expression.span != arm.span
                    })
                {
                    self.typed.known[index].demanded = true;
                }
            }
        }
        if phase > demand {
            let source = self.module.expressions[value.0].span;
            let latest = self
                .module
                .expressions
                .iter()
                .enumerate()
                .find_map(|(i, e)| {
                    if source.start <= e.span.start
                        && e.span.end <= source.end
                        && self.typed.known[i].phase == phase
                    {
                        if let ExprKind::Name(name) = &e.kind {
                            return Some(name.segments.join("."));
                        }
                    }
                    None
                })
                .unwrap_or_else(|| alloc::string::String::from("expression"));
            self.errors.push(Error { span, reason: TypeReason::Limit { message: format!("timing requires {demand}; latest input `{latest}` is known at {phase}") } });
        }
    }

    pub(super) fn type_timings(
        &self,
        module: &Module,
        origin: &Origin,
        id: TypeId,
        visited: &mut Vec<crate::types::TypeName>,
    ) -> Vec<(Phase, Span)> {
        match &module.types[id.0].kind {
            TypeKind::Both(parts) => parts
                .iter()
                .flat_map(|id| self.type_timings(module, origin, *id, visited))
                .collect(),
            TypeKind::Named { .. } => {
                let Some(name) = self.type_name(module, origin, id) else {
                    return Vec::new();
                };
                if name.origin == Origin::Prelude {
                    if name.name == "Compile" {
                        return alloc::vec![(
                            Phase::Compile,
                            module.types[id.0].span
                        )];
                    }
                    if name.name == "Init" {
                        return alloc::vec![(
                            Phase::Init,
                            module.types[id.0].span
                        )];
                    }
                }
                if visited.contains(&name) {
                    return Vec::new();
                }
                visited.push(name.clone());
                self.declaration(&name)
                    .and_then(|(m, b, _)| {
                        b.map(|id| {
                            self.type_timings(m, &name.origin, id, visited)
                        })
                    })
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

pub(super) fn binding(module: &Module, name: &str) -> Option<ExprId> {
    module.entries.iter().rev().find_map(|entry| {
        let Entry::Line(id) = entry else { return None; };
        let ExprKind::Bind { pattern, value, .. } = module.expressions[id.0].kind else { return None; };
        matches!(&module.patterns[pattern.0].kind, PatternKind::Name(n) if n.segments.last().is_some_and(|n| n == name)).then_some(value)
    })
}
fn binding_site(module: &Module, site: Span) -> Option<ExprId> {
    module.expressions.iter().find_map(|e| {
        let ExprKind::Bind { pattern, value, .. } = e.kind else {
            return None;
        };
        (module.patterns[pattern.0].span == site).then_some(value)
    })
}

impl Pass<'_> {
    pub(super) fn target_in(
        &self,
        module: &Module,
        origin: &Origin,
        id: ExprId,
    ) -> Option<Target> {
        if core::ptr::eq(module, self.module) {
            return self.resolved.expr(id).cloned();
        }
        match origin {
            Origin::Line(i) => self
                .frame
                .and_then(|f| f.lines[*i].resolved.expr(id))
                .cloned(),
            Origin::File(stem) => self
                .cache
                .borrow()
                .get(stem)
                .and_then(|c| c.resolved.expr(id))
                .cloned(),
            _ => {
                if let ExprKind::Name(n) = &module.expressions[id.0].kind {
                    if n.segments.len() == 1
                        && matches!(n.segments[0].as_str(), "Compile" | "Init")
                    {
                        Some(Target::Type {
                            origin: Origin::Prelude,
                            name: n.segments[0].clone(),
                        })
                    } else if n.segments.len() == 2
                        && self
                            .declarations
                            .get_module(&n.segments[0])
                            .is_some()
                    {
                        Some(Target::Value {
                            origin: Origin::Host(n.segments[0].clone()),
                            name: n.segments[1].clone(),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }
    pub(super) fn timed_test(
        &self,
        module: &Module,
        origin: &Origin,
        id: ExprId,
    ) -> Option<(Phase, ExprId)> {
        if let ExprKind::Group(ids) = &module.expressions[id.0].kind {
            return ids
                .last()
                .and_then(|id| self.timed_test(module, origin, *id));
        }
        let ExprKind::Call {
            callee, arguments, ..
        } = &module.expressions[id.0].kind
        else {
            return None;
        };
        let phase = crate::resolve::timing(
            self.target_in(module, origin, *callee).as_ref(),
        )?;
        let [crate::ast::Argument {
            kind: ArgumentKind::Positional(value),
            ..
        }] = arguments.as_slice()
        else {
            return None;
        };
        Some((phase, *value))
    }
    pub(super) fn test_value(
        &self,
        module: &Module,
        origin: &Origin,
        id: ExprId,
        known: &super::Known,
    ) -> super::Known {
        if let Some((_, value)) = self.timed_test(module, origin, id) {
            return self.test_value(module, origin, value, known);
        }
        match &module.expressions[id.0].kind {
            ExprKind::Name(n)
                if n.segments.first().is_none_or(|n| n != "self") =>
            {
                self.target(self.target_in(module, origin, id).as_ref())
            }
            ExprKind::Binary {
                op, left, right, ..
            } => super::binary(
                *op,
                &self.test_value(module, origin, *left, known),
                &self.test_value(module, origin, *right, known),
            ),
            ExprKind::Pipe {
                op, left, right, ..
            } if matches!(
                op,
                crate::ast::PipeOp::And | crate::ast::PipeOp::Or
            ) =>
            {
                super::bool_op(
                    *op,
                    &self.test_value(module, origin, *left, known),
                    &self.test_value(module, origin, *right, known),
                )
            }
            ExprKind::Group(ids) => {
                ids.last().map_or_else(super::Known::default, |id| {
                    self.test_value(module, origin, *id, known)
                })
            }
            _ => super::test(module, id, known),
        }
    }
}

impl Pass<'_> {
    pub(super) fn timing_places(&mut self) {
        let module = self.module;
        let mut allowed = Vec::new();
        for expression in &module.expressions {
            if let ExprKind::Bind {
                annotation: Some(id),
                ..
            } = expression.kind
            {
                timing_parts(module, id, &mut allowed);
            }
        }
        for entry in &module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Type { base: Some(id), .. } = item.kind {
                    timing_parts(module, id, &mut allowed);
                }
            }
        }
        for (index, syntax) in module.types.iter().enumerate() {
            if let TypeKind::Not(inner) = syntax.kind {
                let ty = self.syntax_type(module, &Origin::This, inner);
                if self.has_timed_test(&ty, &mut Vec::new()) {
                    self.errors.push(Error {
                        span: syntax.span,
                        reason: TypeReason::Limit {
                            message: alloc::string::String::from(
                                "negation of a type with a timed test",
                            ),
                        },
                    });
                }
            }
            if matches!(syntax.kind, TypeKind::Named { .. })
                && !allowed.contains(&TypeId(index))
                && !self
                    .type_timings(
                        module,
                        &Origin::This,
                        TypeId(index),
                        &mut Vec::new(),
                    )
                    .is_empty()
            {
                self.errors.push(Error {
                    span: syntax.span,
                    reason: TypeReason::Limit {
                        message: alloc::string::String::from(
                            "a timing limits only a binding annotation",
                        ),
                    },
                });
            }
        }
    }
    fn has_timed_test(
        &self,
        ty: &super::Type,
        visited: &mut Vec<crate::types::TypeName>,
    ) -> bool {
        match ty {
            super::Type::Named { name, .. } => {
                if visited.contains(name) {
                    return false;
                }
                visited.push(name.clone());
                self.declaration(name).is_some_and(|(m,b,limits)| limits.iter().any(|l| matches!(l, crate::ast::Limit::Test(id) if self.timed_test(m, &name.origin, *id).is_some())) || b.is_some_and(|id| self.has_timed_test(&self.syntax_type(m, &name.origin, id), visited)))
            }
            super::Type::Both(parts) | super::Type::Either(parts) => {
                parts.iter().any(|p| self.has_timed_test(p, visited))
            }
            super::Type::Not(inner)
            | super::Type::Limited { name: inner, .. } => {
                self.has_timed_test(inner, visited)
            }
            _ => false,
        }
    }
}
fn timing_parts(module: &Module, id: TypeId, allowed: &mut Vec<TypeId>) {
    allowed.push(id);
    if let TypeKind::Both(parts) = &module.types[id.0].kind {
        for part in parts {
            timing_parts(module, *part, allowed);
        }
    }
}

fn type_head(ty: &super::Type) -> Option<&crate::types::TypeName> {
    match ty {
        super::Type::Named { name, .. } => Some(name),
        super::Type::Limited { meaning, .. } => type_head(meaning),
        _ => None,
    }
}
const fn operator_member(op: crate::ast::BinaryOp) -> &'static str {
    use crate::ast::BinaryOp as B;
    match op {
        B::Add => "add",
        B::Subtract => "sub",
        B::Multiply => "mul",
        B::Divide => "div",
        B::Remainder => "rem",
        B::Concat => "concat",
        B::Equal | B::NotEqual => "eq",
        B::Less | B::LessEqual | B::Greater | B::GreaterEqual => "cmp",
    }
}
