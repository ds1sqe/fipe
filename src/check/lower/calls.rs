use super::{
    home_origin, ir, owner, Index, Origin, Scope, Source, String, Type,
    TypeName, Vec,
};
use alloc::string::ToString;

impl Index {
    pub(super) fn call(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        span: crate::token::Span,
        callee: usize,
        bound: &crate::typed::Bound,
        receiver: Option<usize>,
    ) -> ir::Kind {
        let callee = self.constructor_call(source, span, callee, bound);
        let slots: Vec<_> = bound
            .spots
            .iter()
            .map(|(_, ty)| {
                let ty = self.source_ty(source, ty);
                self.store.push_slot(ir::Slot {
                    ty,
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    definition: None,
                    name: None,
                })
            })
            .collect();
        let supplies = bound
            .supplies
            .iter()
            .map(|(spot, supply)| {
                let value = match supply {
                    crate::typed::Supply::Expression(id)
                    | crate::typed::Supply::Element(id) => {
                        scope.expression(*id)
                    }
                    crate::typed::Supply::Head(site) => receiver
                        .or_else(|| scope.receivers.borrow().get(site).copied())
                        .unwrap_or_else(|| {
                            panic!(
                                "receiver {site:?} at {span:?} in {:?}",
                                source.home
                            )
                        }),
                    crate::typed::Supply::Pipe(site) => {
                        self.pipe_supply(source, scope, span, *site)
                    }
                    crate::typed::Supply::Flag(site) => {
                        let meaning = self.meaning(
                            &source.home,
                            &TypeName {
                                origin: Origin::Prelude,
                                name: String::from("Bool"),
                            },
                        );
                        let constructor =
                            self.constructor_index(meaning, "True");
                        self.source_node(
                            source,
                            *site,
                            &Type::named(Origin::Prelude, "Bool", Vec::new()),
                            ir::Kind::Construct {
                                meaning,
                                constructor,
                                fields: Vec::new(),
                            },
                        )
                    }
                };
                ir::Supply {
                    spot: *spot,
                    value,
                    element: matches!(supply, crate::typed::Supply::Element(_)),
                }
            })
            .collect();
        let defaults = self.defaults(source, bound, &slots, span);
        let call = ir::Kind::Call {
            callee,
            slots,
            supplies,
            defaults,
            open: bound.open.clone(),
        };
        if bound.applied.is_empty() {
            return call;
        }
        let operator_result = source.module.expressions.iter().any(|expression| {
            matches!(expression.kind, crate::ast::ExprKind::Binary { operator, .. } | crate::ast::ExprKind::Pipe { operator, .. } | crate::ast::ExprKind::Not { operator, .. } if operator == span)
        }).then(|| {
            let mut ty = &source.typed.callee_types[&(span.start, span.end)];
            for _ in &bound.spots {
                let Type::Function { result, .. } = ty else { unreachable!("checked formal protocol") };
                ty = result;
            }
            ty.clone()
        });
        self.applied(
            source,
            scope,
            span,
            call,
            &bound.applied,
            operator_result.as_ref(),
        )
    }

    fn pipe_supply(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        span: crate::token::Span,
        site: crate::token::Span,
    ) -> usize {
        let id = source
            .module
            .expressions
            .iter()
            .position(|expr| expr.span == site)
            .expect("pipe source");
        let value = scope.expression(crate::ast::ExprId(id));
        if scope
            .payload_casts
            .get(&id)
            .is_some_and(|(stage, _)| *stage == span)
            && source.typed.casts[id].is_some()
        {
            let cast = source.typed.casts[id].as_ref().expect("payload cast");
            let (ty, kind) = self.cast_adapter(source, id, cast, value);
            let namespace = self.namespace(&source.home);
            self.store.push_node(ir::Node {
                namespace,
                span: site,
                ty,
                kind,
            })
        } else {
            value
        }
    }

    /// The operands past a spot list apply to the bound result: the bound
    /// call is a node of its own, a function, and the stage's node applies
    /// it to the operands as a call with one spot each.
    fn applied(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        span: crate::token::Span,
        call: ir::Kind,
        applied: &[(Type, crate::ast::ExprId)],
        operator_result: Option<&Type>,
    ) -> ir::Kind {
        let function = operator_result.cloned().unwrap_or_else(|| {
            let id = source
                .module
                .expressions
                .iter()
                .position(|expression| expression.span == span)
                .expect("stage expression");
            let result = source
                .typed
                .expr(crate::ast::ExprId(id))
                .expect("stage type")
                .clone();
            applied.iter().rev().fold(result, |result, (ty, _)| {
                Type::function(ty.clone(), result)
            })
        });
        let callee = self.source_node(source, span, &function, call);
        scope.receiving_calls.borrow_mut().insert(span, callee);
        let slots = applied
            .iter()
            .map(|(ty, _)| {
                let ty = self.source_ty(source, ty);
                self.store.push_slot(ir::Slot {
                    ty,
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    definition: None,
                    name: None,
                })
            })
            .collect();
        let supplies = applied
            .iter()
            .enumerate()
            .map(|(spot, (_, id))| ir::Supply {
                spot,
                value: scope.expression(*id),
                element: false,
            })
            .collect();
        ir::Kind::Call {
            callee,
            slots,
            supplies,
            defaults: Vec::new(),
            open: Vec::new(),
        }
    }

    fn constructor_call(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        callee: usize,
        bound: &crate::typed::Bound,
    ) -> usize {
        let Some(fields) =
            source.typed.constructor_fields.get(&(span.start, span.end))
        else {
            return callee;
        };
        let original = self.store.tables().nodes[callee].clone();
        let ir::Kind::Constructor {
            meaning,
            constructor,
            ..
        } = original.kind
        else {
            return callee;
        };
        let mut result = original.ty;
        while let ir::Type::Function { result: next, .. } = result {
            result = *next;
        }
        let signature =
            bound
                .spots
                .iter()
                .rev()
                .fold(result, |result, (_, parameter)| ir::Type::Function {
                    parameter: alloc::boxed::Box::new(
                        self.source_ty(source, parameter),
                    ),
                    result: alloc::boxed::Box::new(result),
                });
        // The callee node is this call's own: it takes the fields in
        // place rather than leaving an unused copy beside it.
        *self.store.node_mut(callee) = ir::Node {
            ty: signature,
            kind: ir::Kind::Constructor {
                meaning,
                constructor,
                fields: fields.clone(),
            },
            ..original
        };
        callee
    }

    pub(super) fn selected(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        selected: &crate::typed::Selected,
        arguments: &[Type],
        ty: &Type,
        receiver: Option<usize>,
    ) -> usize {
        let kind =
            self.selected_kind(source, selected, arguments, ty, receiver);
        self.source_node(source, span, ty, kind)
    }

    /// The operation a selection lowers to, for the node that holds it.
    pub(super) fn selected_kind(
        &mut self,
        source: &Source<'_>,
        selected: &crate::typed::Selected,
        arguments: &[Type],
        ty: &Type,
        receiver: Option<usize>,
    ) -> ir::Kind {
        use crate::typed::Selected;
        match selected {
            Selected::Value { origin, site } => {
                let origin = home_origin(&source.home, origin);
                let index = self.definitions[&(origin, *site)];
                let definition = self.store.tables().definitions[index].clone();
                let meanings = definition
                    .hidden
                    .iter()
                    .map(|hidden| {
                        let position = definition
                            .parameters
                            .iter()
                            .position(|parameter| parameter == hidden)
                            .expect("hidden parameter belongs to definition");
                        self.source_ty(source, &arguments[position])
                    })
                    .collect();
                ir::Kind::Definition { index, meanings }
            }
            Selected::Declared { origin, site } => {
                let origin = home_origin(&source.home, origin);
                let key = (origin.clone(), *site);
                let index = if let Some(index) = self.declarations.get(&key) {
                    *index
                } else {
                    let (ordinal, label) = declaration(source, &origin, *site);
                    let namespace = self.namespace(&origin);
                    let ty = self.source_ty(source, ty);
                    let index = self.store.push_declaration(ir::Declaration {
                        namespace,
                        index: ordinal,
                        label,
                        ty,
                    });
                    self.declarations.insert(key, index);
                    index
                };
                ir::Kind::Declared(index)
            }
            Selected::Derived { ty, kind } => ir::Kind::Derived {
                kind: *kind,
                ty: self.source_ty(source, ty),
            },
            Selected::Constructor { ty, name } => {
                let mut result = ty;
                while let Type::Function { result: next, .. } = result {
                    result = next;
                }
                let ir::Type::Meaning { index: meaning, .. } =
                    self.source_ty(source, result)
                else {
                    unreachable!("constructor result is nominal")
                };
                let constructor = self.constructor_index(meaning, name);
                let count = self.store.tables().meanings[meaning].constructors
                    [constructor]
                    .fields
                    .len();
                ir::Kind::Constructor {
                    meaning,
                    constructor,
                    fields: (0..count).collect(),
                }
            }
            Selected::Field { index, .. } => ir::Kind::Field {
                value: receiver.expect("field has a receiver"),
                index: *index,
            },
            Selected::Member { name, .. } => ir::Kind::Dispatch {
                receiver: ir::Receiver::Value(
                    receiver.expect("member has a receiver"),
                ),
                member: self.dispatch[&(name.clone(), false)],
            },
            Selected::Static { head, name } => ir::Kind::Dispatch {
                receiver: ir::Receiver::Meaning(self.source_ty(source, head)),
                member: self.dispatch[&(name.clone(), true)],
            },
        }
    }

    pub(super) fn dispatch_rows(&mut self, source: &Source<'_>) {
        for dispatch in &source.typed.dispatch {
            let index =
                self.dispatch[&(dispatch.name.clone(), dispatch.receiverless)];
            for row in &dispatch.rows {
                let target =
                    if let crate::typed::Selected::Field { index, .. } =
                        row.selected
                    {
                        ir::Implementation::Field(index)
                    } else {
                        let node = self.selected(
                            source,
                            source.module.span,
                            &row.selected,
                            &row.meanings,
                            &row.ty,
                            None,
                        );
                        match self.store.tables().nodes[node].kind.clone() {
                            ir::Kind::Definition { index, meanings } => {
                                ir::Implementation::Definition {
                                    index,
                                    meanings,
                                }
                            }
                            ir::Kind::Declared(index) => {
                                ir::Implementation::Declared(index)
                            }
                            ir::Kind::Derived { kind, ty } => {
                                ir::Implementation::Derived { kind, ty }
                            }
                            _ => unreachable!(
                                "dispatch rows have direct targets"
                            ),
                        }
                    };
                let declared_arity =
                    if matches!(target, ir::Implementation::Declared(_)) {
                        let crate::typed::Selected::Declared { origin, site } =
                            &row.selected
                        else {
                            unreachable!("declared row")
                        };
                        Some(declaration_arity(source, origin, *site))
                    } else {
                        None
                    };
                let input_contract = declared_arity.and_then(|arity| {
                    self.declared_row_input_contract(source, row, arity)
                });
                let row = ir::DispatchRow {
                    input_contract,
                    ty: self.source_ty(source, &row.ty),
                    declared_arity,
                    receiver: self.source_ty(source, &row.receiver),
                    receiver_spot: row.receiver_spot,
                    argument: row
                        .argument
                        .as_ref()
                        .map(|ty| self.source_ty(source, ty)),
                    target,
                };
                if self.store.tables().dispatch[index].rows.contains(&row) {
                    continue;
                }
                self.store.push_row(index, row);
            }
        }
    }
    fn declared_row_input_contract(
        &mut self,
        source: &Source<'_>,
        row: &crate::typed::DispatchRow,
        arity: usize,
    ) -> Option<usize> {
        let crate::typed::Selected::Declared { origin, site } = &row.selected
        else {
            return None;
        };
        let receiver = row.receiver_spot?;
        let mut ty = &row.ty;
        let mut parameters = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..arity {
            let Type::Function { parameter, result } = ty else {
                unreachable!("checked declared raw parameters")
            };
            let predicate =
                self.source_predicate(source, parameter, &mut Vec::new());
            let mut pending = alloc::vec![(
                &predicate,
                matches!(&predicate, ir::Predicate::Function { .. })
            )];
            let mut active = false;
            while let Some((part, entry)) = pending.pop() {
                if entry {
                    if let ir::Predicate::Function { parameter, result } = part
                    {
                        pending.push((result, true));
                        pending.push((
                            parameter,
                            matches!(
                                parameter.as_ref(),
                                ir::Predicate::Function { .. }
                            ),
                        ));
                    }
                    continue;
                }
                match part {
                    ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                        pending.extend(parts.iter().map(|part| (part, false)));
                    }
                    ir::Predicate::Not(inner) => pending.push((inner, false)),
                    ir::Predicate::Test { .. }
                    | ir::Predicate::Nonzero { .. } => active = true,
                    ir::Predicate::Function { .. } => (),
                }
            }
            if active {
                labels.push(match parameter.as_ref() {
                    Type::Limited { name, .. } => name.to_string(),
                    other => other.to_string(),
                });
            }
            parameters.push(predicate);
            ty = result;
        }
        if labels.is_empty() {
            return None;
        }
        let receiver_predicate = parameters.remove(receiver);
        parameters.insert(0, receiver_predicate);
        let mut predicate = ir::Predicate::All(Vec::new());
        for parameter in parameters.into_iter().rev() {
            predicate = ir::Predicate::Function {
                parameter: alloc::boxed::Box::new(parameter),
                result: alloc::boxed::Box::new(predicate),
            };
        }
        let entry = ir::Entry {
            path: Vec::new(),
            span: *site,
            namespace: self.namespace(&home_origin(&source.home, origin)),
            label: if labels.len() == 1 {
                labels.remove(0)
            } else {
                row.ty.to_string()
            },
            predicate,
            phase: ir::Phase::Eval,
            frequency: ir::Frequency::PerCall,
            reason: ir::Reason::UnknownInput,
        };
        // Equivalent source rows must share the same entry identity before row dedup.
        let existing = self
            .store
            .tables()
            .entries
            .iter()
            .position(|prior| prior == &entry);
        Some(existing.unwrap_or_else(|| self.store.push_entry(entry)))
    }

    pub(super) fn constant_default(&mut self, node: usize) -> usize {
        let tables = self.store.tables();
        let value = &tables.nodes[node];
        if matches!(value.ty, ir::Type::Function { .. }) {
            return node;
        }
        let same = |index: &usize| {
            let previous = &tables.nodes[*index];
            if previous.ty != value.ty {
                return false;
            }
            match (&previous.kind, &value.kind) {
                (
                    ir::Kind::Literal {
                        form: a,
                        reader: ar,
                    },
                    ir::Kind::Literal {
                        form: b,
                        reader: br,
                    },
                ) => {
                    a == b
                        && match (ar, br) {
                            (None, None) => true,
                            (Some(a), Some(b)) => {
                                tables.nodes[*a].kind == tables.nodes[*b].kind
                                    && tables.nodes[*a].ty
                                        == tables.nodes[*b].ty
                            }
                            _ => false,
                        }
                }
                (ir::Kind::Definition { .. }, ir::Kind::Definition { .. })
                | (ir::Kind::Declared(_), ir::Kind::Declared(_)) => {
                    previous.kind == value.kind
                }
                _ => false,
            }
        };
        if let Some(existing) =
            self.constant_defaults.iter().find(|index| same(index))
        {
            return *existing;
        }
        if matches!(
            value.kind,
            ir::Kind::Literal { .. }
                | ir::Kind::Definition { .. }
                | ir::Kind::Declared(_)
        ) {
            self.constant_defaults.push(node);
        }
        node
    }
}

// Primitive ordinals follow source order, independent of use order.
fn declaration(
    source: &Source<'_>,
    origin: &Origin,
    site: crate::token::Span,
) -> (usize, String) {
    use crate::ast::{Entry, ImplMember, ItemKind};
    let module = owner(source.declarations, source.files, origin);
    let mut names = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Declare { name, .. } => names.push(name),
            ItemKind::Impl { members, .. } => {
                for member in members {
                    if let ImplMember::Named {
                        name, value: None, ..
                    } = member
                    {
                        names.push(name);
                    }
                }
            }
            _ => (),
        }
    }
    let (index, name) = names
        .iter()
        .enumerate()
        .find(|(_, name)| name.span == site)
        .expect("primitive declaration");
    (index, name.segments.join("."))
}

/// Count the original annotation's arrows before substituting its result.
fn declaration_arity(
    source: &Source<'_>,
    origin: &Origin,
    site: crate::token::Span,
) -> usize {
    use crate::ast::{Entry, ImplMember, ItemKind, TypeKind};
    let module = owner(source.declarations, source.files, origin);
    let mut found = None;
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Declare { name, ty } if name.span == site => {
                found = Some(*ty);
            }
            ItemKind::Impl { members, .. } => {
                for member in members {
                    if let ImplMember::Named {
                        name,
                        annotation: Some(ty),
                        value: None,
                        ..
                    } = member
                    {
                        if name.span == site {
                            found = Some(*ty);
                        }
                    }
                }
            }
            _ => (),
        }
    }
    let mut ty = found.expect("original declaration annotation");
    if let Some(TypeKind::Qualified { ty: inner, .. }) =
        module.ty(ty).map(|ty| &ty.kind)
    {
        ty = *inner;
    }
    let mut arity = 0;
    while let Some(TypeKind::Function { result, .. }) =
        module.ty(ty).map(|ty| &ty.kind)
    {
        arity += 1;
        ty = *result;
    }
    arity
}

#[cfg(test)]
mod declared_row_binding_probe {
    extern crate std;
    use super::*;

    const MAIN: &str = "use host\nuse methods\nget = m => m.op\nf = get (methods.M 7)\nhost.after ()\nf (host.arg ())\n";
    const RECOVERABLE: &str = "pub type Tagged<T>: Integer where self < 10\npub type M<T> { pub token: T }\nimpl M { op: Tagged<T> -> (self: Self) -> Integer }\n";
    const UNBOUND: &str = "pub type Tagged<T>: Integer where self < 10\npub type M { pub token: Integer }\nimpl M { op: Tagged<a> -> (self: Self) -> Integer }\n";

    fn observe(header: &str, required: bool) {
        let mut declarations = crate::decl::Declarations::new();
        if let Err(errors) = declarations.module("methods", header) {
            std::println!(
                "BINDER_HEADER required={required} errors={errors:?}"
            );
            assert!(!required, "recoverable source must admit");
            return;
        }
        declarations
            .module(
                "host",
                "pub after: () -> Integer\npub arg: () -> Integer\n",
            )
            .unwrap();
        let module = crate::parse::parse(crate::lex::lex(MAIN)).unwrap();
        let cache = crate::infer::files::FileCache::default();
        let checked = crate::check::stages(
            &module,
            &declarations,
            &[],
            None,
            &cache,
            crate::check::Options::default(),
            (usize::MAX, 0),
        );
        std::println!(
            "BINDER_CHECK required={required} diagnostics={:?}",
            checked.diagnostics
        );
        if !checked.diagnostics.is_empty() {
            assert!(!required, "recoverable source must check");
            return;
        }
        let typed = checked.typed.unwrap();
        let resolved = checked.resolved.unwrap();
        let mut index = super::super::Index::default();
        let Some(prepared) = index.prepare(
            (&module, Origin::This),
            resolved,
            typed,
            &declarations,
            &[],
            &cache,
        ) else {
            std::println!("BINDER_PREPARE required={required} ready=false");
            assert!(!required, "recoverable source dependencies must be ready");
            return;
        };
        let source = prepared.sources[0].clone();
        let row = source
            .typed
            .dispatch
            .iter()
            .find(|dispatch| dispatch.name == "op")
            .and_then(|dispatch| dispatch.rows.first())
            .expect("actual op row")
            .clone();
        std::println!("BINDER_TYPED required={required} home={:?} parameters_home={:?} receiver={:?} ty={:?} meanings={:?}",
            source.home, source.parameters_home, row.receiver, row.ty, row.meanings);
        let _unit = index.emit_prepared(prepared);
        let predicate =
            index.source_predicate(&source, &row.ty, &mut Vec::new());
        let receiver = index.source_ty(&source, &row.receiver);
        let call = MAIN.find("get (methods.M 7)").unwrap();
        let actual = source
            .typed
            .bound
            .get(&(call, call + "get (methods.M 7)".len()))
            .expect("actual receiving get call")
            .spots[0]
            .1
            .clone();
        let actual = index.source_ty(&source, &actual);
        let tables = index.store.tables();
        let member = tables
            .dispatch
            .iter()
            .position(|dispatch| dispatch.label == "op")
            .unwrap();
        let selected =
            crate::compile::dispatch::select(&tables, member, &actual, None)
                .expect("actual concrete receiver selects op");
        std::println!("BINDER_IR required={required} receiver={receiver:?} actual={actual:?} predicate={predicate:?} selected={selected:?}");
        if !required {
            return;
        }
        assert_predicate_uses_selected_receiver_binder(
            &tables, &receiver, &selected, &predicate, header,
        );
    }

    fn assert_predicate_uses_selected_receiver_binder(
        tables: &ir::Tables,
        receiver: &ir::Type,
        selected: &crate::compile::dispatch::Selected,
        predicate: &ir::Predicate,
        header: &str,
    ) {
        let ir::Type::Meaning { arguments, .. } = receiver else {
            panic!("generic owner receiver")
        };
        let [ir::Type::Parameter(parameter)] = arguments.as_slice() else {
            panic!("one real owner binder")
        };
        let actual_binding = selected
            .bindings
            .iter()
            .find(|(id, _)| id == parameter)
            .expect("predicate owner binder has selected actual");
        let ir::Type::Meaning {
            index: integer,
            arguments,
        } = &actual_binding.1
        else {
            panic!("Integer actual")
        };
        assert!(arguments.is_empty());
        assert_eq!(tables.meanings[*integer].label, "Integer");
        assert_eq!(
            tables.namespaces[tables.meanings[*integer].namespace],
            "prelude"
        );
        let mut pending = alloc::vec![predicate];
        let mut matching = 0;
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Function { parameter, result } => {
                    pending.extend([parameter.as_ref(), result.as_ref()]);
                }
                ir::Predicate::Test { index, meanings } => {
                    let test = &tables.tests[*index];
                    if tables.namespaces[test.namespace] == "methods"
                        && header.get(test.span.start..test.span.end)
                            == Some("self < 10")
                    {
                        assert_eq!(
                            meanings,
                            &[ir::Type::Parameter(*parameter)]
                        );
                        assert_eq!(
                            crate::compile::dispatch::substitute(
                                &meanings[0],
                                &selected.bindings
                            ),
                            actual_binding.1
                        );
                        matching += 1;
                    }
                }
                ir::Predicate::Nonzero { .. } => {}
            }
        }
        assert!(
            matching > 0,
            "actual source predicate Test meaning uses receiver-owned binder"
        );
    }

    #[test]
    fn declared_row_predicate_binder_metadata_probe() {
        observe(UNBOUND, false);
        observe(RECOVERABLE, true);
    }
}
