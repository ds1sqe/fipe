use super::{
    home_origin, ir, Index, Origin, Scope, Source, Type, TypeName, Vec,
};

impl Index {
    pub(super) fn predicate(
        &mut self,
        home: &Origin,
        ty: &Type,
        visiting: &mut Vec<TypeName>,
    ) -> ir::Predicate {
        match ty {
            Type::Function { parameter, result } => ir::Predicate::Function {
                parameter: alloc::boxed::Box::new(
                    self.predicate(home, parameter, visiting),
                ),
                result: alloc::boxed::Box::new(
                    self.predicate(home, result, visiting),
                ),
            },
            Type::Both(parts) => ir::Predicate::All(
                parts
                    .iter()
                    .map(|ty| self.predicate(home, ty, visiting))
                    .collect(),
            ),
            Type::Either(parts) => ir::Predicate::Any(
                parts
                    .iter()
                    .map(|ty| self.predicate(home, ty, visiting))
                    .collect(),
            ),
            Type::Not(inner) => ir::Predicate::Not(alloc::boxed::Box::new(
                self.predicate(home, inner, visiting),
            )),
            Type::Limited { name, .. } => self.predicate(home, name, visiting),
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
                    self.shapes.get(&name).cloned()
                {
                    let Type::Named {
                        arguments: parameters,
                        ..
                    } = &shape.ty
                    else {
                        unreachable!("layout name")
                    };
                    let replacements: Vec<_> = parameters
                        .iter()
                        .zip(arguments)
                        .filter_map(|(parameter, argument)| match parameter {
                            Type::Var(var) => Some((*var, argument.clone())),
                            _ => None,
                        })
                        .collect();
                    for base in &shape.bases {
                        predicates.push(self.predicate(
                            &shape_home,
                            &crate::types::replace(base, &replacements),
                            visiting,
                        ));
                    }
                }
                for index in self.gates.get(&name).cloned().unwrap_or_default()
                {
                    predicates.push(ir::Predicate::Test {
                        index,
                        meanings: self.parts(home, arguments),
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
                tests.iter().any(|index| {
                    self.tables.borrow().tests[*index].body == body
                })
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
            let index = self.tables.borrow().tests.len();
            self.tables.borrow_mut().tests.push(ir::Test {
                parameters,
                namespace,
                span: test_source.module.expressions[test.value.0].span,
                body,
                self_slot: test_scope.slots[&test.self_site],
            });
            self.gates.entry(name).or_default().push(index);
        }
    }

    pub(super) fn entries(
        &mut self,
        source: &Source<'_>,
        scope: &Scope,
        plan: &mut Vec<ir::Entry>,
    ) {
        for check in &source.typed.checks {
            if check.gate || check.place_origin != Origin::This {
                continue;
            }
            let namespace =
                self.namespace(&home_origin(&source.home, &check.place_origin));
            let predicate = check.target.as_ref().map_or(
                ir::Predicate::Nonzero {
                    when_integer: check.when_integer,
                },
                |ty| self.predicate(&source.home, ty, &mut Vec::new()),
            );
            let entry = self.tables.borrow().entries.len();
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
                    crate::typed::CheckReason::Narrowing => {
                        ir::Reason::Narrowing
                    }
                },
            };
            self.tables.borrow_mut().entries.push(recorded.clone());
            plan.push(recorded);
            for (id, node) in scope.nodes.iter().enumerate() {
                if check.value.map_or(
                    source.module.expressions[id].span != check.place,
                    |value| value.0 != id,
                ) {
                    continue;
                }
                let Some(node) = node else { continue };
                let saved = self.tables.borrow().nodes[*node].clone();
                let value = self.tables.borrow().nodes.len();
                self.tables.borrow_mut().nodes.push(saved);
                self.tables.borrow_mut().nodes[*node].kind =
                    ir::Kind::Enter { entry, value };
            }
        }
    }
}
