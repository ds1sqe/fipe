use super::{
    home_origin, ir, owner, Index, Origin, Scope, Source, String, Type,
    TypeName, Vec,
};

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
                let ty = self.ty(&source.home, ty);
                self.store.push_slot(ir::Slot {
                    ty,
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    definition: None,
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
                        let id = source
                            .module
                            .expressions
                            .iter()
                            .position(|expr| expr.span == *site)
                            .expect("pipe source");
                        scope.expression(crate::ast::ExprId(id))
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
                        self.node(
                            &source.home,
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
        self.applied(source, scope, span, call, &bound.applied)
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
    ) -> ir::Kind {
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
        let function = applied
            .iter()
            .rev()
            .fold(result, |result, (ty, _)| Type::function(ty.clone(), result));
        let callee = self.node(&source.home, span, &function, call);
        let slots = applied
            .iter()
            .map(|(ty, _)| {
                let ty = self.ty(&source.home, ty);
                self.store.push_slot(ir::Slot {
                    ty,
                    parameters: Vec::new(),
                    hidden: Vec::new(),
                    definition: None,
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
                        self.ty(&source.home, parameter),
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
        self.node(&source.home, span, ty, kind)
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
                        self.ty(&source.home, &arguments[position])
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
                    let ty = self.ty(&source.home, ty);
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
                ty: self.ty(&source.home, ty),
            },
            Selected::Constructor { ty, name } => {
                let mut result = ty;
                while let Type::Function { result: next, .. } = result {
                    result = next;
                }
                let ir::Type::Meaning { index: meaning, .. } =
                    self.ty(&source.home, result)
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
                receiver: ir::Receiver::Meaning(self.ty(&source.home, head)),
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
                let row = ir::DispatchRow {
                    receiver: self.ty(&source.home, &row.receiver),
                    receiver_spot: row.receiver_spot,
                    argument: row
                        .argument
                        .as_ref()
                        .map(|ty| self.ty(&source.home, ty)),
                    target,
                };
                if self.store.tables().dispatch[index].rows.contains(&row) {
                    continue;
                }
                self.store.push_row(index, row);
            }
        }
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
