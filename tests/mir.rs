use fipe::{
    check, compile, decl::Declarations, ir, lex::lex, mir, parse::parse,
    value::Value,
};

/// Sources every row and the walk share: the straight line.
const STRAIGHT_LINES: &[&str] = &[
    "16",
    "[1, 2]",
    "(1, \"a\")",
    "if 1 == 1 { 2 } else { 3 }",
    "with cwd /tmp { 1 }",
    "{\n    x = 1\n    x\n}",
    "Compile 1",
    "f = (w = 80) (c = (w / 8)) => c\nf 16",
    "add2 = x => x + 2\nadd2 4",
    "pair = a b => (a, b)\npair --b 2",
    "g: Vec<Integer> -> Integer = xs => len xs\ng 1 2 3",
    "id = len\nid [1]",
    "Just",
    "outer = x => (y => (x, y))\nouter 1",
    "p = Point 1 2\np.x\ntype Point { x: Integer, y: Integer }",
    "2.5",
    "x: F32 = 1.5\nx",
    "n = 9223372036854775808\nn",
    "\"text\"",
    "(x => x + 2) 2",
    "k = 1\n(k, k)",
    "Just 1 |: (_ => Just 2)",
    "Just [1, 2] |: map (n => n + 1)",
    "Ok 1 |! (e => 3)",
    "Nothing |! 1",
    "Nothing |! Just 2",
    "Just 1 || Nothing",
    "Just 1 || 2",
    "Just 1 && Just 2",
    "Just 1 && 2",
    "(1 == 1) || (2 == 2)",
    "(1 == 1) && (2 == 2)",
    "{\n    [:v] = Just 1\n    v\n}",
    "{\n    [:v] = Ok 1\n    Just v\n}",
    "{\n    Just x = Just 1\n    x + 1\n}",
    "Just x = Just 1",
    "1 | to<U8>",
    "1 | try<U8>",
    "\"3\" | read<I64>",
    "1 < 2",
    "Nothing",
    "g: Integer -> F64 = n => n\ng 1",
    "f = n => {\n    x: U8 = n\n    x\n}\nf 1",
    "p: Integer | String = 1",
    "type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | to<Box>",
    "type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | try<Box>",
    "type Box: TryFrom<Integer, String> { value: Integer }\nimpl Box: TryFrom<Integer, String> { try_from = n => Ok (Self n) }\n1 | to<Box>",
];

fn program(source: &str) -> (ir::Unit, mir::Program) {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let program = compile::compile(&unit);
    (unit, program)
}

#[test]
fn a_value_is_sixteen_bytes() {
    assert_eq!(core::mem::size_of::<Value>(), 16);
}

#[test]
fn an_operand_is_sixteen_bytes() {
    assert_eq!(core::mem::size_of::<mir::Operand>(), 16);
}

#[path = "mir/calls.rs"]
mod calls;
#[path = "mir/examples.rs"]
mod examples;
#[path = "mir/names.rs"]
mod names;
#[path = "mir/outcomes.rs"]
mod outcomes;
#[path = "mir/patterns.rs"]
mod patterns;
#[path = "mir/walk.rs"]
mod walk;

#[test]
fn marked_payload_conversion_preserves_outcome_switch() {
    for source in [
        "take: Vec<U8> -> Integer = xs => len xs\nr: Result<String, String> = Ok \"abc\"\nr |: take",
        "take: Vec<U8> -> Integer = xs => len xs\nr: Result<String, String> = Err \"abc\"\nr |! take",
        "take: Vec<U8> -> Integer = xs => len xs\nr: Result<String, String> = Ok \"abc\"\nr |: (x => take x)",
    ] {
        let (unit, _) = program(source);
        let tables = unit.tables();
        let left = tables.nodes.iter().find_map(|node| match &node.kind {
            ir::Kind::Marked { left, .. } => Some(*left),
            _ => None,
        }).expect("marked pipe");
        assert!(!matches!(tables.nodes[left].kind, ir::Kind::Cast { .. }));
    }
}

#[test]
fn pack_hidden_tracks_source_parameters_without_forcing_unused_phantoms() {
    let source = "type Phantom<T>: Eq {}\npack: Phantom<a> -> Eq = p => p\nignore: Phantom<a> -> Integer = _ => 1\np: Phantom<Integer> = Phantom {}\n(pack p, ignore p)";
    let module = parse(lex(source)).unwrap();
    let declarations = Declarations::new();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    let pack = source.find("pack:").unwrap();
    let ignore = source.find("ignore:").unwrap();
    let pack = (pack, pack + 4);
    let ignore = (ignore, ignore + 6);
    assert_eq!(typed.schemes[&pack].variables.len(), 1);
    assert_eq!(typed.body_parameters[&pack].len(), 1);
    assert!(typed
        .casts
        .iter()
        .flatten()
        .any(|cast| cast.row == fipe::typed::CastRow::Pack));
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let mut tags = Vec::new();
    for body in &program.bodies {
        for statement in body.blocks.iter().flat_map(|block| &block.statements)
        {
            if let mir::Statement::Assign(
                _,
                mir::Rvalue::Pack {
                    tag: ir::Type::Meaning { arguments, .. },
                    ..
                },
            ) = statement
            {
                let [ir::Type::Parameter(parameter)] = arguments.as_slice()
                else {
                    panic!("generic Phantom tag: {arguments:?}")
                };
                tags.push((*parameter, body.hidden.clone()));
            }
        }
    }
    assert_eq!(tags.len(), 1);
    assert_eq!(
        typed.hidden[&pack],
        [0],
        "actual Pack source reads its phantom parameter"
    );
    assert!(typed.hidden[&ignore].is_empty());
    assert_eq!(tags[0].1, [tags[0].0]);
}

#[test]
fn overloaded_conversion_call_selects_each_source_at_its_call() {
    let source = "type P: From<Integer> & From<String> { n: Integer }\nimpl P: From<Integer> { from: Integer -> Self = n => Self n }\nimpl P: From<String> { from: String -> Self = s => Self 0 }\n(P.from 1, P.from \"x\")";
    let (unit, _) = program(source);
    let tables = unit.tables();
    let mut bodies = Vec::new();
    for (text, expected) in
        [("P.from 1", "Integer"), ("P.from \"x\"", "String")]
    {
        let start = source.rfind(text).unwrap();
        let call = tables
            .nodes
            .iter()
            .find(|node| {
                node.span.start == start
                    && node.span.end == start + text.len()
                    && matches!(node.kind, ir::Kind::Call { .. })
            })
            .expect("the consuming call");
        let ir::Kind::Call {
            callee, supplies, ..
        } = &call.kind
        else {
            unreachable!()
        };
        assert_eq!(supplies.len(), 1);
        let ir::Kind::Convert {
            callee: Some(operation),
            from,
            ..
        } = &tables.nodes[*callee].kind
        else {
            panic!("the selected conversion")
        };
        let ir::Type::Meaning { index, .. } = from else {
            panic!("nominal source")
        };
        assert_eq!(tables.meanings[*index].label, expected);
        let ir::Kind::Definition { index, .. } = tables.nodes[*operation].kind
        else {
            panic!("written conversion body")
        };
        bodies.push(index);
        assert!(!tables.nodes.iter().any(|node| node.span.start == start
            && node.span.end == start + "P.from".len()));
    }
    assert_ne!(bodies[0], bodies[1]);
}

#[test]
fn field_dispatch_receiver_is_the_projection_argument() {
    let (unit, program) = program(
        "type Record { n: Integer }\nget = r => r.n\na = get (Record 4)\na + 1",
    );
    let tables = unit.tables();
    let fields: Vec<_> = tables
        .dispatch
        .iter()
        .filter(|member| member.label == "n")
        .flat_map(|member| &member.rows)
        .filter(|row| matches!(row.target, ir::Implementation::Field(0)))
        .collect();
    assert!(!fields.is_empty(), "actual checked field dispatch");
    assert!(fields.iter().all(|row| row.receiver_spot == Some(0)));
    assert!(
        program.bodies.iter().flat_map(|body| &body.blocks).any(
            |block| matches!(
                block.terminator,
                mir::Terminator::Call {
                    callee: mir::Callee::Dispatch {
                        witness: mir::Witness::Value(_),
                        ..
                    },
                    ..
                }
            )
        ),
        "the generic accessor must retain its value receiver witness"
    );
}

#[test]
fn field_dispatch_receiver_projects_a_function_typed_temporary_before_calling()
{
    let (_, program) = program(
        "type F { op: Integer -> Integer }\nf = F (x => x + 1)\nf.op 7",
    );
    let mut projected = false;
    for body in &program.bodies {
        for block in &body.blocks {
            let mut locals = block.statements.iter().filter_map(|statement| {
                let mir::Statement::Assign(local, mir::Rvalue::Field(_, 0)) =
                    statement
                else {
                    return None;
                };
                matches!(body.locals[*local].ty, ir::Type::Function { .. })
                    .then_some(*local)
            });
            if let mir::Terminator::Call {
                callee: mir::Callee::Value(mir::Operand::Local(local)),
                args,
                ..
            } = &block.terminator
            {
                if locals.any(|projected| projected == *local) {
                    assert_eq!(args.len(), 1);
                    projected = true;
                }
            }
        }
    }
    assert!(projected, "project the callable into its Function-typed local, then use the normal call continuation");
}

#[test]
fn receiver_adapter_static_open_spots_are_direct_definition_calls() {
    let source = "type M { n: Integer }\nimpl M { op = a b self => self.n + a * 10 + b }\np = M.op --self (M 2)\np 3 4";
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    assert!(!unit.tables().nodes.iter().any(|node| matches!(
        &node.kind,
        ir::Kind::Dispatch {
            receiver: ir::Receiver::Meaning(_),
            ..
        }
    )));
    assert!(unit.tables().nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Call { open, .. } if open.len() == 2)));
    let program = compile::compile(&unit);
    assert!(program.receiver_adapters.is_empty());
    assert!(program.bodies.iter().flat_map(|body| &body.blocks).any(
        |block| matches!(
            &block.terminator,
            mir::Terminator::Call {
                callee: mir::Callee::Body(_),
                ..
            }
        )
    ));
}

#[test]
fn receiver_adapter_uses_raw_formals_and_fresh_nonphantom_context_id() {
    let source = "type M<T> { n: Integer }\nimpl M<U> { op = a b self => self.n + a * 10 + b }\nget = m => m.op\nx: M<Integer> = M 2\n(get x) 3 4";
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let program = compile::compile(&unit);
    let tables = program.tables.borrow();
    let adapter = program
        .receiver_adapters
        .iter()
        .find(|adapter| tables.dispatch[adapter.member].label == "op")
        .unwrap();
    let mir::ReceiverTarget::Definition(index) = adapter.target else {
        panic!("definition")
    };
    let raw_index = program.definitions[index].unwrap();
    let raw = &program.bodies[raw_index];
    let body = &program.bodies[adapter.body];
    assert_eq!(raw.arity, 3);
    assert_eq!(body.arity, raw.arity);
    assert!(
        raw.hidden.is_empty(),
        "ordinary phantom owner is not forced hidden"
    );
    assert_eq!(body.hidden.len(), 1);
    let ir::Type::Meaning { arguments, .. } =
        &tables.dispatch[adapter.member].rows[adapter.row].receiver
    else {
        panic!("M receiver")
    };
    assert!(!arguments
        .iter()
        .any(|ty| *ty == ir::Type::Parameter(body.hidden[0])));
    assert!(body.blocks.iter().any(|block| matches!(&block.terminator, mir::Terminator::Call { callee: mir::Callee::Body(target), args, meanings, .. } if *target == raw_index && args == &[mir::Operand::Local(2), mir::Operand::Local(3), mir::Operand::Local(1)] && meanings.is_empty())));
    assert_eq!(program.receiver_contexts.len(), program.bodies.len());
    assert!(program.receiver_contexts[adapter.body].is_some());
}

#[test]
fn bound_member_head_is_a_hidden_dispatch_witness_but_unused_phantom_is_not() {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a").unwrap();
    let module=parse(lex("use host\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => host.get () }\nget = w => w.op\nunused = x => ()\nget (W 1)")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let tables = program.tables.borrow();
    let get = tables
        .definitions
        .iter()
        .position(|definition| definition.label == "get")
        .unwrap();
    let body = &program.bodies[program.definitions[get].unwrap()];
    assert_eq!(body.hidden.len(), 2);
    assert_eq!(body.locals[1].ty, ir::Type::Parameter(body.hidden[0]));
    assert_eq!(body.locals[0].ty, ir::Type::Parameter(body.hidden[1]));
    assert_ne!(body.locals[0].ty, body.locals[1].ty);
    let unused = tables
        .definitions
        .iter()
        .find(|definition| definition.label == "unused")
        .unwrap();
    assert!(unused.hidden.is_empty());
}

#[test]
fn declared_operator_function_result_has_an_inner_call_then_value_application()
{
    let mut host = Declarations::new();
    host.module(
        "host",
        "pub type M { pub n: Integer }\nimpl M { concat: (self: Self) -> a }",
    )
    .unwrap();
    let module=parse(lex("use host\ntext: String = \"x\"\nanswer: Integer = (host.M 7) ++ text\nanswer")).unwrap();
    let unit =
        check::check(&module, &host, &[], check::Options::default()).unwrap();
    let program = compile::compile(&unit);
    let tables = program.tables.borrow();
    let operation = tables
        .declarations
        .iter()
        .position(|declaration| declaration.label == "concat")
        .unwrap();
    let body = &program.bodies[program.main];
    let mut host_dest = None;
    let mut applied = false;
    for block in &body.blocks {
        if let mir::Terminator::Call {
            callee, args, dest, ..
        } = &block.terminator
        {
            if *callee == mir::Callee::Host(operation) {
                assert_eq!(args.len(), 1);
                let ir::Type::Function { parameter, result } =
                    &body.locals[*dest].ty
                else {
                    panic!("whole protocol Function")
                };
                assert!(
                    matches!(parameter.as_ref(),ir::Type::Meaning{index,..}if tables.meanings[*index].label=="String")
                );
                assert!(
                    matches!(result.as_ref(),ir::Type::Meaning{index,..}if tables.meanings[*index].label=="Integer")
                );
                host_dest = Some(*dest);
            }
            if let mir::Callee::Value(mir::Operand::Local(local)) = callee {
                let staged: Vec<_> = block
                    .statements
                    .iter()
                    .filter(|statement| {
                        !matches!(statement, mir::Statement::Charge(_))
                    })
                    .collect();
                let [mir::Statement::Assign(
                    private,
                    mir::Rvalue::Use(mir::Operand::Local(from)),
                ), mir::Statement::Assign(
                    argument,
                    mir::Rvalue::Use(mir::Operand::Global(0)),
                ), mir::Statement::Assign(
                    tuple,
                    mir::Rvalue::Admit { values, spot: 0 },
                ), mir::Statement::Assign(
                    callee_out,
                    mir::Rvalue::Field(mir::Operand::Local(callee_tuple), 0),
                ), mir::Statement::Assign(
                    argument_out,
                    mir::Rvalue::Field(mir::Operand::Local(argument_tuple), 1),
                )] = staged.as_slice()
                else {
                    panic!("the actual Host-returned function's private admission chain")
                };
                assert_eq!(Some(*from), host_dest);
                assert_eq!(
                    *values,
                    [
                        mir::Operand::Local(*private),
                        mir::Operand::Local(*argument)
                    ]
                );
                assert_eq!((callee_out, callee_tuple), (private, tuple));
                assert_eq!((argument_out, argument_tuple), (argument, tuple));
                assert_eq!(*local, *private);
                assert_eq!(args, &[mir::Operand::Local(*argument)]);
                applied = true;
            }
        }
    }
    assert!(host_dest.is_some() && applied);
}

#[test]
fn surviving_default_lookup_demands_its_generic_witness() {
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n";
    const SOURCE: &str = "use host\nmake = w => host.R { w: w }\nr = make (host.W 1)\nhost.after r.result\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let (site, scheme) = typed
        .schemes
        .iter()
        .find(|(site, _)| SOURCE.get(site.0..site.1) == Some("make"))
        .unwrap();
    assert_eq!(scheme.variables.len(), 1);
    let surviving: Vec<_> = typed.default_instances.values().collect();
    assert!(
        !surviving.is_empty(),
        "public-field fixture retains its default lookup"
    );
    assert!(
        surviving
            .iter()
            .any(|value| !value.typed.selected.is_empty()),
        "default's checked operation is nonempty"
    );
    assert_eq!(typed.hidden[site], [0], "only make's actual generic parameter is needed by its retained default");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let definition = program
        .tables
        .borrow()
        .definitions
        .iter()
        .position(|d| d.label == "make")
        .unwrap();
    let body = &program.bodies[program.definitions[definition].unwrap()];
    assert_eq!(body.hidden.len(), 1);
    assert!(body.locals.iter().any(|local| matches!(&local.ty, ir::Type::Meaning { arguments, .. } if arguments == &[ir::Type::Parameter(body.hidden[0])])));
}
#[test]
fn retained_private_default_expression_demands_its_generic_witness() {
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, result: T = w.op }\npub after: Integer -> Integer\npub read: R<a> -> a = r => r.result\n";
    const SOURCE: &str = "use host\nmake = w => host.R { w: w }\nr = make (host.W 1)\nhost.after (host.read r)\n";
    fn expressions<'a>(
        value: &'a fipe::typed::DefaultValue,
        found: &mut Vec<&'a fipe::typed::DeclaredExpression>,
    ) {
        match value {
            fipe::typed::DefaultValue::Expression(value) => found.push(value),
            fipe::typed::DefaultValue::Record { fields, .. } => {
                for (_, _, value) in fields {
                    expressions(value, found);
                }
            }
            fipe::typed::DefaultValue::Array { element, .. } => {
                expressions(element, found);
            }
            _ => (),
        }
    }
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let (site, scheme) = typed
        .schemes
        .iter()
        .find(|(site, _)| SOURCE.get(site.0..site.1) == Some("make"))
        .unwrap();
    assert_eq!(scheme.variables.len(), 1);
    let mut retained_without_lookup = Vec::new();
    for (site, bound) in &typed.bound {
        for (spot, value) in &bound.defaults {
            let mut found = Vec::new();
            expressions(value, &mut found);
            if !typed
                .default_instances
                .contains_key(&(site.0, site.1, *spot))
            {
                retained_without_lookup.extend(found);
            }
        }
    }
    for (site, value) in &typed.derived_defaults {
        let mut found = Vec::new();
        expressions(value, &mut found);
        for instance in found {
            if !typed.default_instances.values().any(|saved| {
                std::rc::Rc::ptr_eq(&saved.typed, &instance.typed)
                    && saved.value == instance.value
            }) {
                retained_without_lookup.push(instance);
            }
        }
        let _ = site;
    }
    for (site, steps) in &typed.accesses {
        for bound in steps.iter().filter_map(|step| step.bound.as_ref()) {
            for (spot, value) in &bound.defaults {
                let mut found = Vec::new();
                expressions(value, &mut found);
                if !typed
                    .default_instances
                    .contains_key(&(site.0, site.1, *spot))
                {
                    retained_without_lookup.extend(found);
                }
            }
        }
    }
    assert!(
        !retained_without_lookup.is_empty(),
        "actual executable Expression remains after temporary lookup rollback"
    );
    assert!(
        retained_without_lookup
            .iter()
            .any(|value| !value.typed.selected.is_empty()),
        "default's checked operation is nonempty"
    );
    assert_eq!(typed.hidden[site], [0], "only make's actual generic parameter is needed by its retained default");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let definition = program
        .tables
        .borrow()
        .definitions
        .iter()
        .position(|d| d.label == "make")
        .unwrap();
    let body = &program.bodies[program.definitions[definition].unwrap()];
    assert_eq!(body.hidden.len(), 1);
    assert!(body.locals.iter().any(|local| matches!(&local.ty, ir::Type::Meaning { arguments, .. } if arguments == &[ir::Type::Parameter(body.hidden[0])])));
}

#[test]
fn annotated_default_lookup_keeps_caller_binder_correspondence() {
    const HOST: &str = "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n";
    const SOURCE: &str = "use host\nmake: host.W<a> -> host.R<a> = w => host.R { w: w }\nr = make (host.W 1)\nhost.after r.result\n";
    let mut declarations = Declarations::new();
    declarations.module("host", HOST).unwrap();
    let module = parse(lex(SOURCE)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let (site, scheme) = typed
        .schemes
        .iter()
        .find(|(site, _)| SOURCE.get(site.0..site.1) == Some("make"))
        .unwrap();
    assert_eq!(scheme.variables.len(), 1);
    let surviving: Vec<_> = typed.default_instances.values().collect();
    assert!(
        !surviving.is_empty(),
        "public-field fixture retains its default lookup"
    );
    assert!(
        surviving
            .iter()
            .any(|value| !value.typed.selected.is_empty()),
        "default's checked operation is nonempty"
    );
    assert_eq!(typed.hidden[site], [0], "only make's actual generic parameter is needed by its retained default");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let definition = program
        .tables
        .borrow()
        .definitions
        .iter()
        .position(|d| d.label == "make")
        .unwrap();
    let body = &program.bodies[program.definitions[definition].unwrap()];
    assert_eq!(body.hidden.len(), 1);
    assert!(body.locals.iter().any(|local| matches!(&local.ty, ir::Type::Meaning { arguments, .. } if arguments == &[ir::Type::Parameter(body.hidden[0])])));
}

#[test]
fn inherited_predicate_arguments_keep_callers_nominal_home() {
    const HOST: &str = "pub type Box { pub tag: String }\npub type Base<T>: Vec<T> where self.len > 0\npub type Derived<T>: Base<T> where self.len < 3\npub items: a -> Vec<a>\npub after: Vec<a> -> Integer\n";
    const SOURCE: &str = "use host\ntype Box { n: Integer }\nother = host.Box \"host\"\nxs: host.Derived<Box> = host.items (Box 7)\nhost.after xs\n";
    let mut declarations = Declarations::new();
    declarations
        .module("host", HOST)
        .expect("inherited Host limits");
    let module = parse(lex(SOURCE)).expect("caller nominal");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("actual checked inheritance");
    let tables = unit.tables();
    let host = tables
        .namespaces
        .iter()
        .position(|name| name == "host")
        .unwrap();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .unwrap();
    let local_box = tables
        .meanings
        .iter()
        .position(|meaning| meaning.namespace == main && meaning.label == "Box")
        .expect("main Box");
    let host_box = tables
        .meanings
        .iter()
        .position(|meaning| meaning.namespace == host && meaning.label == "Box")
        .expect("distinct Host Box retained");
    assert_ne!(local_box, host_box);
    let test = |text: &str| {
        tables
            .tests
            .iter()
            .position(|test| {
                test.namespace == host
                    && HOST.get(test.span.start..test.span.end) == Some(text)
            })
            .expect("actual checked test body")
    };
    let base = test("self.len > 0");
    let derived = test("self.len < 3");
    assert_ne!(base, derived);
    let expected = vec![ir::Type::Meaning {
        index: local_box,
        arguments: vec![],
    }];
    let mut actual = Vec::new();
    let mut combined = false;
    for entry in &unit.plan {
        let mut pending = vec![&entry.predicate];
        let mut relevant = Vec::new();
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts.iter().rev());
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Test { index, meanings }
                    if *index == base || *index == derived =>
                {
                    assert_eq!(
                        meanings, &expected,
                        "EVERY inherited/own entry must retain caller Box"
                    );
                    assert_ne!(entry.phase, ir::Phase::Compile);
                    relevant.push(*index);
                    actual.push(*index);
                }
                _ => (),
            }
        }
        combined |= relevant == [base, derived];
    }
    assert!(actual.contains(&base) && actual.contains(&derived));
    assert!(
        combined,
        "inherited test precedes own test in combined entry"
    );
    drop(tables);
    let program = compile::compile(&unit);
    assert!(program.tests[base].is_some() && program.tests[derived].is_some());
}

#[test]
fn inherited_entry_arguments_are_instantiated_per_use_and_alias() {
    const HOST: &str = "pub type Base<T>: Vec<T> where self.len > 0\npub type Derived<T>: Base<T> where self.len < 3\npub items: a -> Vec<a>\n";
    for expressions in [
        "ints: Alias<Integer> = host.items 7\ntexts: Alias<String> = host.items \"x\"\n(ints, texts)",
        "texts: Alias<String> = host.items \"x\"\nints: Alias<Integer> = host.items 7\n(texts, ints)",
    ] {
        let mut declarations = Declarations::new();
        declarations.module("host", HOST).expect("generic limits");
        let source = format!("use host\ntype Alias<T>: host.Derived<T> where self.len < 4\n{expressions}\n");
        let module = parse(lex(&source)).expect("local inherited alias");
        let unit = check::check(&module, &declarations, &[], check::Options::default())
            .expect("both actual instantiations");
        let tables = unit.tables();
        let host = tables.namespaces.iter().position(|n| n == "host").unwrap();
        let main = tables.namespaces.iter().position(|name| name == "main").unwrap();
        assert_eq!(tables.tests.iter().filter(|test| test.namespace == main && source.get(test.span.start..test.span.end) == Some("self.len < 4")).count(), 1, "repeated instantiations must not duplicate the local predicate");
        let mut seen = Vec::new();
        for entry in &unit.plan {
            let mut pending = vec![&entry.predicate];
            while let Some(predicate) = pending.pop() {
                match predicate {
                    ir::Predicate::All(parts) | ir::Predicate::Any(parts) => pending.extend(parts),
                    ir::Predicate::Not(inner) => pending.push(inner),
                    ir::Predicate::Test { index, meanings } if tables.tests[*index].namespace == host => {
                        assert_eq!(meanings.len(), 1);
                        let ir::Type::Meaning { index, arguments } = &meanings[0] else { panic!("concrete caller argument") };
                        assert!(arguments.is_empty());
                        let meaning = &tables.meanings[*index];
                        assert_eq!(tables.namespaces[meaning.namespace], "prelude");
                        assert!(matches!(meaning.label.as_str(), "Integer" | "String"));
                        seen.push(meaning.label.as_str());
                    }
                    _ => (),
                }
            }
        }
        assert!(seen.contains(&"Integer") && seen.contains(&"String"));
    }
}

#[test]
fn inherited_entry_formals_do_not_capture_qualified_nominal_names() {
    const HOST: &str =
        "pub type Base<T>: Vec<T> where self.len > 0\npub items: a -> Vec<a>\n";
    let mut declarations = Declarations::new();
    declarations
        .module("other", "pub type T { pub n: Integer }\n")
        .expect("external T");
    declarations
        .module("host", HOST)
        .expect("inherited predicate");
    let source = "use other\nuse host\ntype Alias<T>: host.Base<other.T> where self.len < 3\nxs: Alias<Integer> = host.items (other.T 7)\nxs\n";
    let module =
        parse(lex(source)).expect("qualified type and same-spelled formal");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("qualified name keeps its own identity");
    let tables = unit.tables();
    let host = tables
        .namespaces
        .iter()
        .position(|name| name == "host")
        .unwrap();
    let other = tables
        .namespaces
        .iter()
        .position(|name| name == "other")
        .unwrap();
    let external_t = tables
        .meanings
        .iter()
        .position(|meaning| meaning.namespace == other && meaning.label == "T")
        .unwrap();
    let base = tables
        .tests
        .iter()
        .position(|test| {
            test.namespace == host
                && HOST.get(test.span.start..test.span.end)
                    == Some("self.len > 0")
        })
        .expect("real inherited Base test");
    let expected = vec![ir::Type::Meaning {
        index: external_t,
        arguments: vec![],
    }];
    let mut found = 0;
    for entry in &unit.plan {
        let mut pending = vec![&entry.predicate];
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Test { index, meanings } if *index == base => {
                    assert_eq!(meanings, &expected, "inherited Base uses qualified other.T, never formal Integer");
                    assert_ne!(
                        entry.phase,
                        ir::Phase::Compile,
                        "Host result keeps predicate live"
                    );
                    found += 1;
                }
                _ => (),
            }
        }
    }
    assert!(found > 0, "actual inherited predicate leaves are required");
}

#[test]
fn inherited_entry_instantiates_a_formal_only_base() {
    const HOST: &str =
        "pub type Base<T>: Vec<T> where self.len > 0\npub items: a -> Vec<a>\n";
    let mut declarations = Declarations::new();
    declarations
        .module("host", HOST)
        .expect("generic tested base");
    let source = "use host\ntype Identity<T>: T where True\nxs: Identity<host.Base<Integer>> = host.items 7\nxs\n";
    let module = parse(lex(source)).expect("formal-only alias base");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("actual formal base is instantiated");
    let tables = unit.tables();
    let host = tables
        .namespaces
        .iter()
        .position(|name| name == "host")
        .unwrap();
    let prelude = tables
        .namespaces
        .iter()
        .position(|name| name == "prelude")
        .unwrap();
    let integer = tables
        .meanings
        .iter()
        .position(|meaning| {
            meaning.namespace == prelude && meaning.label == "Integer"
        })
        .unwrap();
    let base = tables
        .tests
        .iter()
        .position(|test| {
            test.namespace == host
                && HOST.get(test.span.start..test.span.end)
                    == Some("self.len > 0")
        })
        .expect("formal actual retains Base predicate");
    let expected = vec![ir::Type::Meaning {
        index: integer,
        arguments: vec![],
    }];
    let mut found = 0;
    for entry in &unit.plan {
        let mut pending = vec![&entry.predicate];
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Test { index, meanings } if *index == base => {
                    assert_eq!(meanings, &expected);
                    assert_ne!(entry.phase, ir::Phase::Compile);
                    found += 1;
                }
                _ => (),
            }
        }
    }
    assert!(
        found > 0,
        "formal-only base must contribute actual inherited checks"
    );
}

#[test]
fn declared_reply_dependencies_keep_echo_and_structured_witnesses_but_not_sink_inputs(
) {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub echo: a -> a\npub wrapped: a -> Maybe<a>\npub sink: a -> ()",
        )
        .expect("generic declared contracts");
    let module = parse(lex("use host\nforward = x => host.echo x\nstructured = x => host.wrapped x\nignore = x => host.sink x\nplain = x => x\n(forward 7, structured 7, ignore 7, plain 7)")).expect("wrappers");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .expect("checked generic wrappers");
    let tables = unit.tables();
    for (label, count) in [
        ("forward", 1),
        ("structured", 1),
        ("ignore", 0),
        ("plain", 0),
    ] {
        let (index, definition) = tables
            .definitions
            .iter()
            .enumerate()
            .find(|(_, definition)| definition.label == label)
            .expect("actual wrapper definition");
        assert_eq!(
            definition.parameters.len(),
            1,
            "one actual generic variable for {label}"
        );
        assert_eq!(
            definition.hidden.len(),
            count,
            "only actual result-dependent witness for {label}"
        );
        assert!(tables.nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Definition { index: target, meanings } if *target == index && meanings.len() == count && meanings.iter().all(|ty| matches!(ty, ir::Type::Meaning { index, arguments } if tables.meanings[*index].label == "Integer" && arguments.is_empty())))), "actual instantiated {label} reference preserves the expected witness count and Integer identity");
    }
    let program = compile::compile(&unit);
    assert!(!program.bodies.is_empty());
}

fn predicate_conformance_program(
    source: &str,
) -> (fipe::typed::Typed, ir::Unit, mir::Program) {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub items: () -> Vec<Integer>\npub after: () -> Integer\n",
        )
        .expect("original Host declarations");
    let module = parse(lex(source)).expect("conformance source parses");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("conformance source resolves");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "inference: {errors:?}\n{source}");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_or_else(|errors| panic!("checking: {errors:?}\n{source}"));
    let program = compile::compile(&unit);
    (typed, unit, program)
}

/// Read exactly the test leaves that an immediate Run can invoke.
fn predicate_conformance_run(
    predicate: &ir::Predicate,
    test: usize,
    arguments: &mut Vec<ir::Type>,
) {
    match predicate {
        ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
            for part in parts {
                predicate_conformance_run(part, test, arguments);
            }
        }
        ir::Predicate::Not(inner) => {
            predicate_conformance_run(inner, test, arguments);
        }
        ir::Predicate::Test { index, meanings } if *index == test => {
            arguments.extend(meanings.iter().cloned());
        }
        // Function is true in Run; it does not run its nested test leaves.
        _ => (),
    }
}

fn predicate_conformance_entry(
    predicate: &ir::Predicate,
    test: usize,
) -> Vec<ir::Type> {
    let mut arguments = Vec::new();
    let mut rest = predicate;
    if matches!(rest, ir::Predicate::Function { .. }) {
        while let ir::Predicate::Function { parameter, result } = rest {
            predicate_conformance_run(parameter, test, &mut arguments);
            rest = result;
        }
    } else {
        predicate_conformance_run(rest, test, &mut arguments);
    }
    arguments
}

#[derive(Debug)]
struct PredicateConformanceEntry {
    entry: usize,
    body: usize,
    producer: usize,
    local: bool,
    path: Vec<ir::Projection>,
    arguments: Vec<ir::Type>,
}

fn predicate_conformance_entries(
    source: &str,
    unit: &ir::Unit,
    program: &mir::Program,
) -> Vec<PredicateConformanceEntry> {
    let tables = unit.tables();
    let main = tables
        .namespaces
        .iter()
        .position(|name| name == "main")
        .expect("main namespace");
    let test = tables
        .tests
        .iter()
        .position(|test| {
            test.namespace == main
                && source.get(test.span.start..test.span.end)
                    == Some("is_sorted self")
        })
        .expect("actual Sorted test source identity");
    let test_body = program.tests[test]
        .expect("Sorted test is actually reached by lowering");
    println!(
        "Sorted test {test} formal IDs {:?}, body {test_body} hidden {:?}",
        tables.tests[test].parameters, program.bodies[test_body].hidden
    );
    let mut found = Vec::new();
    for (body_index, body) in program.bodies.iter().enumerate() {
        for block in &body.blocks {
            for (statement_index, statement) in
                block.statements.iter().enumerate()
            {
                let mir::Statement::Enter { entry, value } = statement else {
                    continue;
                };
                let recorded = &tables.entries[*entry];
                let arguments =
                    predicate_conformance_entry(&recorded.predicate, test);
                if arguments.is_empty() {
                    continue;
                }
                let node = block.nodes[statement_index];
                let ir::Kind::Enter {
                    entry: actual_entry,
                    value: producer,
                } = &tables.nodes[node].kind
                else {
                    panic!("MIR entry must retain actual IR Enter node {node}")
                };
                assert_eq!(actual_entry, entry);
                let local = matches!(value, mir::Operand::Local(_));
                if matches!(recorded.predicate, ir::Predicate::Function { .. })
                {
                    assert!(
                        local,
                        "compiler materializes a top Function entry as a Local"
                    );
                }
                println!("entry {entry} path {:?}, predicate {:?}, producer {producer} kind {:?} span {:?} type {:?}, body {body_index} hidden {:?}, operand {value:?}, arguments {arguments:?}", recorded.path, recorded.predicate, tables.nodes[*producer].kind, tables.nodes[*producer].span, tables.nodes[*producer].ty, body.hidden);
                found.push(PredicateConformanceEntry {
                    entry: *entry,
                    body: body_index,
                    producer: *producer,
                    local,
                    path: recorded.path.clone(),
                    arguments,
                });
            }
        }
    }
    assert!(
        !found.is_empty(),
        "actual Sorted Enter/MIR associations required"
    );
    found
}

#[test]
fn predicate_witness_conformance_original_sorted_entries() {
    const GENERIC: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Sorted<a> -> Integer = xs => host.after ()\napply = f x => f x\ngo = xs => apply keep xs\ngo (host.items ())\n";
    const CONCRETE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nx: Sorted<Integer> = host.items ()\nhost.after ()\n";
    for (generic, source) in [(true, GENERIC), (false, CONCRETE)] {
        let (typed, unit, program) = predicate_conformance_program(source);
        let entries = predicate_conformance_entries(source, &unit, &program);
        let tables = unit.tables();
        if generic {
            let start = source.find("go =").unwrap();
            let site = (start, start + 2);
            assert_eq!(
                typed.body_parameters[&site].len(),
                1,
                "original go has one generic body parameter"
            );
            assert_eq!(
                typed.hidden[&site],
                [0],
                "only the original predicate parameter is required"
            );
            let definition = tables
                .definitions
                .iter()
                .position(|definition| {
                    definition.label == "go" && definition.span.start == start
                })
                .expect("semantic go definition");
            let body =
                program.definitions[definition].expect("go body is reached");
            let mut actual = 0;
            for observed in
                entries.iter().filter(|observed| observed.body == body)
            {
                assert!(matches!(
                    tables.entries[observed.entry].predicate,
                    ir::Predicate::Function { .. }
                ));
                for argument in &observed.arguments {
                    let ir::Type::Parameter(parameter) = argument else {
                        panic!("generic Test argument: {argument:?}")
                    };
                    assert!(tables.definitions[definition]
                        .parameters
                        .contains(parameter));
                    assert!(program.bodies[body].hidden.contains(parameter), "actual creator must transport Test argument {parameter}");
                    actual += 1;
                }
            }
            assert!(
                actual > 0,
                "go's actual Function entry, not another body's entry"
            );
        } else {
            let integer = tables
                .meanings
                .iter()
                .position(|meaning| {
                    tables.namespaces[meaning.namespace] == "prelude"
                        && meaning.label == "Integer"
                })
                .expect("actual Integer meaning");
            for argument in
                entries.iter().flat_map(|observed| &observed.arguments)
            {
                assert_eq!(
                    argument,
                    &ir::Type::Meaning {
                        index: integer,
                        arguments: Vec::new()
                    }
                );
            }
        }
    }
}

#[test]
fn predicate_witness_conformance_tuple_arguments_keep_creator_validation() {
    const SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ntake: Ord a => (Sorted<a> * Integer) -> Integer = pair => host.after ()\ngo = xs => take (xs, 0)\ngo (host.items ())\n";
    let (typed, unit, program) = predicate_conformance_program(SOURCE);
    let entries = predicate_conformance_entries(SOURCE, &unit, &program);
    let tables = unit.tables();
    let start = SOURCE.find("go =").unwrap();
    let site = (start, start + 2);
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "go" && definition.span.start == start
        })
        .expect("go definition");
    let body = program.definitions[definition].expect("go body");
    let tuple_start = SOURCE.find("(xs, 0)").unwrap();
    let tuple = tables.nodes.iter().position(|node| {
        node.span.start == tuple_start && matches!(&node.kind, ir::Kind::Tuple(parts) if parts.len() == 2)
    }).expect("the actual tuple supplied to take");
    let ir::Kind::Tuple(parts) = &tables.nodes[tuple].kind else {
        unreachable!()
    };
    let first = predicate_conformance_local_root(&tables, parts[0]);
    let ir::Type::Meaning {
        index,
        arguments: element,
    } = &tables.nodes[first].ty
    else {
        panic!("actual xs Vec type")
    };
    assert_eq!(tables.meanings[*index].label, "Vec");
    assert_eq!(
        tables.namespaces[tables.meanings[*index].namespace],
        "prelude"
    );
    let mut actual = 0;
    let mut direct = 0;
    for observed in entries.iter().filter(|observed| observed.body == body) {
        assert_eq!(
            &observed.arguments, element,
            "Sorted tests the actual Vec element type"
        );
        for argument in &observed.arguments {
            let ir::Type::Parameter(parameter) = argument else {
                panic!("tuple argument Test parameter {argument:?}")
            };
            let position = tables.definitions[definition]
                .parameters
                .iter()
                .position(|known| known == parameter)
                .expect("every retained Test parameter belongs to its creator");
            assert!(typed.hidden[&site].contains(&position));
            assert!(program.bodies[body].hidden.contains(parameter));
            actual += 1;
        }
        if observed.path.is_empty() {
            let producer =
                predicate_conformance_local_root(&tables, observed.producer);
            assert_eq!(
                producer, first,
                "retained validation is on the tuple's actual first operand"
            );
            assert!(observed.local);
            direct += 1;
        }
    }
    assert!(
        actual > 0 && direct > 0,
        "actual Sorted validation of the tuple's xs operand must remain"
    );
    // Canonical facts make own==limit elide the former duplicate Part(0) gate.
    // Genuine nonempty projection is covered separately, not inferred here.
}

fn predicate_conformance_local_root(
    tables: &ir::Tables,
    mut node: usize,
) -> usize {
    for _ in 0..=tables.nodes.len() {
        match tables.nodes[node].kind {
            ir::Kind::Enter { value, .. }
            | ir::Kind::Cast { value, .. }
            | ir::Kind::Pack { value, .. } => node = value,
            ir::Kind::Local { .. } => return node,
            ref kind => panic!(
                "expected the actual local under validation wrappers: {kind:?}"
            ),
        }
    }
    panic!("cyclic validation wrappers")
}

const PREDICATE_FACT_SORTED: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\n";

fn predicate_fact_creator_arguments(source: &str, declarations_text: &str) {
    predicate_fact_named_creator_arguments(source, declarations_text, "go");
}

fn predicate_fact_named_creator_arguments(
    source: &str,
    declarations_text: &str,
    label: &str,
) {
    let mut declarations = Declarations::new();
    declarations
        .module("host", declarations_text)
        .expect("fact ownership Host declarations");
    let module = parse(lex(source)).expect("fact ownership source parses");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("fact ownership resolves");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(
        errors.is_empty(),
        "fact ownership inference: {errors:?}\n{source}"
    );
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_or_else(|errors| {
                panic!("fact ownership check: {errors:?}\n{source}")
            });
    let program = compile::compile(&unit);
    let entries = predicate_conformance_entries(source, &unit, &program);
    let tables = unit.tables();
    let start = source.find(&format!("{label} =")).unwrap();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == label && definition.span.start == start
        })
        .expect("go definition identity");
    let body = program.definitions[definition].expect("go body");
    let mut actual = 0;
    for entry in entries.iter().filter(|entry| entry.body == body) {
        for argument in &entry.arguments {
            let ir::Type::Parameter(parameter) = argument else {
                panic!("generic fact must use go's parameter, not an outer concrete type: {argument:?}")
            };
            let position = tables.definitions[definition]
                .parameters
                .iter()
                .position(|known| known == parameter)
                .expect("fact Test uses the actual creator binder");
            assert!(
                typed.hidden[&(start, start + label.len())].contains(&position)
            );
            assert!(program.bodies[body].hidden.contains(parameter));
            actual += 1;
        }
    }
    assert!(
        actual > 0,
        "actual caller guard must retain the returned/aliased Sorted contract"
    );
}

#[test]
fn generic_predicate_facts_instantiate_each_alias_at_its_read() {
    let source = format!("{PREDICATE_FACT_SORTED}keep: Ord a => Sorted<a> -> Integer = xs => host.after ()\napply = f x => f x\nh = keep\ngo = xs => apply h xs\ngo (host.items ())\n");
    predicate_fact_creator_arguments(
        &source,
        "pub items: () -> Vec<Integer>\npub after: () -> Integer\n",
    );
}

#[test]
fn generic_predicate_facts_keep_independent_nested_annotation_binders() {
    let declarations = "pub items: () -> Vec<Integer>\npub texts: () -> Vec<String>\npub after: () -> Integer\n";
    for (outer, input) in
        [("\"outer\"", "host.items ()"), ("7", "host.texts ()")]
    {
        let source = format!("{PREDICATE_FACT_SORTED}apply = f x => f x\nouter: Ord b => a -> Sorted<b> -> Integer = _ => {{\n inner: Ord a => Sorted<a> -> Integer = xs => 0\n inner\n}}\ngo = xs => apply (outer {outer}) xs\ngo ({input})\n");
        predicate_fact_creator_arguments(&source, declarations);
    }
}

#[test]
fn generic_predicate_facts_keep_same_unit_instantiations_in_both_orders() {
    let declarations = "pub items: () -> Vec<Integer>\npub texts: () -> Vec<String>\npub after: () -> Integer\n";
    let ints = "ints = xs => apply (outer \"outer\") xs\n";
    let texts = "texts = xs => apply (outer 7) xs\n";
    for (first, second, uses) in [
        (ints, texts, "(ints (host.items ()), texts (host.texts ()))"),
        (texts, ints, "(texts (host.texts ()), ints (host.items ()))"),
    ] {
        let source = format!("{PREDICATE_FACT_SORTED}apply = f x => f x\nouter: Ord b => a -> Sorted<b> -> Integer = _ => {{\n inner: Ord a => Sorted<a> -> Integer = xs => 0\n inner\n}}\n{first}{second}{uses}\n");
        predicate_fact_named_creator_arguments(&source, declarations, "ints");
        predicate_fact_named_creator_arguments(&source, declarations, "texts");
    }
}

fn predicate_session_accept(
    session: &mut fipe::session::Session,
    source: &str,
) -> fipe::session::Report {
    use fipe::embed::Outcome;
    let Outcome::Done(report) = session.line(source) else {
        panic!("checking declarations must not request the Host")
    };
    assert!(
        report.diagnostics.is_empty(),
        "{source}: {:?}",
        report.diagnostics
    );
    assert!(report.accepted.is_some());
    report
}

#[test]
fn generic_predicate_facts_read_the_exact_prior_session_owner() {
    const SOURCE: &str = "go = xs => apply keep xs";
    use fipe::{
        resolve::{Origin, Target},
        session::Session,
    };

    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub items: () -> Vec<Integer>\npub after: () -> Integer\n",
        )
        .unwrap();
    let mut session = Session::new(declarations);
    let initial = predicate_session_accept(&mut session, PREDICATE_FACT_SORTED);
    let initial_unit = initial.unit.expect("Sorted source unit");
    let test = initial_unit
        .tables()
        .tests
        .iter()
        .position(|test| {
            PREDICATE_FACT_SORTED.get(test.span.start..test.span.end)
                == Some("is_sorted self")
        })
        .expect("actual prior Sorted test source");
    let keep = predicate_session_accept(
        &mut session,
        "keep: Ord a => Sorted<a> -> Integer = xs => host.after ()",
    );
    predicate_session_accept(&mut session, "apply = f x => f x");
    let report = predicate_session_accept(&mut session, SOURCE);
    let start = report.offset;
    let read = report.offset + SOURCE.find("keep").unwrap();
    let module = report.module.as_ref().unwrap();
    let fipe::ast::Node::Expr(id) = module.at(read).expect("actual keep read")
    else {
        panic!("keep read expression")
    };
    assert!(
        matches!(
            report.resolved.as_ref().unwrap().expr(id),
            Some(Target::Local { .. })
        ),
        "the regression specifically uses a resolver Local for a prior line"
    );
    let typed = report.typed.as_ref().unwrap();
    let read_span = module.expr(id).unwrap().span;
    let fipe::typed::Selected::Value {
        origin: Origin::Line(_),
        site,
    } = &typed.selected[&(read_span.start, read_span.end)]
    else {
        panic!("actual selected owner must be the prior accepted line")
    };
    assert!(
        site.start >= keep.offset && site.end <= keep.offset + "keep".len()
    );
    let unit = report.unit.as_ref().unwrap();
    let program = compile::compile(unit);
    let tables = unit.tables();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "go" && definition.span.start == start
        })
        .expect("this line's go definition");
    let body = program.definitions[definition].expect("go body");
    let mut actual = 0;
    for statement in program.bodies[body]
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
    {
        let mir::Statement::Enter { entry, .. } = statement else {
            continue;
        };
        for argument in
            predicate_conformance_entry(&tables.entries[*entry].predicate, test)
        {
            let ir::Type::Parameter(parameter) = argument else {
                panic!("generic prior-line contract")
            };
            let position = tables.definitions[definition].parameters.iter().position(|known| *known == parameter).expect("prior fact was instantiated to this creator, not the prior scheme's Var ID");
            assert!(typed.hidden[&(start, start + 2)].contains(&position));
            assert!(program.bodies[body].hidden.contains(&parameter));
            actual += 1;
        }
    }
    assert!(
        actual > 0,
        "actual prior-Sorted Function entry in this line's go"
    );
}

fn predicate_annotated_go_site(
    module: &fipe::ast::Module,
) -> fipe::token::Span {
    module
        .entries
        .iter()
        .find_map(|entry| {
            let fipe::ast::Entry::Line(id) = entry else {
                return None;
            };
            let fipe::ast::ExprKind::Bind {
                pattern,
                annotation: Some(_),
                ..
            } = &module.expr(*id)?.kind
            else {
                return None;
            };
            let pattern = module.pattern(*pattern)?;
            let fipe::ast::PatternKind::Name(name) = &pattern.kind else {
                return None;
            };
            (name.segments.as_slice() == ["go"]).then_some(name.span)
        })
        .expect("actual annotated go binding, not a literal go-equals search")
}

#[test]
fn predicate_witness_conformance_unknown_tuple_result_keeps_projection() {
    const SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ngo: Ord a => a -> (Vec<a> * Integer) = seed => {\n pair: (Sorted<a> * Integer) = host.pair seed\n pair\n}\ngo 7\n";
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub pair: a -> (Vec<a> * Integer)\n")
        .expect("public generic tuple producer");
    let module = parse(lex(SOURCE)).expect("generic projected Sorted source");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("projected source resolves");
    let (typed, errors) =
        fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
    assert!(errors.is_empty(), "projection inference: {errors:?}");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_or_else(|errors| panic!("projection check: {errors:?}"));
    let program = compile::compile(&unit);
    let entries = predicate_conformance_entries(SOURCE, &unit, &program);
    let site = predicate_annotated_go_site(&module);
    let tables = unit.tables();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "go" && definition.span == site
        })
        .expect("generic go definition");
    let body =
        program.definitions[definition].expect("go body is actually lowered");
    let mut actual = 0;
    for observed in entries.iter().filter(|entry| {
        entry.body == body && entry.path == [ir::Projection::Part(0)]
    }) {
        assert!(observed.local, "the projected whole tuple occupies a Local");
        let ir::Kind::Call { callee, .. } =
            tables.nodes[observed.producer].kind
        else {
            panic!("entry's actual child must be the unknown Host tuple Call")
        };
        let ir::Kind::Declared(declaration) = tables.nodes[callee].kind else {
            panic!("actual Host pair callee")
        };
        assert_eq!(tables.declarations[declaration].label, "pair");
        assert_eq!(
            tables.namespaces[tables.declarations[declaration].namespace],
            "host"
        );
        let ir::Type::Tuple(parts) = &tables.nodes[observed.producer].ty else {
            panic!("actual whole tuple producer type")
        };
        assert_eq!(parts.len(), 2);
        let ir::Type::Meaning { index, arguments } = &parts[0] else {
            panic!("actual tuple's Vec component")
        };
        assert_eq!(tables.meanings[*index].label, "Vec");
        assert_eq!(
            tables.namespaces[tables.meanings[*index].namespace],
            "prelude"
        );
        assert_eq!(
            &observed.arguments, arguments,
            "Sorted argument is exactly this tuple component's element type"
        );
        assert!(!matches!(
            tables.entries[observed.entry].predicate,
            ir::Predicate::Function { .. }
        ));
        for argument in &observed.arguments {
            let ir::Type::Parameter(parameter) = argument else {
                panic!("generic projected Test argument: {argument:?}")
            };
            let position = tables.definitions[definition]
                .parameters
                .iter()
                .position(|known| known == parameter)
                .expect("projected Test belongs to actual creator");
            assert!(typed.hidden[&(site.start, site.end)].contains(&position));
            assert!(program.bodies[body].hidden.contains(parameter));
            actual += 1;
        }
    }
    assert!(
        actual > 0,
        "genuine nonempty Part(0) Sorted entry on unknown Host tuple result"
    );
}

fn raw_host_default_expressions<'a>(
    value: &'a fipe::typed::DefaultValue,
    found: &mut Vec<&'a fipe::typed::DeclaredExpression>,
) {
    use fipe::typed::DefaultValue;
    match value {
        DefaultValue::Expression(instance) => found.push(instance),
        DefaultValue::Record { fields, .. } => {
            for (_, _, value) in fields {
                raw_host_default_expressions(value, found);
            }
        }
        DefaultValue::Array { element, .. } => {
            raw_host_default_expressions(element, found);
        }
        _ => {}
    }
}
fn raw_host_vec_element(ty: &ir::Type, tables: &ir::Tables) -> ir::Type {
    let ir::Type::Meaning { index, arguments } = ty else {
        panic!("actual Vec input")
    };
    assert_eq!(tables.meanings[*index].label, "Vec");
    assert_eq!(
        tables.namespaces[tables.meanings[*index].namespace],
        "prelude"
    );
    let [element] = arguments.as_slice() else {
        panic!("one actual Vec argument")
    };
    element.clone()
}
fn raw_host_default_identity(
    host: &str,
    host_module: &fipe::ast::Module,
) -> (fipe::ast::ExprId, (usize, usize), (usize, usize)) {
    let helper_start = host.find("helper:").unwrap();
    let helper_site = (helper_start, helper_start + "helper".len());
    let default_root = host_module
        .entries
        .iter()
        .find_map(|entry| {
            let fipe::ast::Entry::Item(item) = entry else {
                return None;
            };
            let fipe::ast::ItemKind::Type {
                name,
                members: Some(members),
                ..
            } = &item.kind
            else {
                return None;
            };
            if name.segments.as_slice() != ["R"] {
                return None;
            }
            members.iter().find_map(|member| {
                let fipe::ast::Member::Field(field) = member else {
                    return None;
                };
                (field.name.segments.as_slice() == ["checked"] && !field.public)
                    .then_some(field.default)
                    .flatten()
            })
        })
        .expect("actual private field default");
    let root = host_module.expr(default_root).unwrap();
    assert_eq!(
        host.get(root.span.start..root.span.end),
        Some("(helper items)")
    );
    let fipe::ast::ExprKind::Group(parts) = &root.kind else {
        panic!("grouped field default")
    };
    let [call] = parts.as_slice() else {
        panic!("one default call")
    };
    let fipe::ast::ExprKind::Call { callee, .. } =
        &host_module.expr(*call).unwrap().kind
    else {
        panic!("actual helper call")
    };
    let read = host_module.expr(*callee).unwrap();
    assert_eq!(host.get(read.span.start..read.span.end), Some("helper"));
    let read_site = (read.span.start, read.span.end);
    (default_root, helper_site, read_site)
}

fn raw_host_helper_case(
    host: &str,
    source: &str,
    predicate: bool,
    default_root: fipe::ast::ExprId,
    helper_site: (usize, usize),
    read_site: (usize, usize),
) {
    use fipe::{resolve::Origin, types::Type};
    let mut declarations = Declarations::new();
    declarations
        .module("host", host)
        .expect("raw Host fixture admits");
    let module = parse(lex(source)).unwrap();
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let make_start = source.find("make =").unwrap();
    let make_site = (make_start, make_start + "make".len());
    let scheme = &typed.schemes[&make_site];
    let [variable] = scheme.variables.as_slice() else {
        panic!("one canonical maker variable")
    };
    let Type::Function { parameter, result } = &scheme.ty else {
        panic!("actual maker callable")
    };
    for (ty, label, origin) in [
        (parameter.as_ref(), "Vec", Origin::Prelude),
        (result.as_ref(), "R", Origin::Host("host".into())),
    ] {
        let Type::Named { name, arguments } = ty else {
            panic!("actual generic maker type")
        };
        assert_eq!((&name.name, &name.origin), (&label.to_string(), &origin));
        assert_eq!(
            arguments,
            &[Type::Var(*variable)],
            "raw helper Rigids must not leak into maker"
        );
    }
    assert_eq!(
        typed.hidden[&make_site],
        if predicate { vec![0] } else { Vec::new() }
    );
    let (receiving, bound) = typed
        .bound
        .iter()
        .find(|(site, _)| {
            source.get(site.0..site.1) == Some("host.R { items: xs }")
        })
        .expect("actual constructor/default receiving call");
    let mut retained = 0;
    for (spot, value) in &bound.defaults {
        let mut instances = Vec::new();
        raw_host_default_expressions(value, &mut instances);
        for instance in instances {
            if instance.origin == Origin::Host("host".into())
                && instance.value == default_root
            {
                retained += 1;
                assert!(!typed.default_instances.contains_key(&(receiving.0, receiving.1, *spot)), "the private default is retained without its temporary lookup");
                assert!(
                    matches!(instance.typed.selected.get(&read_site), Some(fipe::typed::Selected::Value { origin: Origin::Host(name), site }) if name == "host" && (site.start, site.end) == helper_site)
                );
                assert_eq!(
                    instance.typed.meaning_arguments[&read_site],
                    [Type::Var(*variable)]
                );
            }
        }
    }
    assert!(retained > 0, "actual private default Group is retained");
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let tables = unit.tables();
    let helper = tables
        .definitions
        .iter()
        .position(|definition| {
            tables.namespaces[definition.namespace] == "host"
                && definition.label == "helper"
                && (definition.span.start, definition.span.end) == helper_site
        })
        .expect("canonical Host helper definition");
    let maker = tables
        .definitions
        .iter()
        .position(|definition| {
            tables.namespaces[definition.namespace] == "main"
                && definition.label == "make"
                && (definition.span.start, definition.span.end) == make_site
        })
        .expect("actual maker definition");
    raw_host_helper_emission(&program, &tables, host, helper, maker, predicate);
}

fn raw_host_helper_calls(
    program: &mir::Program,
    tables: &ir::Tables,
    helper_body: usize,
    maker_body: usize,
    maker_element: &ir::Type,
    predicate: bool,
) {
    let helper_calls: Vec<_> = program.bodies[maker_body]
        .blocks
        .iter()
        .filter_map(|block| {
            let mir::Terminator::Call {
                callee: mir::Callee::Body(body),
                meanings,
                ..
            } = &block.terminator
            else {
                return None;
            };
            (*body == helper_body).then_some(meanings)
        })
        .collect();
    assert!(!helper_calls.is_empty());
    for meanings in helper_calls {
        assert_eq!(
            *meanings,
            if predicate {
                vec![(*maker_element).clone()]
            } else {
                Vec::new()
            }
        );
    }
    let mut concrete = Vec::new();
    for block in program.bodies.iter().flat_map(|body| &body.blocks) {
        let mir::Terminator::Call {
            callee: mir::Callee::Body(body),
            meanings,
            ..
        } = &block.terminator
        else {
            continue;
        };
        if *body == maker_body {
            if predicate {
                let [ir::Type::Meaning { index, arguments }] =
                    meanings.as_slice()
                else {
                    panic!("actual concrete maker witness")
                };
                assert!(arguments.is_empty());
                assert_eq!(
                    tables.namespaces[tables.meanings[*index].namespace],
                    "prelude"
                );
                concrete.push(tables.meanings[*index].label.as_str());
            } else {
                assert!(meanings.is_empty());
                concrete.push("unused");
            }
        }
    }
    concrete.sort_unstable();
    assert_eq!(
        concrete,
        if predicate {
            vec!["Integer", "String"]
        } else {
            vec!["unused", "unused"]
        }
    );
}

fn raw_host_helper_emission(
    program: &mir::Program,
    tables: &ir::Tables,
    host: &str,
    helper: usize,
    maker: usize,
    predicate: bool,
) {
    let helper_body = program.definitions[helper].unwrap();
    let maker_body = program.definitions[maker].unwrap();
    let helper_element =
        raw_host_vec_element(&program.bodies[helper_body].locals[1].ty, tables);
    let maker_element =
        raw_host_vec_element(&program.bodies[maker_body].locals[1].ty, tables);
    for (definition, body, input) in [
        (helper, helper_body, &helper_element),
        (maker, maker_body, &maker_element),
    ] {
        assert_eq!(tables.definitions[definition].parameters.len(), 1);
        let ir::Type::Parameter(parameter) = input else {
            panic!("actual creator element parameter")
        };
        assert_eq!(tables.definitions[definition].parameters, [*parameter]);
        let hidden = if predicate {
            vec![*parameter]
        } else {
            Vec::new()
        };
        assert_eq!(tables.definitions[definition].hidden, hidden);
        assert_eq!(program.bodies[body].hidden, hidden);
    }
    let test = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "host"
                && host.get(test.span.start..test.span.end)
                    == Some("self.len > 0"))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if predicate {
        assert_eq!(test.len(), 1, "one canonical supplied predicate test");
    }
    let mut actual_tests = 0;
    for statement in program.bodies[helper_body]
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
    {
        let mir::Statement::Enter { entry, .. } = statement else {
            continue;
        };
        for test in &test {
            let arguments = predicate_conformance_entry(
                &tables.entries[*entry].predicate,
                *test,
            );
            if !arguments.is_empty() {
                assert_eq!(
                    arguments.as_slice(),
                    std::slice::from_ref(&helper_element)
                );
                actual_tests += 1;
            }
        }
    }
    assert_eq!(
        actual_tests > 0,
        predicate,
        "helper's own actual Nonempty validation"
    );
    raw_host_helper_calls(
        program,
        tables,
        helper_body,
        maker_body,
        &maker_element,
        predicate,
    );
}

#[test]
fn raw_host_helper_predicates_keep_canonical_binders_and_retained_calls() {
    const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type R<T> { pub items: Vec<T>, checked: Integer = (helper items) }\npub helper: Vec<a> -> Integer = xs => {\n checked: Nonempty<a> = xs\n 0\n}\npub read: R<a> -> Integer = r => r.checked\n";
    const CONTROL_HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type R<T> { pub items: Vec<T>, checked: Integer = (helper items) }\npub helper: Vec<a> -> Integer = xs => 0\npub read: R<a> -> Integer = r => r.checked\n";
    for predicate in [true, false] {
        let host = if predicate { HOST } else { CONTROL_HOST };
        let host_module = parse(lex(host)).expect("raw Host fixture parses");
        let (default_root, helper_site, read_site) =
            raw_host_default_identity(host, &host_module);
        for reverse in [false, true] {
            let uses = if reverse {
                "(host.read (make [\"s\"]), host.read (make [1]))"
            } else {
                "(host.read (make [1]), host.read (make [\"s\"]))"
            };
            let source = format!(
                "use host\nmake = xs => host.R {{ items: xs }}\n{uses}\n"
            );
            raw_host_helper_case(
                host,
                &source,
                predicate,
                default_root,
                helper_site,
                read_site,
            );
        }
    }
}

/// Read the same one-step scheme-to-body substitution used by `lower::slot`.
/// Actual body parameters are not substituted a second time.
fn exposure_bound_parameter<'a>(
    ty: &'a fipe::types::Type,
    scheme: &fipe::types::Scheme,
    body_parameters: &'a [fipe::types::Type],
) -> &'a fipe::types::Type {
    let fipe::types::Type::Var(variable) = ty else {
        return ty;
    };
    let position = scheme
        .variables
        .iter()
        .position(|known| known == variable)
        .expect("contract variable belongs to the exact checked scheme");
    &body_parameters[position]
}

fn exposure_contract_name(
    ty: &fipe::types::Type,
) -> (&fipe::types::TypeName, &[fipe::types::Type]) {
    match ty {
        fipe::types::Type::Named { name, arguments } => (name, arguments),
        fipe::types::Type::Limited { name, .. } => exposure_contract_name(name),
        other => {
            panic!("checked signature must retain its named limit: {other:?}")
        }
    }
}

fn exposure_method_source<'a>(
    module: &'a fipe::ast::Module,
    resolved: &fipe::resolve::Resolved,
) -> (
    fipe::token::Span,
    usize,
    fipe::types::TypeName,
    &'a fipe::ast::Name,
) {
    use fipe::{ast, resolve::Target};
    let methods: Vec<_> = module
        .entries
        .iter()
        .filter_map(|entry| {
            let ast::Entry::Item(ast::Item {
                kind: ast::ItemKind::Impl { members, .. },
                ..
            }) = entry
            else {
                return None;
            };
            Some(members)
        })
        .flatten()
        .filter_map(|member| {
            let ast::ImplMember::Named {
                name,
                annotation: Some(annotation),
                value: Some(value),
                ..
            } = member
            else {
                return None;
            };
            Some((name, *annotation, *value))
        })
        .collect();
    let [(name, annotation, value)] = methods.as_slice() else {
        panic!("one actual annotated method producer");
    };
    let site = name.span;
    let ast::ExprKind::Arms(arms) = &module.expr(*value).unwrap().kind else {
        panic!("actual body-backed callable");
    };
    let arity = arms[0].patterns.len();
    assert_eq!(arity, 1, "written body, not the full result-function spine");
    assert!(arms.iter().all(|arm| arm.patterns.len() == arity));
    let ast::TypeKind::Function { result, .. } =
        &module.ty(*annotation).unwrap().kind
    else {
        panic!("checked method arrow");
    };
    let ast::TypeKind::Function {
        parameter: limit, ..
    } = &module.ty(*result).unwrap().kind
    else {
        panic!("declared returned-function contract");
    };
    let Target::Type {
        origin,
        name: limit_name,
    } = resolved.ty(*limit).unwrap()
    else {
        panic!("the actual annotation names a limit type");
    };
    let expected_name = fipe::types::TypeName {
        origin: origin.clone(),
        name: limit_name.clone(),
    };
    (site, arity, expected_name, *name)
}

fn exposure_predicate_only_contract(
    scheme: &fipe::types::Scheme,
    body_parameters: &[fipe::types::Type],
    raw: &ir::Definition,
    rows: &[&fipe::typed::DispatchRow],
    tables: &ir::Tables,
    definition: usize,
    hidden: &[usize],
) {
    use fipe::types::Type;
    let Type::Function { result, .. } = &scheme.ty else {
        panic!("full checked method scheme");
    };
    let Type::Function { parameter, .. } = &**result else {
        panic!("scheme retains returned function");
    };
    let (_, arguments) = exposure_contract_name(parameter);

    let [argument] = arguments else {
        panic!("actual generic named predicate has one parameter");
    };
    let Type::Var(variable) = argument else {
        panic!("scheme parameter is genuinely generic");
    };
    let position = scheme
        .variables
        .iter()
        .position(|known| known == variable)
        .unwrap();
    assert_eq!(
        exposure_bound_parameter(argument, scheme, body_parameters),
        &body_parameters[position]
    );
    assert!(!hidden.contains(&position), "required nonvacuity: predicate variable is not a raw body runtime read");
    let canonical = raw.parameters[position];
    let Type::Function {
        parameter: owner, ..
    } = &scheme.ty
    else {
        unreachable!();
    };
    let (_, owner_arguments) = exposure_contract_name(owner);
    assert_eq!(
        owner_arguments, arguments,
        "predicate argument is the checked owner binder"
    );
    let ir::Type::Function {
        parameter: owner, ..
    } = &raw.ty
    else {
        panic!("canonical erased raw prototype");
    };
    let ir::Type::Meaning {
        arguments: owner_arguments,
        ..
    } = &**owner
    else {
        panic!("canonical nominal self");
    };
    assert_eq!(
        owner_arguments,
        &[ir::Type::Parameter(canonical)],
        "actual slot conversion maps the same owner binder to its canonical ID"
    );
    assert!(!raw.hidden.contains(&canonical));
    for row in rows {
        let Type::Named {
            arguments: owner_arguments,
            ..
        } = &row.receiver
        else {
            panic!("checked generic nominal owner");
        };
        let [actual] = owner_arguments.as_slice() else {
            panic!("owner carries the predicate argument");
        };
        assert_eq!(
            &row.meanings[position], actual,
            "full checked row map retains the predicate-only actual"
        );
    }
    assert!(tables.dispatch.iter().flat_map(|dispatch| &dispatch.rows).any(|row| {
            matches!(&row.target, ir::Implementation::Definition { index, meanings } if *index == definition && meanings.len() == raw.hidden.len())
        }), "raw target forwards only its original hidden subset");
}

fn exposure_raw_method<'a>(
    tables: &'a ir::Tables,
    program: &mir::Program,
    typed: &'a fipe::typed::Typed,
    site: fipe::token::Span,
    definition: usize,
    arity: usize,
    parameter_count: usize,
) -> (&'a ir::Definition, &'a [usize]) {
    let raw = &tables.definitions[definition];
    assert_eq!(tables.slots[raw.slot].name, Some((raw.namespace, site)));
    assert_eq!(tables.slots[raw.slot].parameters, raw.parameters);
    assert_eq!(raw.parameters.len(), parameter_count);
    let hidden = typed
        .hidden
        .get(&(site.start, site.end))
        .map_or(&[][..], Vec::as_slice);
    assert_eq!(
        raw.hidden,
        hidden
            .iter()
            .map(|position| raw.parameters[*position])
            .collect::<Vec<_>>()
    );
    let body = &program.bodies
        [program.definitions[definition].expect("raw selected method body")];
    assert_eq!(body.arity, arity);
    assert_eq!(body.hidden, raw.hidden);
    assert!(
        program.globals[definition].is_none(),
        "do not invent a value-backed initializer"
    );
    (raw, hidden)
}

fn exposure_method_metadata(source: &str, header: &str, predicate_only: bool) {
    use fipe::{resolve::Origin, typed::Selected, types::Type};
    let mut declarations = Declarations::new();
    declarations.module("host", header).unwrap();
    let module = parse(lex(source)).expect("prototype syntax");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[])
        .expect("prototype resolves");
    let typed = fipe::infer::infer(&module, &resolved, &declarations, &[])
        .unwrap_or_else(|errors| {
            panic!("prototype admission remains evidence: {errors:?}")
        });
    let (site, arity, expected_name, name) =
        exposure_method_source(&module, &resolved);
    let scheme = &typed.schemes[&(site.start, site.end)];
    let body_parameters = typed
        .body_parameters
        .get(&(site.start, site.end))
        .cloned()
        .unwrap_or_else(|| {
            scheme.variables.iter().copied().map(Type::Var).collect()
        });
    assert_eq!(scheme.variables.len(), body_parameters.len());
    let Type::Function { result, .. } = &scheme.ty else {
        panic!("full checked method scheme");
    };
    let Type::Function { parameter, result } = &**result else {
        panic!("scheme retains returned function");
    };
    let (actual_name, _) = exposure_contract_name(parameter);
    assert_eq!(
        actual_name, &expected_name,
        "full scheme retains the exact resolved annotation identity"
    );
    if !predicate_only {
        assert_eq!(
            exposure_contract_name(result).0,
            &expected_name,
            "original signed result limit is retained too"
        );
    }
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    let tables = unit.tables();
    let definitions: Vec<_> = tables
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(index, definition)| {
            (definition.span == site
                && tables.namespaces[definition.namespace] == "main")
                .then_some(index)
        })
        .collect();
    let [definition] = definitions.as_slice() else {
        panic!("exact AST method site owns one raw definition");
    };
    let (raw, hidden) = exposure_raw_method(
        &tables,
        &program,
        &typed,
        site,
        *definition,
        arity,
        body_parameters.len(),
    );
    let rows: Vec<_> = typed.dispatch.iter().flat_map(|dispatch| &dispatch.rows).filter(|row| {
        matches!(&row.selected, Selected::Value { origin: Origin::This, site: actual } if *actual == site)
    }).collect();
    assert!(
        !rows.is_empty(),
        "actual dynamic row owns this exact checked method"
    );
    assert!(rows
        .iter()
        .all(|row| row.meanings.len() == raw.parameters.len()));
    let reads = resolved.expressions().filter_map(|(id, _)| {
        let span = module.expr(id).unwrap().span;
        matches!(typed.selected.get(&(span.start, span.end)), Some(Selected::Member { name: selected, .. }) if Some(selected) == name.segments.last()).then_some(id)
    }).count();
    assert_eq!(
        reads, 1,
        "the generic member read is an actual checked selection"
    );
    if predicate_only {
        exposure_predicate_only_contract(
            scheme,
            &body_parameters,
            raw,
            &rows,
            &tables,
            *definition,
            hidden,
        );
    }
}

#[test]
fn selected_method_full_contract_keeps_original_signed_limits_and_raw_provenance(
) {
    const SOURCE: &str = "use host\ntype M { n: Integer }\nimpl M { op: Self -> U8 -> U8 = self => (x => x) }\nget = m => m.op\nf = get (M 7)\nhost.after ()\nf (host.arg ())";
    exposure_method_metadata(
        SOURCE,
        "pub after: () -> Integer\npub arg: () -> Integer\n",
        false,
    );
}

#[test]
fn selected_method_full_contract_retains_a_genuine_predicate_only_binder() {
    const SOURCE: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\ntype M<T: Ord> { n: Integer }\nimpl M<a> { op: Self -> Sorted<a> -> Integer = self => (xs => host.after ()) }\nget = m => m.op\nx: M<Integer> = M 7\nf = get x\nf (host.items ())";
    exposure_method_metadata(
        SOURCE,
        "pub items: () -> Vec<Integer>\npub after: () -> Integer\n",
        true,
    );
}

#[test]
fn higher_order_result_proof_is_not_replaced_by_a_function_parameter_guard() {
    const BAD: &str = "use host\nneed: (Integer -> U8) -> Integer = f => f 3\nprovide: Integer -> Integer = x => host.arg ()\nneed provide";
    const GOOD: &str = "use host\nf: U8 -> U8 = n => n\nid = x => x\nh2 = g x => g x\nk = n => h2 (id f) n\nk (host.arg ())";
    let mut declarations = Declarations::new();
    declarations
        .module("host", "pub arg: () -> Integer\n")
        .unwrap();
    let bad = parse(lex(BAD)).unwrap();
    fipe::resolve::resolve(&bad, &declarations, &[])
        .expect("negative fixture resolves before function proof");
    let fipe::ast::Entry::Line(last) = bad.entries.last().unwrap() else {
        panic!("actual higher-order call");
    };
    let fipe::ast::ExprKind::Call { arguments, .. } =
        &bad.expr(*last).unwrap().kind
    else {
        panic!("actual higher-order call");
    };
    let fipe::ast::ArgumentKind::Positional(value) = arguments[0].kind else {
        panic!("actual function argument");
    };
    let span = bad.expr(value).unwrap().span;
    let errors =
        check::check(&bad, &declarations, &[], check::Options::default())
            .unwrap_err();
    assert!(errors.iter().any(|error| matches!(error,
        check::Error::Type(fipe::infer::Error { span: actual, reason: fipe::infer::Reason::Limit { message } })
        if *actual == span && message.starts_with("unproved function entry"))), "unproved higher-order result must remain a checker refusal: {errors:?}");
    let good = parse(lex(GOOD)).unwrap();
    let resolved = fipe::resolve::resolve(&good, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&good, &resolved, &declarations, &[]).unwrap();
    let unit =
        check::check(&good, &declarations, &[], check::Options::default())
            .unwrap();
    let tables = unit.tables();
    let opens: Vec<_> = typed
        .checks
        .iter()
        .filter(|check| {
            matches!(check.target, Some(fipe::types::Type::Function { .. }))
        })
        .collect();
    assert!(
        !opens.is_empty(),
        "existing variable-function route plans a real function entry"
    );
    assert!(tables.nodes.iter().any(|node| {
        let ir::Kind::Enter { entry, .. } = node.kind else { return false; };
        let entry = &tables.entries[entry];
        tables.namespaces[entry.namespace] == "main"
            && opens.iter().any(|check| check.place == entry.span)
            && matches!(entry.predicate, ir::Predicate::Function { .. })
    }), "actual positive function entry is lowered, rather than treating Function as true");
}

fn member_contract_has_u8(ty: &fipe::types::Type) -> bool {
    use fipe::{resolve::Origin, types::Type};
    match ty {
        Type::Named { name, arguments } => {
            (name.origin == Origin::Prelude && name.name == "U8")
                || arguments.iter().any(member_contract_has_u8)
        }
        Type::Limited { name, .. } => member_contract_has_u8(name),
        Type::Function { parameter, result } => {
            member_contract_has_u8(parameter) || member_contract_has_u8(result)
        }
        Type::Tuple(parts) | Type::Both(parts) | Type::Either(parts) => {
            parts.iter().any(member_contract_has_u8)
        }
        Type::Array { element, .. } | Type::Not(element) => {
            member_contract_has_u8(element)
        }
        _ => false,
    }
}

fn member_contract_binding_site(
    module: &fipe::ast::Module,
    binding_name: &str,
) -> fipe::token::Span {
    use fipe::ast;
    let sites: Vec<_> = module
        .entries
        .iter()
        .filter_map(|entry| {
            let ast::Entry::Line(id) = entry else {
                return None;
            };
            let ast::ExprKind::Bind { pattern, .. } =
                module.expr(*id).unwrap().kind
            else {
                return None;
            };
            let ast::PatternKind::Name(name) =
                &module.pattern(pattern).unwrap().kind
            else {
                return None;
            };
            (name
                .segments
                .last()
                .is_some_and(|name| name == binding_name))
            .then_some(name.span)
        })
        .collect();
    let [site] = sites.as_slice() else {
        panic!("exact source binding");
    };
    *site
}

#[test]
fn member_callable_contract_metadata_keeps_declared_places_separate_from_meaning_schemes(
) {
    use fipe::{resolve::Origin, types::Type};
    let prefix = "use host\ntype M { n: Integer }\nimpl M { op: Self -> Vec<Integer -> U8> -> Integer = self => (fs => host.after ()) }\n";
    for (binding_name, route, call) in [
        ("get", "get = m => m.op\nf = get (M 7)\n", "f [provide]"),
        (
            "invoke",
            "invoke = m fs => m.op fs\n",
            "invoke (M 7) [provide]",
        ),
    ] {
        let source =
            format!("{prefix}{route}provide: Integer -> U8 = n => 3\n{call}");
        let mut declarations = Declarations::new();
        declarations
            .module("host", "pub after: () -> Integer\n")
            .unwrap();
        let module = parse(lex(&source)).unwrap();
        let resolved =
            fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed =
            fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
        let site = member_contract_binding_site(&module, binding_name);
        let scheme = &typed.schemes[&(site.start, site.end)];
        assert!(
            !member_contract_has_u8(&scheme.ty),
            "generic meaning scheme stays unchanged"
        );
        let reads = &typed.member_contract_reads[&(site.start, site.end)];
        assert_eq!(reads.len(), scheme.obligations.len());
        let contracts: Vec<_> = typed
            .member_contracts
            .iter()
            .filter(|(key, _)| {
                key.origin == Origin::This && key.binding == site
            })
            .collect();
        assert_eq!(
            contracts.len(),
            1,
            "one actual binding instantiation provider"
        );
        let (key, contract) = contracts[0];
        assert!(
            member_contract_has_u8(&contract.callable),
            "authored nested function result remains a declaration place"
        );
        assert_eq!(contract.reads, reads[key.obligation]);
        assert_eq!(contract.reads.len(), 1);
        assert_eq!(
            &source[contract.reads[0].span.start..contract.reads[0].span.end],
            "m.op"
        );
        assert!(
            matches!(&contract.selected, fipe::typed::Selected::Value { origin: Origin::This, site } if &source[site.start..site.end] == "op")
        );
        assert!(contract.meanings.is_empty());
        if binding_name == "invoke" {
            assert!(scheme.obligations.iter().any(|obligation| matches!(
                obligation,
                fipe::types::Obligation::Member {
                    ty: Type::Function { .. },
                    ..
                }
            )));
            assert!(typed.bound.values().any(|bound| bound.spots.len() == 2 && !member_contract_has_u8(&bound.spots[1].1)), "meaning Bound remains separate; exact receiving obligations are checked per use");
        } else {
            assert!(
                typed.callee_types.values().any(member_contract_has_u8),
                "actual f callee place retains its contract"
            );
        }
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        let tables = unit.tables();
        let methods = tables
            .definitions
            .iter()
            .filter(|definition| {
                tables.namespaces[definition.namespace] == "main"
                    && definition.label == "op"
            })
            .count();
        assert_eq!(
            methods, 1,
            "no generated exposure body or runtime schema change"
        );
        let _ = compile::compile(&unit);
    }
}
