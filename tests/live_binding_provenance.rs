//! Observe existing binding provenance, without installing a schema or view.
use fipe::{
    ast::{Entry, ExprId, ExprKind, PatternKind},
    compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    ir, mir,
    resolve::Target,
    session::{Report, Session},
    token::Span,
    value::{
        resource::{ResourceDomain, ResourceType},
        Value,
    },
    vm::Machine,
};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Route {
    site: Span,
    slot: usize,
    definition: usize,
    namespace: usize,
}
fn checked(session: &mut Session, text: &str) -> Report {
    let offset = session.source().len();
    let Outcome::Done(report) = session.line(text) else {
        panic!("this fixture has no file requests");
    };
    assert_eq!(report.offset, offset);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(report.accepted.is_some());
    report
}
fn binding(report: &Report, name: &str) -> (Span, ExprId) {
    let module = report.module.as_ref().unwrap();
    let found: Vec<_> = report
        .entries()
        .iter()
        .filter_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let ExprKind::Bind { pattern, value, .. } = module.expr(*id)?.kind
            else {
                return None;
            };
            let pattern = module.pattern(pattern)?;
            let PatternKind::Name(bound) = &pattern.kind else {
                return None;
            };
            (bound.segments.as_slice() == [name]).then_some((bound.span, value))
        })
        .collect();
    let [found] = found.as_slice() else {
        panic!("one actual named pattern");
    };
    *found
}
fn route(report: &Report, name: &str) -> Route {
    let (site, _) = binding(report, name);
    let unit = report.unit.as_ref().unwrap();
    let tables = unit.tables();
    let found: Vec<_> = unit
        .main
        .iter()
        .filter_map(|node| {
            let ir::Kind::Bind { pattern, .. } = &tables.nodes[*node].kind
            else {
                return None;
            };
            let ir::PatternKind::Slot(slot) = pattern.kind else {
                return None;
            };
            let (namespace, span) = tables.slots[slot].name?;
            if span != site {
                return None;
            }
            let definition = tables.slots[slot].definition.unwrap();
            let declared = &tables.definitions[definition];
            assert_eq!(declared.namespace, namespace);
            assert_eq!(declared.span, site);
            assert_eq!(declared.label, name);
            assert_eq!(declared.slot, slot);
            assert_eq!(declared.ty, tables.slots[slot].ty);
            assert!(
                declared.parameters.is_empty() && declared.hidden.is_empty()
            );
            Some(Route {
                site,
                slot,
                definition,
                namespace,
            })
        })
        .collect();
    let [found] = found.as_slice() else {
        panic!("one actual IR binding");
    };
    *found
}
fn assert_store(program: &mir::Program, route: Route) {
    let stores = program
        .bodies
        .iter()
        .flat_map(|body| &body.blocks)
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(statement, mir::Statement::Store { definition, .. }
                if *definition == route.definition)
        })
        .count();
    assert_eq!(stores, 1);
    let tables = program.tables.borrow();
    assert_eq!(tables.definitions[route.definition].span, route.site);
    assert_eq!(tables.slots[route.slot].definition, Some(route.definition));
}
fn stored(machine: &Machine, route: Route) -> &Value {
    machine
        .globals()
        .find_map(|(index, value)| (index == route.definition).then_some(value))
        .unwrap()
}
fn assert_alias(actual: &Value, issued: &Value) {
    let (Value::Resource(actual), Value::Resource(issued)) = (actual, issued)
    else {
        panic!("genuine resource aliases");
    };
    assert!(Rc::ptr_eq(actual, issued));
}
fn load(
    machine: &mut Machine,
    program: &mir::Program,
    domain: &ResourceDomain,
    key: &[u8],
    calls: &mut usize,
) -> Value {
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(program)
    else {
        panic!("one real load request");
    };
    let tables = machine.program().unwrap().tables.borrow();
    let declared = &tables.declarations[declaration];
    assert_eq!(tables.namespaces[declared.namespace], "json");
    assert_eq!(declared.label, "load");
    assert_eq!(arguments, [Value::Unit]);
    drop(tables);
    let ty = machine.pending_resource_type().unwrap();
    assert_eq!(
        ty,
        ResourceType::registered(
            &machine.program().unwrap().tables.borrow(),
            "json",
            "Json"
        )
        .unwrap()
    );
    *calls += 1;
    let value = domain.issue(&ty, Rc::from(key));
    assert!(matches!(
        machine.reply(program, Reply::Value(Ok(value.clone()))),
        Outcome::Done(Ok(Value::Unit))
    ));
    value
}
fn resolved_use(report: &Report, expression: ExprId, site: Span) {
    let resolved = report.resolved.as_ref().unwrap();
    let target = resolved.expr(expression).unwrap();
    assert_eq!(target, &Target::Local { site });
    assert_eq!(resolved.site(target), Some(site));
}
fn shadow(session: &mut Session, machine: &mut Machine) -> (Report, Route) {
    let report = checked(session, "j = 7\nj");
    let route = route(&report, "j");
    let Entry::Line(last) = report.entries().last().unwrap() else {
        panic!("j use");
    };
    resolved_use(&report, *last, route.site);
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert_store(&program, route);
    assert!(matches!(
        machine.run(&program),
        Outcome::Done(Ok(Value::Int(7)))
    ));
    assert_eq!(stored(machine, route), &Value::Int(7));
    (report, route)
}
fn withdraw(
    session: &mut Session,
    machine: &mut Machine,
    report: &Report,
    route: Route,
) {
    let undone = session.withdraw(report.accepted.unwrap()).unwrap();
    machine.withdraw(&undone);
    assert!(machine
        .globals()
        .all(|(index, _)| index != route.definition));
    assert!(machine
        .program()
        .unwrap()
        .tables
        .borrow()
        .definitions
        .get(route.definition)
        .is_none());
}
fn joined_read(
    session: &mut Session,
    machine: &mut Machine,
    routes: [Route; 3],
    values: [&Value; 3],
) {
    let report = checked(session, "(j,k,alias)");
    let module = report.module.as_ref().unwrap();
    let Entry::Line(last) = report.entries().last().unwrap() else {
        panic!("tuple use");
    };
    let ExprKind::Tuple(parts) = &module.expr(*last).unwrap().kind else {
        panic!("tuple");
    };
    assert_eq!(parts.len(), 3);
    for (part, route) in parts.iter().zip(routes) {
        resolved_use(&report, *part, route.site);
    }
    let program = compile::compile(report.unit.as_ref().unwrap());
    let tuple: Vec<_> = program
        .bodies
        .iter()
        .flat_map(|body| &body.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            if let mir::Statement::Assign(_, mir::Rvalue::Tuple(parts)) =
                statement
            {
                Some(parts)
            } else {
                None
            }
        })
        .collect();
    assert!(tuple.iter().any(|parts| parts.as_slice()
        == routes.map(|route| mir::Operand::Global(route.definition))));
    let Outcome::Done(Ok(Value::Tuple(actual))) = machine.run(&program) else {
        panic!("pure joined read");
    };
    assert_eq!(actual.len(), 3);
    for ((actual, expected), route) in actual.iter().zip(values).zip(routes) {
        assert_alias(actual, expected);
        assert_alias(stored(machine, route), expected);
    }
}

#[test]
fn original_load_bindings_keep_definition_identity_through_join_and_withdrawal()
{
    let mut declarations = Declarations::new();
    declarations
        .module("json", "pub type resource Json\npub load: () -> Json")
        .unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(None);
    let domain = ResourceDomain::new();
    let mut calls = 0;
    let first = checked(&mut session, "use json\nj = json.load ()");
    let j = route(&first, "j");
    let first_program = compile::compile(first.unit.as_ref().unwrap());
    assert_store(&first_program, j);
    let old = load(
        &mut machine,
        &first_program,
        &domain,
        b"entry-j",
        &mut calls,
    );
    assert_alias(stored(&machine, j), &old);
    let second = checked(&mut session, "k = json.load ()");
    let k = route(&second, "k");
    assert_ne!(j.site, k.site);
    assert_ne!(j.slot, k.slot);
    assert_ne!(j.definition, k.definition);
    let second_program = compile::compile(second.unit.as_ref().unwrap());
    assert!(Rc::ptr_eq(&first_program.tables, &second_program.tables));
    assert_store(&second_program, k);
    let other = load(
        &mut machine,
        &second_program,
        &domain,
        b"entry-k",
        &mut calls,
    );
    assert!(Rc::ptr_eq(
        &machine.program().unwrap().tables,
        &first_program.tables
    ));
    assert_store(machine.program().unwrap(), j);
    assert_store(machine.program().unwrap(), k);
    assert_alias(stored(&machine, j), &old);
    assert_alias(stored(&machine, k), &other);
    assert_ne!(old, other);
    let alias_report = checked(&mut session, "alias = j");
    let (_, initializer) = binding(&alias_report, "alias");
    resolved_use(&alias_report, initializer, j.site);
    let alias = route(&alias_report, "alias");
    assert_ne!(alias.definition, j.definition);
    let alias_program = compile::compile(alias_report.unit.as_ref().unwrap());
    assert_store(&alias_program, alias);
    assert!(matches!(
        machine.run(&alias_program),
        Outcome::Done(Ok(Value::Unit))
    ));
    assert_alias(stored(&machine, alias), &old);
    let (first_shadow, original_shadow) = shadow(&mut session, &mut machine);
    assert_ne!(original_shadow.site, j.site);
    assert_ne!(original_shadow.definition, j.definition);
    assert_alias(stored(&machine, j), &old);
    assert_alias(stored(&machine, k), &other);
    assert_alias(stored(&machine, alias), &old);
    withdraw(&mut session, &mut machine, &first_shadow, original_shadow);
    let (second_shadow, reused_shadow) = shadow(&mut session, &mut machine);
    assert_eq!(original_shadow, reused_shadow);
    assert_eq!(first_shadow.offset, second_shadow.offset);
    assert_ne!(first_shadow.accepted, second_shadow.accepted);
    assert!(session.withdraw(first_shadow.accepted.unwrap()).is_none());
    assert_eq!(stored(&machine, reused_shadow), &Value::Int(7));
    withdraw(&mut session, &mut machine, &second_shadow, reused_shadow);
    joined_read(
        &mut session,
        &mut machine,
        [j, k, alias],
        [&old, &other, &old],
    );
    assert_eq!(calls, 2);
}
