use super::{
    free_parameters, ir, lines, owner, Body, Index, Origin, Rc, Scope, Source,
    String, Type, Vec,
};

impl Index {
    pub fn emit(
        &mut self,
        module: &crate::ast::Module,
        resolved: crate::resolve::Resolved,
        typed: crate::typed::Typed,
        declarations: &crate::decl::Declarations,
        files: &[(String, crate::ast::Module)],
        cache: &crate::infer::files::FileCache,
    ) -> ir::Unit {
        let home = self.home.clone().unwrap_or(Origin::This);
        self.source_modules.insert(home, Rc::new(module.clone()));
        let entry_start = self.tables.borrow().entries.len();
        let mut sources = sources(
            module,
            resolved,
            typed,
            declarations,
            files,
            cache,
            &self.source_modules,
        );
        sources[0].home = self.home.clone().unwrap_or(Origin::This);
        sources.retain(|source| {
            !self
                .emitted
                .contains(&(source.home.clone(), source.roots.clone()))
        });
        let scopes: Vec<_> =
            sources.iter().map(|source| self.reserve(source)).collect();
        for (source, scope) in sources.iter().zip(&scopes) {
            for ((start, end), _) in &source.typed.frozen {
                let site = crate::token::Span {
                    start: *start,
                    end: *end,
                };
                if let Some(slot) = scope.slots.get(&site) {
                    self.slots.insert((source.home.clone(), site), *slot);
                }
            }
        }
        let bodies: Vec<_> = sources
            .iter()
            .zip(&scopes)
            .map(|(source, scope)| self.definitions(source, scope))
            .collect();
        for source in &sources {
            for dispatch in &source.typed.dispatch {
                let key = (dispatch.name.clone(), dispatch.receiverless);
                if !self.dispatch.contains_key(&key) {
                    let index = self.tables.borrow().dispatch.len();
                    self.tables.borrow_mut().dispatch.push(ir::Dispatch {
                        label: dispatch.name.clone(),
                        rows: Vec::new(),
                    });
                    self.dispatch.insert(key, index);
                }
            }
        }
        for source in &sources {
            self.dispatch_rows(source);
        }
        for (source, scope) in sources.iter().zip(&scopes) {
            self.tests(source, scope, &sources, &scopes);
        }
        for ((source, scope), bodies) in
            sources.iter().zip(&scopes).zip(&bodies)
        {
            self.fill(source, scope, bodies);
            self.captures(scope);
        }
        self.modules(&sources, &scopes);
        let main_kind =
            self.sequence(&sources[0], &scopes[0], &sources[0].roots);
        let main = match main_kind {
            ir::Kind::Block(lines) => lines,
            kind => {
                let ty = sources[0].typed.main.as_ref().expect("main type");
                alloc::vec![self.node(
                    &sources[0].home,
                    sources[0].module.span,
                    ty,
                    kind
                )]
            }
        };
        let mut plan = Vec::new();
        for (source, scope) in sources.iter().zip(&scopes) {
            self.entries(source, scope, &mut plan);
        }
        let mut parameters = Vec::new();
        if let Some(ty) = &sources[0].typed.main {
            free_parameters(&self.ty(&sources[0].home, ty), &mut parameters);
        }
        for source in &sources {
            self.emitted
                .push((source.home.clone(), source.roots.clone()));
        }
        self.remember(&sources[0], &scopes[0]);
        ir::Unit {
            tables: self.tables.clone(),
            parameters,
            main,
            plan: self.tables.borrow().entries[entry_start..].to_vec(),
        }
    }

    fn remember(&mut self, source: &Source<'_>, scope: &Scope) {
        if self.home.is_some() {
            for ((start, end), _) in &source.typed.frozen {
                let site = crate::token::Span {
                    start: *start,
                    end: *end,
                };
                if let Some(slot) = scope.slots.get(&site) {
                    self.visible.insert(site, *slot);
                }
            }
        }
    }

    fn modules(&mut self, sources: &[Source<'_>], scopes: &[Scope]) {
        for (source, scope) in sources.iter().zip(scopes) {
            if let Origin::File(label) = &source.home {
                let kind = self.sequence(source, scope, &source.roots);
                let ir::Kind::Block(lines) = kind else {
                    unreachable!("module sequence")
                };
                self.tables.borrow_mut().modules.push(ir::Module {
                    label: label.clone(),
                    lines,
                });
            }
        }
    }

    pub(super) fn fill(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        bodies: &[Body],
    ) {
        for (index, node) in scope.nodes.iter().enumerate() {
            let Some(node) = node else { continue };
            let kind =
                self.expression(source, scope, crate::ast::ExprId(index));
            self.tables.borrow_mut().nodes[*node].kind = kind;
            if let Some(cast) = &source.typed.casts[index] {
                let mut original = self.tables.borrow().nodes[*node].clone();
                original.ty = self.ty(&source.home, &cast.from);
                let value = self.tables.borrow().nodes.len();
                self.tables.borrow_mut().nodes.push(original);
                let from = self.ty(&source.home, &cast.from);
                let to = self.ty(&source.home, &cast.to);
                let kind = if cast.row == crate::typed::CastRow::Pack {
                    ir::Kind::Pack { tag: from, value }
                } else {
                    let row = match cast.row {
                        crate::typed::CastRow::Number => ir::CastRow::Number,
                        crate::typed::CastRow::Read => ir::CastRow::Read,
                        crate::typed::CastRow::Show => ir::CastRow::Show,
                        crate::typed::CastRow::Codec => ir::CastRow::Codec,
                        crate::typed::CastRow::Bytes => ir::CastRow::Bytes,
                        crate::typed::CastRow::Pack => unreachable!(),
                    };
                    let operation = cast.operation.as_ref().map(|operation| {
                        self.selected(
                            source,
                            source.module.expressions[index].span,
                            &operation.selected,
                            &operation.meanings,
                            &operation.ty,
                            Some(value),
                        )
                    });
                    ir::Kind::Cast {
                        operation,
                        row,
                        from,
                        to: to.clone(),
                        value,
                    }
                };
                self.tables.borrow_mut().nodes[*node].ty = to;
                self.tables.borrow_mut().nodes[*node].kind = kind;
            }
        }
        for body in bodies {
            if body.expression.is_none() {
                let node =
                    self.tables.borrow().definitions[body.definition].body;
                let kind = self.function(source, scope, &body.arms);
                self.tables.borrow_mut().nodes[node].kind = kind;
            }
            let definition =
                self.tables.borrow().definitions[body.definition].clone();
            let mut body = definition.body;
            loop {
                let next = match &self.tables.borrow().nodes[body].kind {
                    ir::Kind::Block(parts) if parts.len() == 1 => {
                        Some(parts[0])
                    }
                    _ => None,
                };
                let Some(next) = next else { break };
                body = next;
            }
            if let ir::Kind::Function {
                parameters, hidden, ..
            } = &mut self.tables.borrow_mut().nodes[body].kind
            {
                *parameters = definition.parameters;
                *hidden = definition.hidden;
            }
        }
    }

    pub(super) fn captures(&self, scope: &Scope) {
        use alloc::collections::BTreeMap;
        let mut tables = self.tables.borrow_mut();
        let functions: Vec<_> = scope
            .nodes
            .iter()
            .flatten()
            .copied()
            .filter(|index| {
                matches!(tables.nodes[*index].kind, ir::Kind::Function { .. })
            })
            .collect();
        let Some(first) = functions.first() else {
            return;
        };
        let namespace = tables.nodes[*first].namespace;
        let sites: BTreeMap<_, _> = scope
            .slots
            .iter()
            .map(|(site, slot)| (*slot, *site))
            .collect();
        let mut events: Vec<_> =
            functions.iter().map(|index| (*index, true)).collect();
        events.extend(tables.nodes.iter().enumerate().filter_map(
            |(index, node)| {
                (node.namespace == namespace
                    && matches!(
                        node.kind,
                        ir::Kind::Local { .. } | ir::Kind::Capture { .. }
                    ))
                .then_some((index, false))
            },
        ));
        events.sort_by_key(|(index, function)| {
            let span = tables.nodes[*index].span;
            (span.start, core::cmp::Reverse(span.end), !function)
        });
        let mut enclosing: Vec<usize> = Vec::new();
        let mut captures: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, function) in events {
            let span = tables.nodes[index].span;
            while enclosing.last().is_some_and(|outer| {
                let outer = tables.nodes[*outer].span;
                !crate::check::within(span, outer) || span.start == outer.end
            }) {
                enclosing.pop();
            }
            if function {
                enclosing.push(index);
                continue;
            }
            let (ir::Kind::Local { slot, meanings }
            | ir::Kind::Capture { slot, meanings }) =
                tables.nodes[index].kind.clone()
            else {
                unreachable!()
            };
            let Some(site) = sites.get(&slot) else {
                continue;
            };
            let mut external = false;
            for outer in &enclosing {
                let span = tables.nodes[*outer].span;
                if !(span.start <= site.start && site.end <= span.end) {
                    let slots = captures.entry(*outer).or_default();
                    if !slots.contains(&slot) {
                        slots.push(slot);
                    }
                    external = true;
                }
            }
            if external {
                tables.nodes[index].kind = ir::Kind::Capture { slot, meanings };
            }
        }
        for function in functions {
            if let ir::Kind::Function {
                captures: stored, ..
            } = &mut tables.nodes[function].kind
            {
                *stored = captures.remove(&function).unwrap_or_default();
            }
        }
    }

    pub(super) fn definitions(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
    ) -> Vec<Body> {
        use crate::ast::{Entry, ExprKind, ImplMember, ItemKind, PatternKind};
        let mut bodies = Vec::new();
        for entry in &source.module.entries {
            match entry {
                Entry::Line(id) => {
                    let ExprKind::Bind { pattern, value, .. } =
                        source.module.expressions[id.0].kind
                    else {
                        continue;
                    };
                    let PatternKind::Name(name) =
                        &source.module.patterns[pattern.0].kind
                    else {
                        // A line's pattern binds names later lines read,
                        // so each is a value definition its main stores.
                        let Some(body) = scope.nodes[value.0] else {
                            continue;
                        };
                        let mut names = Vec::new();
                        pattern_names(&source.module, pattern, &mut names);
                        for name in names {
                            if scope.slots.contains_key(&name.span) {
                                self.definition(
                                    source,
                                    scope,
                                    name.span,
                                    name.segments.join("."),
                                    body,
                                );
                            }
                        }
                        continue;
                    };
                    if !scope.slots.contains_key(&name.span) {
                        continue;
                    }
                    let Some(body) = scope.nodes[value.0] else {
                        continue;
                    };
                    let definition = self.definition(
                        source,
                        scope,
                        name.span,
                        name.segments.join("."),
                        body,
                    );
                    bodies.push(Body {
                        definition,
                        expression: Some(value),
                        arms: Vec::new(),
                    });
                }
                Entry::Item(item) => {
                    let ItemKind::Impl { members, .. } = &item.kind else {
                        continue;
                    };
                    let mut arms = Vec::new();
                    for member in members {
                        match member {
                            ImplMember::Named {
                                name,
                                value: Some(value),
                                ..
                            } => {
                                let Some(body) = scope.nodes[value.0] else {
                                    continue;
                                };
                                let definition = self.definition(
                                    source,
                                    scope,
                                    name.span,
                                    name.segments.join("."),
                                    body,
                                );
                                bodies.push(Body {
                                    definition,
                                    expression: Some(*value),
                                    arms: Vec::new(),
                                });
                            }
                            ImplMember::Constructor { arm, .. } => {
                                arms.push(arm.clone());
                            }
                            ImplMember::Named { value: None, .. } => (),
                        }
                    }
                    if let Some(body) =
                        self.constructor_body(source, scope, item.span, arms)
                    {
                        bodies.push(body);
                    }
                }
            }
        }
        bodies
    }

    fn constructor_body(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        span: crate::token::Span,
        arms: Vec<crate::ast::Arm>,
    ) -> Option<Body> {
        let site = arms.first()?.span;
        if self.definitions.contains_key(&(source.home.clone(), site))
            || arms.iter().any(|arm| scope.nodes[arm.body.0].is_none())
        {
            return None;
        }
        let scheme = source.typed.schemes.get(&(site.start, site.end))?;
        let body = self.node(
            &source.home,
            span,
            &scheme.ty,
            ir::Kind::Block(Vec::new()),
        );
        let definition =
            self.definition(source, scope, site, String::from("Self"), body);
        Some(Body {
            definition,
            expression: None,
            arms,
        })
    }

    pub(super) fn definition(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        site: crate::token::Span,
        label: String,
        body: usize,
    ) -> usize {
        let key = (source.home.clone(), site);
        if let Some(index) = self.definitions.get(&key) {
            return *index;
        }
        let slot = scope.slots[&site];
        let signature = self.tables.borrow().slots[slot].clone();
        let namespace = self.namespace(&source.home);
        let index = self.tables.borrow().definitions.len();
        self.tables.borrow_mut().definitions.push(ir::Definition {
            namespace,
            label,
            span: site,
            parameters: signature.parameters,
            hidden: signature.hidden,
            ty: signature.ty,
            body,
            slot,
        });
        self.tables.borrow_mut().slots[slot].definition = Some(index);
        self.slots.insert(key.clone(), slot);
        self.definitions.insert(key, index);
        index
    }

    pub(super) fn sequence(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        lines: &[crate::ast::ExprId],
    ) -> ir::Kind {
        let mut parts = Vec::new();
        for (index, id) in lines.iter().enumerate() {
            if let crate::ast::ExprKind::Bind { pattern, value, .. } =
                source.module.expressions[id.0].kind
            {
                let span = source.module.patterns[pattern.0].span;
                let site = (span.start, span.end);
                if let Some(rebuild) = source.typed.rebuilds.get(&site) {
                    let remainder =
                        self.sequence(source, scope, &lines[index + 1..]);
                    let ir::Kind::Block(tail) = &remainder else {
                        unreachable!("sequence is a block")
                    };
                    let ty = tail.last().map_or(ir::Type::Unit, |node| {
                        self.tables.borrow().nodes[*node].ty.clone()
                    });
                    let body =
                        self.node(&source.home, span, &Type::Unit, remainder);
                    self.tables.borrow_mut().nodes[body].ty = ty;
                    let pattern = self.pattern(source, scope, pattern);
                    let kind = ir::Kind::Lift {
                        pattern,
                        value: scope.expression(value),
                        body,
                        rebuild: *rebuild,
                    };
                    let ty = &source.typed.rebuild_types[&site];
                    parts.push(self.node(&source.home, span, ty, kind));
                    break;
                }
            }
            parts.push(scope.expression(*id));
        }
        ir::Kind::Block(parts)
    }
}

/// The names a pattern binds, in written order.
fn pattern_names<'m>(
    module: &'m crate::ast::Module,
    pattern: crate::ast::PatternId,
    names: &mut Vec<&'m crate::ast::Name>,
) {
    use crate::ast::PatternKind as P;
    match &module.patterns[pattern.0].kind {
        P::Name(name) => names.push(name),
        P::Wildcard | P::Literal(_) | P::Default { .. } => (),
        P::Constructor { payload, .. } | P::Group(payload) => {
            pattern_names(module, *payload, names);
        }
        P::Tuple(parts) | P::List(parts) => {
            for part in parts {
                pattern_names(module, *part, names);
            }
        }
        P::Outcome { payload, .. } => {
            if let Some(payload) = payload {
                pattern_names(module, *payload, names);
            }
        }
        P::Record { fields, .. } => {
            for field in fields {
                match field.pattern {
                    Some(part) => pattern_names(module, part, names),
                    None => names.push(&field.name),
                }
            }
        }
    }
}

fn sources<'a>(
    module: &'a crate::ast::Module,
    resolved: crate::resolve::Resolved,
    typed: crate::typed::Typed,
    declarations: &'a crate::decl::Declarations,
    files: &'a [(String, crate::ast::Module)],
    cache: &crate::infer::files::FileCache,
    modules: &alloc::collections::BTreeMap<Origin, Rc<crate::ast::Module>>,
) -> Vec<Source<'a>> {
    let mut sources = alloc::vec![Source {
        declarations,
        files,
        module: Rc::new(module.clone()),
        resolved: Rc::new(resolved),
        typed: Rc::new(typed),
        home: Origin::This,
        roots: lines(module),
    }];
    let mut ordered = file_order(module, files);
    for stem in cache.files.borrow().keys() {
        let module = &files
            .iter()
            .find(|(name, _)| name == stem)
            .expect("checked file")
            .1;
        for dependency in file_order(module, files)
            .into_iter()
            .chain(core::iter::once(stem.clone()))
        {
            if !ordered.contains(&dependency) {
                ordered.push(dependency);
            }
        }
    }
    for stem in ordered {
        let Some(checked) = cache.files.borrow().get(&stem).cloned() else {
            continue;
        };
        let module = &files
            .iter()
            .find(|(name, _)| *name == stem)
            .expect("checked file")
            .1;
        sources.push(Source {
            declarations,
            files,
            module: Rc::new(module.clone()),
            resolved: checked.resolved.clone(),
            typed: checked.typed,
            home: Origin::File(stem.clone()),
            roots: lines(module),
        });
    }
    let mut cursor = 0;
    while cursor < sources.len() {
        let expressions = sources[cursor].typed.declared.clone();
        for expression in expressions {
            // Written defaults are instantiated by their owning call.
            if expression.origin == Origin::This {
                continue;
            }
            if sources[cursor]
                .typed
                .default_instances
                .values()
                .any(|default| {
                    default.origin == expression.origin
                        && default.value == expression.value
                })
            {
                continue;
            }
            if sources.iter().any(|source| {
                source.home == expression.origin
                    && source.roots.contains(&expression.value)
            }) {
                continue;
            }
            let module =
                modules.get(&expression.origin).cloned().unwrap_or_else(|| {
                    Rc::new(
                        owner(declarations, files, &expression.origin).clone(),
                    )
                });
            sources.push(Source {
                declarations,
                files,
                module,
                resolved: expression.resolved,
                typed: expression.typed,
                home: expression.origin,
                roots: alloc::vec![expression.value],
            });
        }
        cursor += 1;
    }
    sources
}

pub fn file_order(
    module: &crate::ast::Module,
    files: &[(String, crate::ast::Module)],
) -> Vec<String> {
    fn visit(
        module: &crate::ast::Module,
        files: &[(String, crate::ast::Module)],
        seen: &mut Vec<String>,
        ordered: &mut Vec<String>,
    ) {
        use crate::ast::{Entry, ItemKind};
        for entry in &module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Use { names, .. } = &item.kind else {
                continue;
            };
            for name in names {
                let Some(stem) = name.segments.first() else {
                    continue;
                };
                if seen.contains(stem) {
                    continue;
                }
                let Some((_, module)) =
                    files.iter().find(|(name, _)| name == stem)
                else {
                    continue;
                };
                seen.push(stem.clone());
                visit(module, files, seen, ordered);
                ordered.push(stem.clone());
            }
        }
    }
    let mut ordered = Vec::new();
    visit(module, files, &mut Vec::new(), &mut ordered);
    ordered
}
