use super::{
    home_origin, ir, Index, Origin, Scope, Source, Type, TypeInput, TypeName,
    Vec,
};

impl Index {
    pub(super) fn source_predicate(
        &mut self,
        source: &Source<'_>,
        ty: &Type,
        visiting: &mut Vec<TypeName>,
    ) -> ir::Predicate {
        self.predicate_input(
            TypeInput {
                ty,
                home: &source.home,
                parameters_home: &source.parameters_home,
                bindings: &[],
            },
            visiting,
        )
    }

    fn predicate_input(
        &mut self,
        input: TypeInput<'_>,
        visiting: &mut Vec<TypeName>,
    ) -> ir::Predicate {
        if let Some(actual) = input.bound() {
            return self.predicate_input(actual, visiting);
        }
        let TypeInput { ty, home, .. } = input;
        match ty {
            Type::Function { parameter, result } => ir::Predicate::Function {
                parameter: alloc::boxed::Box::new(
                    self.predicate_input(input.child(parameter), visiting),
                ),
                result: alloc::boxed::Box::new(
                    self.predicate_input(input.child(result), visiting),
                ),
            },
            Type::Both(parts) => ir::Predicate::All(
                parts
                    .iter()
                    .map(|ty| self.predicate_input(input.child(ty), visiting))
                    .collect(),
            ),
            Type::Either(parts) => ir::Predicate::Any(
                parts
                    .iter()
                    .map(|ty| self.predicate_input(input.child(ty), visiting))
                    .collect(),
            ),
            Type::Not(inner) => ir::Predicate::Not(alloc::boxed::Box::new(
                self.predicate_input(input.child(inner), visiting),
            )),
            Type::Limited { name, .. } => {
                self.predicate_input(input.child(name), visiting)
            }
            Type::Named { name, arguments } => {
                let name = TypeName {
                    origin: home_origin(home, &name.origin),
                    name: name.name.clone(),
                };
                if visiting.contains(&name) {
                    return ir::Predicate::All(Vec::new());
                }
                visiting.push(name.clone());
                let mut predicates = Vec::new();
                if let Some((shape_home, shape)) =
                    self.store.shapes().get(&name).cloned()
                {
                    let Type::Named {
                        arguments: parameters,
                        ..
                    } = &shape.ty
                    else {
                        unreachable!("layout name")
                    };
                    let bindings: Vec<_> = parameters
                        .iter()
                        .zip(arguments)
                        .filter_map(|(parameter, argument)| match parameter {
                            Type::Var(var) => {
                                Some((*var, input.child(argument)))
                            }
                            _ => None,
                        })
                        .collect();
                    for base in &shape.bases {
                        predicates.push(self.predicate_input(
                            TypeInput {
                                ty: base,
                                home: &shape_home,
                                parameters_home: &shape_home,
                                bindings: &bindings,
                            },
                            visiting,
                        ));
                    }
                }
                for index in self.gates.get(&name).cloned().unwrap_or_default()
                {
                    predicates.push(ir::Predicate::Test {
                        index,
                        meanings: arguments
                            .iter()
                            .map(|ty| self.type_input(input.child(ty)))
                            .collect(),
                    });
                }
                visiting.pop();
                ir::Predicate::All(predicates)
            }
            _ => ir::Predicate::All(Vec::new()),
        }
    }

    pub(super) fn tests(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        sources: &[Source<'_>],
        scopes: &[Scope],
    ) {
        for test in &source.typed.tests {
            if test.expression.as_ref().is_some_and(|expression| {
                self.emitted.iter().any(|(origin, roots)| {
                    *origin == expression.origin
                        && roots.contains(&expression.value)
                })
            }) {
                continue;
            }
            let (test_source, test_scope) = test.expression.as_ref().map_or(
                (source, scope),
                |expression| {
                    let index = sources
                        .iter()
                        .position(|source| {
                            source.home == expression.origin
                                && source.roots.contains(&expression.value)
                        })
                        .expect("checked supplied test source");
                    (&sources[index], &scopes[index])
                },
            );
            let body = test_scope.expression(test.value);
            let Type::Named { name, arguments } = &test.owner else {
                unreachable!("test owner")
            };
            let name = TypeName {
                origin: home_origin(&source.home, &name.origin),
                name: name.name.clone(),
            };
            if self.gates.get(&name).is_some_and(|tests| {
                tests
                    .iter()
                    .any(|index| self.store.tables().tests[*index].body == body)
            }) {
                continue;
            }
            let parameters = arguments
                .iter()
                .map(|ty| {
                    let ir::Type::Parameter(index) =
                        self.ty(&test_source.home, ty)
                    else {
                        unreachable!("test parameter")
                    };
                    index
                })
                .collect();
            let namespace = self.namespace(&test_source.home);
            let index = self.store.push_test(ir::Test {
                parameters,
                namespace,
                span: test_source.module.expressions[test.value.0].span,
                body,
                self_slot: test_scope.slots[&test.self_site],
            });
            self.gates.entry(name).or_default().push(index);
        }
    }

    fn payload_entry(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        id: usize,
        check: &crate::typed::Check,
        entry: usize,
    ) -> bool {
        let Some((stage, expression)) = scope.payload_casts.get(&id) else {
            return false;
        };
        if check.receiving_call != Some(*stage) {
            return false;
        }
        let bound = &source.typed.bound[&(stage.start, stage.end)];
        let (spot, _) = bound
            .supplies
            .iter()
            .find(|(_, supply)| {
                matches!(
                    supply, crate::typed::Supply::Pipe(site)
                        if *site == source.module.expressions[id].span
                )
            })
            .expect("marked payload supply");
        let ty = self.source_ty(source, &bound.spots[*spot].1);
        let mut call = scope
            .receiving_calls
            .borrow()
            .get(stage)
            .copied()
            .unwrap_or_else(|| scope.expression(*expression));
        loop {
            let next = match &self.store.tables().nodes[call].kind {
                ir::Kind::Cast { value, .. }
                | ir::Kind::Pack { value, .. }
                | ir::Kind::Enter { value, .. } => Some(*value),
                _ => None,
            };
            let Some(next) = next else { break };
            call = next;
        }
        let value = {
            let tables = self.store.tables();
            let ir::Kind::Call { supplies, .. } = &tables.nodes[call].kind
            else {
                unreachable!("bound marked stage is a call")
            };
            supplies
                .iter()
                .find(|s| s.spot == *spot)
                .expect("payload spot")
                .value
        };
        let namespace = self.namespace(&source.home);
        let entered = self.store.push_node(ir::Node {
            namespace,
            span: check.place,
            ty,
            kind: ir::Kind::Enter { entry, value },
        });
        let mut node = self.store.node_mut(call);
        let ir::Kind::Call { supplies, .. } = &mut node.kind else {
            unreachable!("marked call")
        };
        supplies
            .iter_mut()
            .find(|s| s.spot == *spot)
            .expect("payload spot")
            .value = entered;
        true
    }

    pub(super) fn entries(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        plan: &mut Vec<ir::Entry>,
    ) {
        for check in &source.typed.checks {
            if check.role == crate::limits::CheckRole::CallerReport {
                continue;
            }
            let emitted = scope.nodes.iter().enumerate().any(|(id, node)| {
                if node.is_none() {
                    return false;
                }
                let place = source.module.expressions[id].span;
                check
                    .value
                    .map_or(place == check.place, |value| value.0 == id)
            });
            if check.gate || check.place_origin != Origin::This || !emitted {
                continue;
            }
            let namespace =
                self.namespace(&home_origin(&source.home, &check.place_origin));
            let predicate = check.target.as_ref().map_or(
                ir::Predicate::Nonzero {
                    when_integer: check.when_integer,
                },
                |ty| self.source_predicate(source, ty, &mut Vec::new()),
            );
            let entry = self.store.tables().entries.len();
            let recorded = ir::Entry {
                path: check.path.clone(),
                namespace,
                span: check.place,
                label: check.name.clone(),
                predicate,
                phase: match check.phase {
                    crate::typed::Phase::Compile => ir::Phase::Compile,
                    crate::typed::Phase::Init => ir::Phase::Init,
                    crate::typed::Phase::Eval => ir::Phase::Eval,
                },
                frequency: match check.frequency {
                    crate::typed::Frequency::Once => ir::Frequency::Once,
                    crate::typed::Frequency::PerCall => ir::Frequency::PerCall,
                    crate::typed::Frequency::PerElement => {
                        ir::Frequency::PerElement
                    }
                },
                reason: match check.reason {
                    crate::typed::CheckReason::UnknownInput => {
                        ir::Reason::UnknownInput
                    }
                    crate::typed::CheckReason::NoRule => ir::Reason::NoRule,
                    crate::typed::CheckReason::CompileTimeEvaluation => {
                        ir::Reason::CompileTimeEvaluation
                    }
                    crate::typed::CheckReason::CompileBudget => {
                        ir::Reason::CompileBudget
                    }
                },
            };
            self.store.push_entry(recorded.clone());
            plan.push(recorded);
            for (id, node) in scope.nodes.iter().enumerate() {
                if check.value.map_or(
                    source.module.expressions[id].span != check.place,
                    |value| value.0 != id,
                ) {
                    continue;
                }
                let Some(node) = node else { continue };
                if self.payload_entry(source, scope, id, check, entry) {
                    continue;
                }
                let saved = self.store.tables().nodes[*node].clone();
                let value = self.store.push_node(saved);
                self.store.node_mut(*node).kind =
                    ir::Kind::Enter { entry, value };
            }
        }
    }
}

#[cfg(test)]
mod predicate_input_tests {
    use super::*;

    #[test]
    fn bound_base_and_nested_arguments_keep_both_nominal_contexts() {
        for (declaration_home, caller_home) in [
            (
                Origin::Host(alloc::string::String::from("host")),
                Origin::This,
            ),
            (
                Origin::This,
                Origin::Host(alloc::string::String::from("host")),
            ),
        ] {
            let mut index = Index::default();
            let formal = crate::types::Var(0);
            let argument = Type::named(
                Origin::This,
                "Caller",
                alloc::vec![Type::Rigid(alloc::string::String::from("a"))],
            );
            let actual = TypeInput {
                ty: &argument,
                home: &caller_home,
                parameters_home: &caller_home,
                bindings: &[],
            };
            let bindings = [(formal, actual)];
            index.gates.insert(
                TypeName {
                    origin: caller_home.clone(),
                    name: alloc::string::String::from("Caller"),
                },
                alloc::vec![0],
            );
            index.gates.insert(
                TypeName {
                    origin: declaration_home.clone(),
                    name: alloc::string::String::from("Wrapper"),
                },
                alloc::vec![1],
            );
            let base = Type::Var(formal);
            let direct = index.predicate_input(
                TypeInput {
                    ty: &base,
                    home: &declaration_home,
                    parameters_home: &declaration_home,
                    bindings: &bindings,
                },
                &mut Vec::new(),
            );
            let expected_parameter = index.ty(
                &caller_home,
                &Type::Rigid(alloc::string::String::from("a")),
            );
            assert_eq!(
                direct,
                ir::Predicate::All(alloc::vec![ir::Predicate::Test {
                    index: 0,
                    meanings: alloc::vec![expected_parameter]
                }])
            );
            let nested =
                Type::named(Origin::This, "Wrapper", alloc::vec![base]);
            let predicate = index.predicate_input(
                TypeInput {
                    ty: &nested,
                    home: &declaration_home,
                    parameters_home: &declaration_home,
                    bindings: &bindings,
                },
                &mut Vec::new(),
            );
            let expected_argument = index.ty(&caller_home, &argument);
            assert_eq!(
                predicate,
                ir::Predicate::All(alloc::vec![ir::Predicate::Test {
                    index: 1,
                    meanings: alloc::vec![expected_argument]
                }])
            );
        }
    }
}
