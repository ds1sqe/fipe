use super::{
    Error, ExprId, ExprKind, Module, Origin, Pass, Phase, Span, Target,
    TypeReason,
};
use crate::ast::{ArgumentKind, Entry, ImplMember, ItemKind, TypeId, TypeKind};
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
            ExprKind::Literal(_) => {
                if self.pure_expression(id) {
                    Phase::Compile
                } else {
                    self.site_phase(expression.span)
                }
            }
            ExprKind::Word(_)
            | ExprKind::Path(_)
            | ExprKind::Selector(_)
            | ExprKind::Arms(_) => Phase::Compile,
            ExprKind::Name(_) | ExprKind::Instance { .. } => {
                self.target_phase(self.resolved.expr(id), expression.span)
            }
            ExprKind::Group(ids)
            | ExprKind::Block(ids)
            | ExprKind::Tuple(ids) => latest(ids),
            ExprKind::List(ids) => {
                latest(ids).max(if self.pure_expression(id) {
                    Phase::Compile
                } else {
                    self.site_phase(expression.span)
                })
            }
            ExprKind::Bind { value, .. } | ExprKind::Not { value, .. } => {
                get(*value)
            }
            ExprKind::Field { value, .. } => {
                get(*value).max(if self.pure_expression(id) {
                    Phase::Compile
                } else {
                    self.site_phase(expression.span)
                })
            }
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
        self.contexts
            .get(&span.start)
            .map_or(Phase::Eval, |context| context.0)
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
                if self
                    .owner(origin)
                    .and_then(|m| self.bindings_of(m).name(name))
                    .is_some()
                {
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
            Some(Target::TypeMember { .. } | Target::Constructor { .. }) => {
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
        if !self.form_purity(self.module, self.resolved, id, &mut Vec::new()) {
            return false;
        }
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

    fn form_purity(
        &self,
        module: &Module,
        resolved: &crate::resolve::Resolved,
        id: ExprId,
        visited: &mut Vec<Target>,
    ) -> bool {
        let output_type = self.expression_type(module, id);
        let mut output = output_type.as_ref();
        let input_type = super::first_input(module, id)
            .and_then(|input| self.expression_type(module, input));
        let input = input_type.as_ref();
        let members: &[&str] = match &module.expressions[id.0].kind {
            ExprKind::Literal(_) => &["literal"],
            ExprKind::List(_)
                if !matches!(output, Some(super::Type::Array { .. })) =>
            {
                &["sequence"]
            }
            ExprKind::Call { .. }
            | ExprKind::Pipe {
                op: crate::ast::PipeOp::Plain,
                ..
            } => {
                match super::call_head(module, id)
                    .and_then(|head| resolved.expr(head))
                {
                    Some(Target::Value {
                        origin: Origin::Prelude,
                        name,
                    }) => match name.as_str() {
                        "sum" => &["add", "zero"],
                        "show" => {
                            output = input;
                            &["show"]
                        }
                        "to" => &["from"],
                        "try" | "read" => {
                            if let Some(crate::types::Type::Named {
                                arguments,
                                ..
                            }) = output
                            {
                                output = arguments.first();
                            }
                            if name == "try" {
                                &["try_from"]
                            } else {
                                &["read"]
                            }
                        }
                        _ => &[],
                    },
                    _ => &[],
                }
            }
            _ => &[],
        };
        members.iter().all(|member| {
            self.pure_member_of(member, output.and_then(type_head), visited)
        })
    }

    fn pure_name(&self, target: Option<&Target>) -> bool {
        self.pure_target(target, &mut Vec::new())
    }

    pub(super) fn pure_test(&self, id: ExprId) -> bool {
        self.pure_tree(self.module, self.resolved, id, &mut Vec::new())
    }

    fn pure_tree(
        &self,
        module: &Module,
        resolved: &crate::resolve::Resolved,
        id: ExprId,
        visited: &mut Vec<Target>,
    ) -> bool {
        self.pure_region(
            module,
            resolved,
            module.expressions[id.0].span,
            id,
            visited,
        )
    }

    fn pure_region(
        &self,
        module: &Module,
        resolved: &crate::resolve::Resolved,
        span: Span,
        id: ExprId,
        visited: &mut Vec<Target>,
    ) -> bool {
        let start = module.expressions[..=id.0]
            .partition_point(|e| e.span.end <= span.start);
        (start..=id.0).all(|index| {
            let expression = &module.expressions[index];
            if expression.span.start < span.start
                || expression.span.end > span.end
            {
                return true;
            }
            let pure = resolved.expr(ExprId(index)).is_none_or(|target| {
                self.pure_scoped(module, resolved, target, visited)
            });
            pure && self.form_purity(module, resolved, ExprId(index), visited)
                && if let ExprKind::Binary { op, left, .. } = expression.kind {
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
        self.cache.files.borrow().get(stem)?.typed.expr(id).cloned()
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
                if let Some(value) = self.bindings_of(module).site(*site) {
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
        let Some(target) = target else { return true };
        if let Some(pure) = self.purity.borrow().get(target) {
            return *pure;
        }
        if let Target::Local { site } = target {
            if let Some(fact) = self
                .frame
                .and_then(|f| f.facts.get(&(site.start, site.end)))
            {
                return fact.pure;
            }
        }
        let cycles = self.purity_cycles.get();
        let depth = visited.len();
        let pure = self.pure_target_uncached(Some(target), visited);
        visited.truncate(depth);
        if !pure || self.purity_cycles.get() == cycles {
            self.purity.borrow_mut().insert(target.clone(), pure);
        }
        pure
    }

    fn pure_target_uncached(
        &self,
        target: Option<&Target>,
        visited: &mut Vec<Target>,
    ) -> bool {
        let Some(target) = target else {
            return true;
        };
        if visited.contains(target) {
            self.purity_cycles.set(self.purity_cycles.get() + 1);
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
                // This line's sites, then the one accepted line whose
                // text holds the site: lines lie in order in the session.
                let found = self
                    .bindings_of(self.module)
                    .site(*site)
                    .map(|value| (self.module, self.resolved, value))
                    .or_else(|| {
                        let lines = &self.frame?.lines;
                        let after = lines.partition_point(|line| {
                            line.module.span.start <= site.start
                        });
                        let line = lines.get(after.checked_sub(1)?)?;
                        line.bindings
                            .site(*site)
                            .map(|value| (&line.module, &line.resolved, value))
                    });
                if let Some((module, resolved, value)) = found {
                    self.pure_tree(module, resolved, value, visited)
                } else {
                    true
                }
            }
            Target::Value { origin, name } => {
                let Some(module) = self.owner(origin) else {
                    return false;
                };
                let Some(value) = self.bindings_of(module).name(name) else {
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
                    return self.cache.files.borrow().get(stem).is_some_and(
                        |c| self.pure_tree(module, &c.resolved, value, visited),
                    );
                }
                // A supplied written value is a host constant.
                true
            }
            Target::Constructor { origin, ty, .. } => {
                self.pure_constructor(origin, ty, visited)
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

    fn pure_constructor(
        &self,
        origin: &Origin,
        ty: &str,
        visited: &mut Vec<Target>,
    ) -> bool {
        let mut modules =
            core::iter::once((self.module, Origin::This))
                .chain(self.frame.into_iter().flat_map(|f| {
                    f.lines
                        .iter()
                        .enumerate()
                        .map(|(i, line)| (&line.module, Origin::Line(i)))
                }))
                .chain(self.files.iter().map(|(name, module)| {
                    (module, Origin::File(name.clone()))
                }));
        modules.all(|(module, home)| {
            let arms: Vec<_> = module
                .entries
                .iter()
                .filter_map(|entry| {
                    let Entry::Item(item) = entry else {
                        return None;
                    };
                    let ItemKind::Impl {
                        target, members, ..
                    } = &item.kind
                    else {
                        return None;
                    };
                    self.type_name(module, &home, *target)
                        .is_some_and(|name| {
                            name.origin == *origin && name.name == ty
                        })
                        .then_some(members)
                })
                .flatten()
                .filter_map(|member| match member {
                    ImplMember::Constructor { arm, .. } => Some(arm),
                    ImplMember::Named { .. } => None,
                })
                .collect();
            if arms.is_empty() {
                return true;
            }
            let mut pure = |resolved| {
                arms.iter().all(|arm| {
                    self.pure_region(
                        module, resolved, arm.span, arm.body, visited,
                    )
                })
            };
            match &home {
                Origin::This => pure(self.resolved),
                Origin::Line(index) => self
                    .frame
                    .is_some_and(|frame| pure(&frame.lines[*index].resolved)),
                Origin::File(stem) => self
                    .cache
                    .files
                    .borrow()
                    .get(stem)
                    .is_some_and(|file| pure(&file.resolved)),
                _ => false,
            }
        })
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
        if owner.is_some_and(|owner| owner.origin == Origin::Prelude)
            && matches!(name, "literal" | "sequence")
        {
            return true;
        }
        let key = Target::Value {
            origin: owner.map_or(Origin::This, |n| n.origin.clone()),
            name: format!(
                "@member:{}:{name}",
                owner.map_or("*", |n| n.name.as_str())
            ),
        };
        if visited.contains(&key) {
            self.purity_cycles.set(self.purity_cycles.get() + 1);
            return true;
        }
        if let Some(pure) = self.purity.borrow().get(&key) {
            return *pure;
        }
        let cycles = self.purity_cycles.get();
        visited.push(key.clone());
        let pure = self.pure_member_uncached(name, owner, visited);
        if !pure || self.purity_cycles.get() == cycles {
            self.purity.borrow_mut().insert(key, pure);
        }
        pure
    }

    /// Whether every written `name` member of `owner` (of any type when
    /// `None`) is pure; only accepted lines that write an `impl` are
    /// read.
    fn pure_member_uncached(
        &self,
        name: &str,
        owner: Option<&crate::types::TypeName>,
        visited: &mut Vec<Target>,
    ) -> bool {
        let modules = core::iter::once((self.module, Origin::This))
            .chain(self.frame.into_iter().flat_map(|f| {
                f.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.bindings.impls)
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
                            if self.cache.files.borrow().get(stem).is_some_and(
                                |c| {
                                    !self.pure_tree(
                                        module,
                                        &c.resolved,
                                        *value,
                                        visited,
                                    )
                                },
                            ) {
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

    fn demand(&mut self, value: ExprId, demand: Phase, _span: Span) {
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
            let latest = self.module.expressions.iter().enumerate().find_map(
                |(i, e)| {
                    if source.start <= e.span.start && e.span.end <= source.end
                    {
                        if let ExprKind::Name(name) = &e.kind {
                            if self.typed.known[i].phase == phase
                                || !self.typed.known[i].pure
                            {
                                return Some((
                                    name.segments.join("."),
                                    e.span,
                                    self.typed.known[i].phase < phase,
                                ));
                            }
                        }
                    }
                    if source.start <= e.span.start
                        && e.span.end <= source.end
                        && !self.typed.known[i].pure
                    {
                        let name = match &e.kind {
                            ExprKind::Field { field, .. } => Some(field),
                            _ => super::call_head(self.module, ExprId(i))
                                .and_then(|callee| {
                                    match &self.module.expressions[callee.0]
                                        .kind
                                    {
                                        ExprKind::Name(name) => Some(name),
                                        _ => None,
                                    }
                                }),
                        };
                        if let Some(name) = name {
                            return Some((
                                name.segments.join("."),
                                name.span,
                                true,
                            ));
                        }
                    }
                    None
                },
            );
            let (latest, at, impure) = latest.unwrap_or_else(|| {
                (alloc::string::String::from("expression"), source, false)
            });
            let message = if impure {
                format!(
                    "timing requires {demand}; input `{latest}` is not pure"
                )
            } else {
                format!("timing requires {demand}; latest input `{latest}` is known at {phase}")
            };
            self.errors.push(Error {
                span: at,
                reason: TypeReason::Limit { message },
            });
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
        let target = match origin {
            Origin::Line(i) => self
                .frame
                .and_then(|f| f.lines[*i].resolved.expr(id))
                .cloned(),
            Origin::File(stem) => self
                .cache
                .files
                .borrow()
                .get(stem)
                .and_then(|c| c.resolved.expr(id))
                .cloned(),
            _ => None,
        };
        target.or_else(|| {
            let ExprKind::Name(n) = &module.expressions[id.0].kind else {
                return None;
            };
            if n.segments.len() == 1
                && matches!(n.segments[0].as_str(), "Compile" | "Init")
            {
                Some(Target::Type {
                    origin: Origin::Prelude,
                    name: n.segments[0].clone(),
                })
            } else if n.segments.len() == 2
                && self.declarations.get_module(&n.segments[0]).is_some()
            {
                Some(Target::Value {
                    origin: Origin::Host(n.segments[0].clone()),
                    name: n.segments[1].clone(),
                })
            } else {
                None
            }
        })
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
            } => super::test_binary(
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
            ExprKind::Not { value, .. } => {
                let mut result = self.test_value(module, origin, *value, known);
                result.boolean = result.boolean.map(|b| !b);
                result
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
