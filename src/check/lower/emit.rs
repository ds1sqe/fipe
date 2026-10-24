use super::{
    free_parameters, ir, lines, owner, Body, Contribution, DefinitionInput,
    Index, Origin, Prepared, Rc, Readiness, Scope, Source, String, Type, Vec,
};

impl Index {
    pub(super) fn replay_plan(
        &self,
        source: &Source<'_>,
    ) -> (Vec<DefinitionInput>, Contribution) {
        let inputs = self.replay_inputs(source);
        let mut contribution =
            Self::contribution(source, &inputs, source.roots.clone(), false);
        contribution.replay = true;
        (inputs, contribution)
    }

    fn replay_ready(
        &self,
        source: &Source<'_>,
        recorded: &crate::typed::DeclaredExpression,
        earlier: &[crate::token::Span],
        ready: &Readiness<'_, '_>,
    ) -> bool {
        let Some(instance) = self.recorded_source(source, recorded) else {
            return false;
        };
        let (inputs, contribution) = self.replay_plan(&instance);
        self.source_ready(&instance, &inputs, earlier, &contribution, ready)
    }

    pub(super) fn replay_inputs(
        &self,
        source: &Source<'_>,
    ) -> Vec<DefinitionInput> {
        let contribution =
            Self::contribution(source, &[], source.roots.clone(), false);
        self.definition_inputs(source)
            .into_iter()
            .filter(|input| contribution.owned.contains(&input.site))
            .collect()
    }

    pub(super) fn contribution(
        source: &Source<'_>,
        inputs: &[DefinitionInput],
        mut roots: Vec<crate::ast::ExprId>,
        lexical: bool,
    ) -> Contribution {
        let mut patterns = Vec::new();
        for input in inputs {
            roots.extend(input.expression);
            for arm in &input.arms {
                roots.push(arm.body);
                roots.extend(arm.guard);
                patterns.extend(arm.patterns.iter().copied());
            }
        }
        let nodes = crate::infer::SourceNodes::from_roots(
            &source.module,
            roots,
            patterns,
        );
        let mut contribution = Contribution {
            nodes: crate::infer::SourceNodes::default(),
            owned: alloc::collections::BTreeSet::default(),
            lexical,
            replay: false,
        };
        Self::extend_contribution(source, &mut contribution, nodes);
        contribution
            .owned
            .extend(inputs.iter().map(|input| input.site));
        contribution
    }

    fn extend_contribution(
        source: &Source<'_>,
        contribution: &mut Contribution,
        nodes: crate::infer::SourceNodes,
    ) {
        for id in &nodes.patterns {
            let mut names = Vec::new();
            let pattern = crate::ast::PatternId(*id);
            if let crate::ast::PatternKind::Default { name, .. } =
                &source.module.patterns[*id].kind
            {
                names.push(name);
            } else {
                pattern_names(&source.module, pattern, &mut names);
            }
            contribution
                .owned
                .extend(names.into_iter().map(|name| name.span));
        }
        contribution.nodes.expressions.extend(nodes.expressions);
        contribution.nodes.patterns.extend(nodes.patterns);
        contribution.nodes.types.extend(nodes.types);
        contribution.nodes.sites.extend(nodes.sites);
    }

    /// A replay may capture a binding from an earlier accepted line. Its
    /// global site must still belong to that registered line and name the
    /// same published slot; foreign declaration syntax never uses this path.
    fn prior_line_alias(
        &self,
        source: &Source<'_>,
        contribution: &Contribution,
        site: crate::token::Span,
    ) -> Option<usize> {
        if !contribution.replay {
            return None;
        }
        let Origin::Line(line) = &source.home else {
            return None;
        };
        let visible = *self.store.visible().get(&site)?;
        self.store
            .source_modules()
            .range(Origin::Line(0)..Origin::Line(*line))
            .rev()
            .find_map(|(owner, module)| {
                (module.span.start <= site.start && site.end <= module.span.end)
                    .then(|| self.store.slots().get(&(owner.clone(), site)))
                    .flatten()
                    .filter(|slot| **slot == visible)
                    .copied()
            })
    }

    /// Exactly the local bindings read by this emitted contribution.
    fn executable_aliases(
        source: &Source<'_>,
        contribution: &Contribution,
    ) -> Vec<crate::token::Span> {
        fn local(
            target: &crate::resolve::Target,
        ) -> Option<crate::token::Span> {
            match target {
                crate::resolve::Target::Local { site } => Some(*site),
                crate::resolve::Target::Members { head, .. } => local(head),
                crate::resolve::Target::TypeMember { ty, .. } => local(ty),
                _ => None,
            }
        }
        source
            .resolved
            .expressions()
            .filter(|(id, _)| {
                contribution.nodes.expressions.contains(&id.0)
                    && source.typed.expr(*id).is_some()
            })
            .filter_map(|(_, target)| local(target))
            .collect()
    }

    fn aliases_ready(
        &self,
        source: &Source<'_>,
        contribution: &Contribution,
        available: &alloc::collections::BTreeSet<(Origin, crate::token::Span)>,
        earlier: &[crate::token::Span],
    ) -> bool {
        Self::executable_aliases(source, contribution)
            .into_iter()
            .all(|site| {
                contribution.owned.contains(&site)
                    || earlier.contains(&site)
                    || available.contains(&(source.home.clone(), site))
                    || self
                        .store
                        .slots()
                        .contains_key(&(source.home.clone(), site))
                    || self
                        .prior_line_alias(source, contribution, site)
                        .is_some()
                    || contribution.lexical
                        && self.store.visible().contains_key(&site)
            })
    }

    pub(super) fn resolve_aliases(
        &self,
        source: &Source<'_>,
        scope: &mut Scope,
        contribution: &Contribution,
        available: &alloc::collections::BTreeMap<
            (Origin, crate::token::Span),
            usize,
        >,
    ) {
        let sites: alloc::collections::BTreeSet<_> = source
            .typed
            .schemes
            .keys()
            .map(|(start, end)| crate::token::Span {
                start: *start,
                end: *end,
            })
            .chain(Self::executable_aliases(source, contribution))
            .collect();
        for site in sites {
            // Owned and supplied earlier bindings have already been installed.
            if scope.slots.contains_key(&site) {
                continue;
            }
            let key = (source.home.clone(), site);
            let slot = available
                .get(&key)
                .or_else(|| self.store.slots().get(&key))
                .copied()
                .or_else(|| self.prior_line_alias(source, contribution, site));
            if let Some(slot) = slot {
                scope.slots.insert(site, slot);
            }
        }
        let available = available.keys().cloned().collect();
        assert!(
            self.aliases_ready(
                source,
                contribution,
                &available,
                &scope.slots.keys().copied().collect::<Vec<_>>()
            ),
            "prepared executable aliases"
        );
    }

    pub fn emit(
        &mut self,
        module: &crate::ast::Module,
        resolved: crate::resolve::Resolved,
        typed: crate::typed::Typed,
        declarations: &crate::decl::Declarations,
        files: &[(String, crate::ast::Module)],
        cache: &crate::infer::files::FileCache,
    ) -> ir::Unit {
        let prepared = self
            .prepare(
                (module, self.home.clone().unwrap_or(Origin::This)),
                resolved,
                typed,
                declarations,
                files,
                cache,
            )
            .expect("checked source runtime dependencies");
        self.emit_prepared(prepared)
    }

    fn source_contributions(
        &self,
        sources: &[Source<'_>],
        definitions: &[Vec<DefinitionInput>],
    ) -> Vec<Contribution> {
        let mut contributions: Vec<_> = sources
            .iter()
            .zip(definitions)
            .enumerate()
            .map(|(index, (source, inputs))| {
                let roots = if index == 0 || source.initializes_file {
                    source.roots.clone()
                } else {
                    Vec::new()
                };
                Self::contribution(
                    source,
                    inputs,
                    roots,
                    index == 0 && self.home.is_some(),
                )
            })
            .collect();
        let mut pending: alloc::collections::VecDeque<_> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.clone(), Some(index)))
            .collect();
        let mut prepared_seen = alloc::collections::BTreeSet::new();
        let mut replay_seen = alloc::collections::BTreeSet::new();
        while let Some((source, prepared)) = pending.pop_front() {
            let replay;
            let contribution = if let Some(index) = prepared {
                if !prepared_seen.insert((
                    index,
                    Self::contribution_size(&contributions[index]),
                )) {
                    continue;
                }
                &contributions[index]
            } else {
                let key = (
                    Rc::as_ptr(&source.typed).addr(),
                    source.home.clone(),
                    source.parameters_home.clone(),
                    source.roots.iter().map(|id| id.0).collect::<Vec<_>>(),
                );
                if !replay_seen.insert(key) {
                    continue;
                }
                replay = self.replay_plan(&source);
                &replay.1
            };
            let mut defaults = Vec::new();
            for (value, place, _) in
                Self::executable_defaults(&source, contribution)
            {
                self.replay_default_sources(
                    &source,
                    value,
                    place,
                    &mut defaults,
                );
            }
            pending.extend(defaults.into_iter().map(|source| (source, None)));
            for test in &source.typed.tests {
                if test.expression.as_ref().is_some_and(|expression| {
                    self.emitted.iter().any(|(home, roots)| {
                        *home == expression.origin
                            && roots.contains(&expression.value)
                    })
                }) {
                    continue;
                }
                let index = test.expression.as_ref().map_or_else(
                    || prepared,
                    |expression| {
                        sources.iter().position(|candidate| {
                            candidate.home == expression.origin
                                && candidate.roots.contains(&expression.value)
                        })
                    },
                );
                if let Some(index) = index {
                    let before = Self::contribution_size(&contributions[index]);
                    let found = crate::infer::SourceNodes::from_roots(
                        &sources[index].module,
                        [test.value],
                        core::iter::empty(),
                    );
                    Self::extend_contribution(
                        &sources[index],
                        &mut contributions[index],
                        found,
                    );
                    contributions[index].owned.insert(test.self_site);
                    if Self::contribution_size(&contributions[index]) != before
                    {
                        pending
                            .push_back((sources[index].clone(), Some(index)));
                    }
                }
            }
        }
        contributions
    }

    fn contribution_size(
        contribution: &Contribution,
    ) -> (usize, usize, usize, usize, usize) {
        (
            contribution.nodes.expressions.len(),
            contribution.nodes.patterns.len(),
            contribution.nodes.types.len(),
            contribution.nodes.sites.len(),
            contribution.owned.len(),
        )
    }

    fn executable_defaults<'a>(
        source: &'a Source<'_>,
        contribution: &Contribution,
    ) -> Vec<(
        &'a crate::typed::DefaultValue,
        (crate::token::Span, usize),
        Vec<crate::token::Span>,
    )> {
        let mut defaults = Vec::new();
        for ((start, end), bound) in source.typed.bound.iter().chain(
            source.typed.accesses.iter().flat_map(|(site, accesses)| {
                accesses.iter().filter_map(move |access| {
                    access.bound.as_ref().map(|bound| (site, bound))
                })
            }),
        ) {
            if !contribution.nodes.sites.contains(&(*start, *end)) {
                continue;
            }
            let span = crate::token::Span {
                start: *start,
                end: *end,
            };
            for (spot, value) in &bound.defaults {
                let earlier =
                    super::defaults::earlier_spots(source, bound, span, *spot)
                        .into_iter()
                        .map(|(_, site)| site)
                        .collect();
                defaults.push((value, (span, *spot), earlier));
            }
        }
        for ((start, end), value) in &source.typed.derived_defaults {
            if contribution.nodes.sites.contains(&(*start, *end)) {
                defaults.push((
                    value,
                    (
                        crate::token::Span {
                            start: *start,
                            end: *end,
                        },
                        0,
                    ),
                    Vec::new(),
                ));
            }
        }
        defaults
    }

    fn replay_default_sources<'a>(
        &self,
        source: &Source<'a>,
        value: &crate::typed::DefaultValue,
        (span, spot): (crate::token::Span, usize),
        found: &mut Vec<Source<'a>>,
    ) {
        use crate::typed::DefaultValue;
        match value {
            DefaultValue::Record { fields, .. } => {
                for (spot, (_, _, value)) in fields.iter().enumerate() {
                    self.replay_default_sources(
                        source,
                        value,
                        (span, spot),
                        found,
                    );
                }
            }
            DefaultValue::Array { element, .. } => {
                self.replay_default_sources(source, element, (span, 0), found);
            }
            DefaultValue::Expression(recorded) => {
                if let Some(instance) = self.recorded_source(source, recorded) {
                    found.push(instance);
                }
            }
            DefaultValue::Declared { .. } => {
                if let Some(recorded) = source
                    .typed
                    .default_instances
                    .get(&(span.start, span.end, spot))
                {
                    if let Some(instance) =
                        self.recorded_source(source, recorded)
                    {
                        found.push(instance);
                    }
                }
            }
            DefaultValue::Prepared { .. } | DefaultValue::Type(_) => {}
        }
    }

    pub(crate) fn prepare<'a>(
        &self,
        (module, home): (&'a crate::ast::Module, Origin),
        resolved: crate::resolve::Resolved,
        typed: crate::typed::Typed,
        declarations: &'a crate::decl::Declarations,
        files: &'a [(String, crate::ast::Module)],
        cache: &crate::infer::files::FileCache,
    ) -> Option<Prepared<'a>> {
        let mut sources = sources(
            module,
            resolved,
            typed,
            declarations,
            files,
            cache,
            self.store.source_modules(),
        )?;
        sources[0].parameters_home = home.clone();
        sources[0].home = home;
        sources.retain(|source| {
            !self
                .emitted
                .contains(&(source.home.clone(), source.roots.clone()))
        });
        let definitions: Vec<_> = sources
            .iter()
            .map(|source| self.definition_inputs(source))
            .collect();
        let contributions = self.source_contributions(&sources, &definitions);
        let planned = sources
            .iter()
            .zip(&definitions)
            .flat_map(|(source, inputs)| {
                inputs
                    .iter()
                    .map(move |input| (source.home.clone(), input.site))
            })
            .collect();
        let available: alloc::collections::BTreeSet<_> = sources
            .iter()
            .zip(&contributions)
            .flat_map(|(source, contribution)| {
                contribution
                    .owned
                    .iter()
                    .map(move |site| (source.home.clone(), *site))
            })
            .collect();
        if !sources
            .iter()
            .zip(&contributions)
            .all(|(source, contribution)| {
                self.aliases_ready(source, contribution, &available, &[])
            })
        {
            return None;
        }
        let ready = Readiness {
            sources: &sources,
            contributions: &contributions,
            owned: &available,
            definitions: &planned,
        };
        if !sources.iter().zip(&definitions).zip(&contributions).all(
            |((source, own), contribution)| {
                self.source_ready(source, own, &[], contribution, &ready)
            },
        ) {
            return None;
        }
        Some(Prepared {
            sources,
            definitions,
            contributions,
        })
    }

    fn reserve_sources(
        &mut self,
        sources: &[Source<'_>],
        contributions: &[Contribution],
    ) -> Vec<Scope> {
        let mut scopes = Vec::new();
        let mut owned_slots = alloc::collections::BTreeMap::new();
        for (source, contribution) in sources.iter().zip(contributions) {
            let scope = self.reserve(source, contribution, &owned_slots);
            for site in &contribution.owned {
                if let Some(slot) = scope.slots.get(site) {
                    owned_slots.insert((source.home.clone(), *site), *slot);
                }
            }
            scopes.push(scope);
        }
        for ((source, scope), contribution) in
            sources.iter().zip(&scopes).zip(contributions)
        {
            for ((start, end), _) in &source.typed.frozen {
                let site = crate::token::Span {
                    start: *start,
                    end: *end,
                };
                if !contribution.owned.contains(&site) {
                    continue;
                }
                if let Some(slot) = scope.slots.get(&site) {
                    self.store.insert_slot((source.home.clone(), site), *slot);
                }
            }
        }
        for ((source, scope), contribution) in
            sources.iter().zip(&mut scopes).zip(contributions)
        {
            self.resolve_aliases(source, scope, contribution, &owned_slots);
        }
        scopes
    }

    pub(crate) fn emit_prepared(&mut self, prepared: Prepared<'_>) -> ir::Unit {
        let Prepared {
            sources,
            definitions,
            contributions,
        } = prepared;
        self.store
            .resources(sources[0].declarations.has_resources());
        self.source_module(&sources[0].module);
        let entry_start = self.store.tables().entries.len();
        let scopes = self.reserve_sources(&sources, &contributions);
        let bodies: Vec<_> = sources
            .iter()
            .zip(&scopes)
            .zip(definitions)
            .map(|((source, scope), inputs)| {
                self.emit_definitions(source, scope, inputs)
            })
            .collect();
        for source in &sources {
            for dispatch in &source.typed.dispatch {
                let key = (dispatch.name.clone(), dispatch.receiverless);
                if !self.dispatch.contains_key(&key) {
                    let index = self.store.push_dispatch(ir::Dispatch {
                        label: dispatch.name.clone(),
                        rows: Vec::new(),
                    });
                    self.dispatch.insert(key, index);
                }
            }
        }
        for (source, scope) in sources.iter().zip(&scopes) {
            self.tests(source, scope, &sources, &scopes);
        }
        for source in &sources {
            self.dispatch_rows(source);
        }
        for ((source, scope), bodies) in
            sources.iter().zip(&scopes).zip(&bodies)
        {
            self.fill(source, scope, bodies);
            self.local_functions(scope);
            self.captures(scope);
        }
        self.modules(&sources, &scopes);
        let main_kind =
            self.sequence(&sources[0], &scopes[0], &sources[0].roots);
        let main = match main_kind {
            ir::Kind::Block(lines) => lines,
            kind => {
                let ty = sources[0].typed.main.as_ref().expect("main type");
                alloc::vec![self.source_node(
                    &sources[0],
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
            free_parameters(&self.source_ty(&sources[0], ty), &mut parameters);
        }
        for source in &sources {
            self.emitted
                .push((source.home.clone(), source.roots.clone()));
        }
        self.remember(&sources[0], &scopes[0], &contributions[0]);
        let plan = self.store.tables().entries[entry_start..].to_vec();
        ir::Unit {
            tables: self.store.unit(),
            parameters,
            main,
            plan,
        }
    }

    /// Keep the module the unit's home emits from, for later lines'
    /// supplied expressions.
    fn source_module(&mut self, module: &crate::ast::Module) {
        let home = self.home.clone().unwrap_or(Origin::This);
        self.store
            .insert_source_module(home, Rc::new(module.clone()));
    }

    fn remember(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        contribution: &Contribution,
    ) {
        if self.home.is_some() {
            for ((start, end), _) in &source.typed.frozen {
                let site = crate::token::Span {
                    start: *start,
                    end: *end,
                };
                if !contribution.owned.contains(&site) {
                    continue;
                }
                if let Some(slot) = scope.slots.get(&site) {
                    self.store.insert_visible(site, *slot);
                }
            }
        }
    }

    fn modules(&mut self, sources: &[Source<'_>], scopes: &[Scope]) {
        for (source, scope) in sources.iter().zip(scopes) {
            if source.initializes_file {
                let Origin::File(label) = &source.home else {
                    unreachable!("file initializer has a File origin")
                };
                let kind = self.sequence(source, scope, &source.roots);
                let ir::Kind::Block(lines) = kind else {
                    unreachable!("module sequence")
                };
                self.store.push_module(ir::Module {
                    label: label.clone(),
                    lines,
                });
            }
        }
    }

    pub(super) fn cast_adapter(
        &mut self,
        source: &Source<'_>,
        index: usize,
        cast: &crate::typed::Cast,
        value: usize,
    ) -> (ir::Type, ir::Kind) {
        let from = self.source_ty(source, &cast.from);
        let to = self.source_ty(source, &cast.to);
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
        (to, kind)
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
            self.store.node_mut(*node).kind = kind;
            if let Some(cast) = source.typed.casts[index]
                .as_ref()
                .filter(|_| !scope.payload_casts.contains_key(&index))
            {
                let mut original = self.store.tables().nodes[*node].clone();
                original.ty = self.source_ty(source, &cast.from);
                let value = self.store.push_node(original);
                let (to, kind) = self.cast_adapter(source, index, cast, value);
                let mut node = self.store.node_mut(*node);
                node.ty = to;
                node.kind = kind;
            }
        }
        for body in bodies {
            if body.expression.is_none() {
                let node =
                    self.store.tables().definitions[body.definition].body;
                let kind = self.function(source, scope, &body.arms);
                self.store.node_mut(node).kind = kind;
            }
            let definition =
                self.store.tables().definitions[body.definition].clone();
            let mut body = definition.body;
            loop {
                let next = match &self.store.tables().nodes[body].kind {
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
            } = &mut self.store.node_mut(body).kind
            {
                *parameters = definition.parameters;
                *hidden = definition.hidden;
            }
        }
    }

    /// A local binding of a lambda whose scheme keeps a member limit
    /// (`body = n x => n + 1`): the slot carries hidden meaning
    /// parameters, so the lambda's function node carries them too and
    /// each use re-creates the closure with the use's witnesses.
    pub(super) fn local_functions(&mut self, scope: &Scope) {
        let tables = self.store.tables();
        let mut generic = Vec::new();
        for node in scope.nodes.iter().flatten().copied() {
            let ir::Kind::Bind { pattern, value } = &tables.nodes[node].kind
            else {
                continue;
            };
            let ir::PatternKind::Slot(slot) = pattern.kind else {
                continue;
            };
            let signature = &tables.slots[slot];
            if signature.definition.is_some() || signature.hidden.is_empty() {
                continue;
            }
            let mut body = *value;
            while let ir::Kind::Block(parts) = &tables.nodes[body].kind {
                let [part] = parts.as_slice() else { break };
                body = *part;
            }
            if matches!(tables.nodes[body].kind, ir::Kind::Function { .. }) {
                generic.push((
                    body,
                    signature.parameters.clone(),
                    signature.hidden.clone(),
                ));
            }
        }
        drop(tables);
        for (body, parameters, hidden) in generic {
            if let ir::Kind::Function {
                parameters: stored,
                hidden: stored_hidden,
                ..
            } = &mut self.store.node_mut(body).kind
            {
                *stored = parameters;
                *stored_hidden = hidden;
            }
        }
    }

    /// A nested function that uses a generic local lambda re-creates
    /// its closure, so it captures what the lambda captures.
    fn lambda_captures(
        scope: &Scope,
        tables: &ir::Tables,
        functions: &[usize],
        sites: &alloc::collections::BTreeMap<usize, crate::token::Span>,
        captures: &mut alloc::collections::BTreeMap<usize, Vec<usize>>,
    ) {
        use alloc::collections::BTreeMap;
        let lambdas: BTreeMap<usize, usize> = scope
            .nodes
            .iter()
            .flatten()
            .filter_map(|node| match &tables.nodes[*node].kind {
                ir::Kind::Bind { pattern, value } => match pattern.kind {
                    ir::PatternKind::Slot(slot)
                        if !tables.slots[slot].hidden.is_empty() =>
                    {
                        let mut body = *value;
                        while let ir::Kind::Block(parts) =
                            &tables.nodes[body].kind
                        {
                            let [part] = parts.as_slice() else { break };
                            body = *part;
                        }
                        Some((slot, body))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let mut settled = false;
        while !settled {
            settled = true;
            for function in functions {
                let Some(slots) = captures.get(function).cloned() else {
                    continue;
                };
                for slot in slots {
                    let Some(lambda) = lambdas.get(&slot) else {
                        continue;
                    };
                    let Some(inner) = captures.get(lambda).cloned() else {
                        continue;
                    };
                    let mine = captures.entry(*function).or_default();
                    for slot in inner {
                        let site = sites.get(&slot);
                        let span = tables.nodes[*function].span;
                        let outside = site.is_some_and(|site| {
                            !(span.start <= site.start && site.end <= span.end)
                        });
                        if outside && !mine.contains(&slot) {
                            mine.push(slot);
                            settled = false;
                        }
                    }
                }
            }
        }
    }

    pub(super) fn captures(&mut self, scope: &Scope) {
        let first_node = self.store.first_node();
        let tables = self.store.tables();
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
        let sites: alloc::collections::BTreeMap<_, _> = scope
            .slots
            .iter()
            .map(|(site, slot)| (*slot, *site))
            .collect();
        let mut events: Vec<_> =
            functions.iter().map(|index| (*index, true)).collect();
        // A function this line emitted has its locals among the line's
        // own nodes; only an older namespace's needs the whole table.
        let from = if functions.iter().all(|index| *index >= first_node) {
            first_node
        } else {
            0
        };
        events.extend(tables.nodes.iter().enumerate().skip(from).filter_map(
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
        let mut captures: alloc::collections::BTreeMap<usize, Vec<usize>> =
            alloc::collections::BTreeMap::new();
        let mut external = Vec::new();
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
            let mut outside = false;
            for outer in &enclosing {
                let span = tables.nodes[*outer].span;
                if !(span.start <= site.start && site.end <= span.end) {
                    let slots = captures.entry(*outer).or_default();
                    if !slots.contains(&slot) {
                        slots.push(slot);
                    }
                    outside = true;
                }
            }
            if outside {
                external.push((index, ir::Kind::Capture { slot, meanings }));
            }
        }
        Self::lambda_captures(
            scope,
            &tables,
            &functions,
            &sites,
            &mut captures,
        );
        drop(tables);
        for (index, kind) in external {
            self.store.node_mut(index).kind = kind;
        }
        for function in functions {
            if let ir::Kind::Function {
                captures: stored, ..
            } = &mut self.store.node_mut(function).kind
            {
                *stored = captures.remove(&function).unwrap_or_default();
            }
        }
    }

    pub(super) fn definition_inputs(
        &self,
        source: &Source<'_>,
    ) -> Vec<DefinitionInput> {
        use crate::ast::{Entry, ExprKind, ImplMember, ItemKind};
        let mut inputs = Vec::new();
        for entry in &source.module.entries {
            match entry {
                Entry::Line(id) => {
                    let ExprKind::Bind { pattern, value, .. } =
                        source.module.expressions[id.0].kind
                    else {
                        continue;
                    };
                    if source.typed.expr(value).is_none() {
                        continue;
                    }
                    let mut names = Vec::new();
                    pattern_names(&source.module, pattern, &mut names);
                    let fill = matches!(
                        source.module.patterns[pattern.0].kind,
                        crate::ast::PatternKind::Name(_)
                    );
                    for name in names {
                        if self.scope_has(source, name.span, &[]) {
                            inputs.push(DefinitionInput {
                                site: name.span,
                                body_span: name.span,
                                label: name.segments.join("."),
                                expression: Some(value),
                                arms: Vec::new(),
                                fill,
                            });
                        }
                    }
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
                            } if source.typed.expr(*value).is_some() => inputs
                                .push(DefinitionInput {
                                    site: name.span,
                                    body_span: name.span,
                                    label: name.segments.join("."),
                                    expression: Some(*value),
                                    arms: Vec::new(),
                                    fill: true,
                                }),
                            ImplMember::Constructor { arm, .. } => {
                                arms.push(arm.clone());
                            }
                            ImplMember::Named { .. } => (),
                        }
                    }
                    if let Some(first) = arms.first() {
                        if !self
                            .definitions
                            .contains_key(&(source.home.clone(), first.span))
                            && arms
                                .iter()
                                .all(|a| source.typed.expr(a.body).is_some())
                            && source.typed.schemes.contains_key(&(
                                first.span.start,
                                first.span.end,
                            ))
                        {
                            inputs.push(DefinitionInput {
                                site: first.span,
                                body_span: item.span,
                                label: String::from("Self"),
                                expression: None,
                                arms,
                                fill: true,
                            });
                        }
                    }
                }
            }
        }
        inputs
    }

    pub(super) fn emit_definitions(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        inputs: Vec<DefinitionInput>,
    ) -> Vec<Body> {
        let mut bodies = Vec::new();
        for input in inputs {
            // Plans precede emission of every source. An earlier source may
            // already have emitted this constructor, as in the original pass.
            if input.expression.is_none()
                && self
                    .definitions
                    .contains_key(&(source.home.clone(), input.site))
            {
                continue;
            }

            let body = input.expression.map_or_else(
                || {
                    let scheme = &source.typed.schemes
                        [&(input.site.start, input.site.end)];
                    self.source_node(
                        source,
                        input.body_span,
                        &scheme.ty,
                        ir::Kind::Block(Vec::new()),
                    )
                },
                |value| scope.expression(value),
            );
            let definition =
                self.definition(source, scope, input.site, input.label, body);
            if input.fill {
                bodies.push(Body {
                    definition,
                    expression: input.expression,
                    arms: input.arms,
                });
            }
        }
        bodies
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
        let signature = self.store.tables().slots[slot].clone();
        let namespace = self.namespace(&source.home);
        let index = self.store.push_definition(ir::Definition {
            namespace,
            label,
            span: site,
            parameters: signature.parameters,
            hidden: signature.hidden,
            ty: signature.ty,
            body,
            slot,
        });
        self.store.slot_mut(slot).definition = Some(index);
        self.store.insert_slot(key.clone(), slot);
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
                        self.store.tables().nodes[*node].ty.clone()
                    });
                    let body =
                        self.source_node(source, span, &Type::Unit, remainder);
                    self.store.node_mut(body).ty = ty;
                    let pattern = self.pattern(source, scope, pattern);
                    let kind = ir::Kind::Lift {
                        pattern,
                        value: scope.expression(value),
                        body,
                        rebuild: *rebuild,
                    };
                    let ty = &source.typed.rebuild_types[&site];
                    parts.push(self.source_node(source, span, ty, kind));
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
) -> Option<Vec<Source<'a>>> {
    let mut sources = alloc::vec![Source {
        initializes_file: false,
        declarations,
        files,
        module: Rc::new(module.clone()),
        resolved: Rc::new(resolved),
        typed: Rc::new(typed),
        home: Origin::This,
        parameters_home: Origin::This,
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
            initializes_file: true,
            declarations,
            files,
            module: Rc::new(module.clone()),
            resolved: checked.resolved.clone(),
            typed: checked.typed,
            home: Origin::File(stem.clone()),
            parameters_home: Origin::File(stem.clone()),
            roots: lines(module),
        });
    }
    let mut cursor = 0;
    while cursor < sources.len() {
        let constructors = sources[cursor].typed.constructors.clone();
        constructor_sources(
            &mut sources,
            declarations,
            files,
            constructors,
            modules,
        )?;
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
                source_owner(declarations, files, modules, &expression.origin)?;
            sources.push(Source {
                initializes_file: false,
                declarations,
                files,
                module,
                resolved: expression.resolved,
                typed: expression.typed,
                parameters_home: expression.origin.clone(),
                home: expression.origin,
                roots: alloc::vec![expression.value],
            });
        }
        cursor += 1;
    }
    Some(sources)
}

fn constructor_sources<'a>(
    sources: &mut Vec<Source<'a>>,
    declarations: &'a crate::decl::Declarations,
    files: &'a [(String, crate::ast::Module)],
    constructors: Vec<crate::typed::DeclaredConstructor>,
    modules: &alloc::collections::BTreeMap<Origin, Rc<crate::ast::Module>>,
) -> Option<()> {
    for constructor in constructors {
        if sources.iter().any(|source| {
            source.home == constructor.home && source.roots == constructor.roots
        }) {
            continue;
        }
        let module =
            source_owner(declarations, files, modules, &constructor.home)?;
        sources.push(Source {
            initializes_file: false,
            declarations,
            files,
            module,
            resolved: constructor.resolved,
            typed: constructor.typed,
            parameters_home: constructor.home.clone(),
            home: constructor.home,
            roots: constructor.roots,
        });
    }
    Some(())
}

fn source_owner(
    declarations: &crate::decl::Declarations,
    files: &[(String, crate::ast::Module)],
    modules: &alloc::collections::BTreeMap<Origin, Rc<crate::ast::Module>>,
    origin: &Origin,
) -> Option<Rc<crate::ast::Module>> {
    if let Some(module) = modules.get(origin) {
        return Some(module.clone());
    }
    if matches!(origin, Origin::Line(_)) {
        return None;
    }
    Some(Rc::new(owner(declarations, files, origin).clone()))
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

impl Index {
    fn scope_has(
        &self,
        source: &Source<'_>,
        site: crate::token::Span,
        earlier: &[crate::token::Span],
    ) -> bool {
        (self.home.as_ref() == Some(&source.home)
            && self.store.visible().contains_key(&site))
            || source.typed.schemes.contains_key(&(site.start, site.end))
            || earlier.contains(&site)
    }

    fn selected_ready(
        &self,
        source: &Source<'_>,
        selected: &crate::typed::Selected,
        planned: &alloc::collections::BTreeSet<(Origin, crate::token::Span)>,
        own: &[DefinitionInput],
    ) -> bool {
        let crate::typed::Selected::Value { origin, site } = selected else {
            return true;
        };
        let home = super::home_origin(&source.home, origin);
        self.definitions.contains_key(&(home.clone(), *site))
            || planned.contains(&(home.clone(), *site))
            || home == source.home
                && own.iter().any(|input| input.site == *site)
    }

    fn source_ready(
        &self,
        source: &Source<'_>,
        own: &[DefinitionInput],
        earlier: &[crate::token::Span],
        contribution: &Contribution,
        ready: &Readiness<'_, '_>,
    ) -> bool {
        if !self.aliases_ready(source, contribution, ready.owned, earlier) {
            return false;
        }
        let nodes = &contribution.nodes;
        let typed = &source.typed;
        if !self.selections_ready(source, ready.definitions, own, Some(nodes)) {
            return false;
        }
        for (value, place, earlier) in
            Self::executable_defaults(source, contribution)
        {
            if !self.default_ready(source, value, place, own, &earlier, ready) {
                return false;
            }
        }
        for test in &typed.tests {
            let Some(recorded) = &test.expression else {
                continue;
            };
            if self.emitted.iter().any(|(home, roots)| {
                *home == recorded.origin && roots.contains(&recorded.value)
            }) {
                continue;
            }
            let Some(index) = ready.sources.iter().position(|candidate| {
                candidate.home == recorded.origin
                    && candidate.roots.contains(&recorded.value)
            }) else {
                return false;
            };
            let assigned = &ready.contributions[index];
            // Every assigned source is checked by the outer prepare loop. Do
            // not recursively replay its complete frozen snapshot as a default.
            if !assigned.nodes.expressions.contains(&test.value.0)
                || !assigned.owned.contains(&test.self_site)
                || !self.aliases_ready(
                    &ready.sources[index],
                    assigned,
                    ready.owned,
                    &[],
                )
            {
                return false;
            }
        }
        true
    }

    fn selections_ready(
        &self,
        source: &Source<'_>,
        planned: &alloc::collections::BTreeSet<(Origin, crate::token::Span)>,
        own: &[DefinitionInput],
        nodes: Option<&crate::infer::SourceNodes>,
    ) -> bool {
        let typed = &source.typed;
        // name() directly lowers lexical locals; call_expression() normally
        // uses its callee node. Neither consumes the redundant selection here.
        let unused: alloc::collections::BTreeSet<_> = source
            .module
            .expressions
            .iter()
            .enumerate()
            .filter(|(index, expr)| match &expr.kind {
                crate::ast::ExprKind::Name(_)
                | crate::ast::ExprKind::Instance { .. } => matches!(
                    source.resolved.expr(crate::ast::ExprId(*index)),
                    Some(crate::resolve::Target::Local { .. })
                ),
                crate::ast::ExprKind::Call { .. } => !typed
                    .conversions
                    .contains_key(&(expr.span.start, expr.span.end)),
                _ => false,
            })
            .map(|(_, expr)| (expr.span.start, expr.span.end))
            .collect();
        let selections = typed
            .selected
            .iter()
            .filter(|(site, _)| {
                !unused.contains(site)
                    && nodes.is_none_or(|nodes| nodes.sites.contains(site))
            })
            .map(|(_, selected)| selected)
            .chain(
                typed
                    .roots
                    .iter()
                    .filter(|(site, _)| {
                        nodes.is_none_or(|nodes| nodes.sites.contains(site))
                    })
                    .map(|(_, root)| &root.selected),
            )
            .chain(
                typed
                    .accesses
                    .iter()
                    .filter(|(site, _)| {
                        nodes.is_none_or(|nodes| nodes.sites.contains(site))
                    })
                    .flat_map(|(_, accesses)| accesses)
                    .map(|access| &access.selected),
            )
            .chain(
                typed
                    .casts
                    .iter()
                    .enumerate()
                    .filter(|(id, _)| {
                        nodes.is_none_or(|nodes| nodes.expressions.contains(id))
                    })
                    .filter_map(|(_, cast)| cast.as_ref())
                    .filter_map(|cast| {
                        cast.operation.as_ref().map(|op| &op.selected)
                    }),
            )
            .chain(
                typed
                    .pattern_equals
                    .iter()
                    .filter(|(id, _)| {
                        nodes.is_none_or(|nodes| nodes.patterns.contains(id))
                    })
                    .map(|(_, op)| &op.selected),
            )
            .chain(
                typed
                    .dispatch
                    .iter()
                    .flat_map(|d| d.rows.iter().map(|r| &r.selected)),
            );
        if selections
            .into_iter()
            .any(|s| !self.selected_ready(source, s, planned, own))
        {
            return false;
        }
        true
    }

    fn recorded_source<'a>(
        &self,
        source: &Source<'a>,
        recorded: &crate::typed::DeclaredExpression,
    ) -> Option<Source<'a>> {
        let home = super::home_origin(&source.home, &recorded.origin);
        let module = if recorded.origin == Origin::This {
            source.module.clone()
        } else {
            source_owner(
                source.declarations,
                source.files,
                self.store.source_modules(),
                &home,
            )?
        };
        Some(Source {
            initializes_file: false,
            declarations: source.declarations,
            files: source.files,
            module,
            resolved: recorded.resolved.clone(),
            typed: recorded.typed.clone(),
            home,
            parameters_home: source.parameters_home.clone(),
            roots: alloc::vec![recorded.value],
        })
    }

    fn default_ready(
        &self,
        source: &Source<'_>,
        value: &crate::typed::DefaultValue,
        (span, spot): (crate::token::Span, usize),
        own: &[DefinitionInput],
        earlier: &[crate::token::Span],
        ready: &Readiness<'_, '_>,
    ) -> bool {
        use crate::typed::DefaultValue;
        match value {
            DefaultValue::Prepared { selected, .. } => {
                self.selected_ready(source, selected, ready.definitions, own)
            }
            DefaultValue::Record { fields, .. } => {
                fields.iter().enumerate().all(|(spot, (_, _, value))| {
                    let earlier: Vec<_> = fields
                        .iter()
                        .take(spot)
                        .map(|(site, _, _)| *site)
                        .collect();
                    self.default_ready(
                        source,
                        value,
                        (span, spot),
                        own,
                        &earlier,
                        ready,
                    )
                })
            }
            DefaultValue::Array { element, .. } => {
                self.default_ready(source, element, (span, 0), own, &[], ready)
            }
            DefaultValue::Expression(recorded) => {
                self.replay_ready(source, recorded, earlier, ready)
            }
            DefaultValue::Declared { .. } => source
                .typed
                .default_instances
                .get(&(span.start, span.end, spot))
                .is_some_and(|recorded| {
                    self.replay_ready(source, recorded, earlier, ready)
                }),
            DefaultValue::Type(_) => true,
        }
    }
}
