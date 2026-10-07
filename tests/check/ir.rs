use fipe::{check, decl::Declarations, ir, lex::lex, parse::parse};

#[test]
fn indexed_literals_and_functions_need_no_source_tree() {
    for source in [
        "1",
        "id = x => x\nid 1",
        "f: Integer -> String = { 0 => \"a\", _ => \"b\" }\nf 1",
        "type Mask: Literal & Eq { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nf: Mask -> String = { 0 => \"a\", _ => \"b\" }",
    ] {
        let unit = {
            let module = parse(lex(source)).unwrap();
            check::check(&module, &Declarations::new(), &[], check::Options::default()).unwrap()
        };
        let tables = unit.tables();
        assert_ne!(unit.main, [] as [usize; 0]);
        for index in &unit.main { assert!(*index < tables.nodes.len()); }
        assert!(tables.nodes.iter().any(|node| matches!(node.kind, ir::Kind::Literal { .. })));
    }
}

#[test]
fn indexed_calls_keep_defaults_and_dispatch_rows() {
    let mut declarations = Declarations::new();
    declarations
        .commands("copy: (value: a) -> (other: a = value) -> a")
        .unwrap();
    for source in [
        "copy 1",
        "copy \"text\"",
        "inc = x => x + 1\ninc 4",
        "same = x => x == x\nsame 4",
        "type Point { x: Integer }\nget = p => p.x\nget (Point 1)",
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &[],
            check::Options::default(),
        )
        .unwrap();
        for node in &unit.tables().nodes {
            if let ir::Kind::Dispatch { member, .. } = node.kind {
                assert!(
                    !unit.tables().dispatch[member].rows.is_empty(),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn indexed_closures_capture_outer_slots() {
    let module = parse(lex("outer = x => (y => (x, y))\nouter 1")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    assert!(unit.tables().nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Function { captures, .. } if captures.len() == 1)));
    assert!(unit
        .tables()
        .nodes
        .iter()
        .any(|node| matches!(node.kind, ir::Kind::Capture { .. })));
}

#[test]
fn indexed_gates_reference_checked_tests() {
    let module = parse(lex("type Positive: Integer where self > 0\ntype Other: Integer & !Positive\nf: Integer -> Integer = { Positive x => x, Other x => x }\nf 1")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    assert_ne!(unit.tables().tests, [] as [fipe::ir::Test; 0]);
}

#[test]
fn indexed_operations_keep_comparisons_and_lifts() {
    for (source, lift) in [
        ("1 < 2", false),
        ("1 != 2", false),
        ("Just x = Just 1\nx + 1", true),
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap();
        assert_eq!(
            unit.tables()
                .nodes
                .iter()
                .any(|node| matches!(node.kind, ir::Kind::Lift { .. })),
            lift
        );
    }
}

#[test]
fn indexed_supplied_limits_and_constants_have_bodies() {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type Box<T: Eq> { v: T } where self.v == self.v\npub get: Box<Integer>\npub answer = 42").unwrap();
    let module = parse(lex("use host\n(host.get, host.answer)")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    assert_ne!(unit.tables().tests, [] as [fipe::ir::Test; 0]);
    assert_ne!(unit.tables().definitions, [] as [fipe::ir::Definition; 0]);
}

#[test]
fn indexed_raw_records_keep_instantiated_defaults() {
    let module = parse(lex(
        "type Point { x: Integer, y: Integer = x }\nPoint { x: 3 }",
    ))
    .unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    assert!(unit.tables().nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Call { defaults, .. } if defaults.len() == 1)));
}

#[test]
fn a_pipe_stage_that_calls_is_typed_as_its_result_after_the_cast() {
    let module = parse(lex("a: U8 = 5\na | hypot 3")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    let pipe = &tables.nodes[*unit.main.last().unwrap()];
    let ir::Kind::Pipe { left, stage } = pipe.kind else {
        panic!("{:?}", pipe.kind)
    };
    assert!(matches!(tables.nodes[left].kind, ir::Kind::Cast { .. }));
    assert_eq!(tables.nodes[left].ty, pipe.ty);
    assert_eq!(tables.nodes[stage].ty, pipe.ty);
    assert!(matches!(
        &tables.nodes[stage].kind,
        ir::Kind::Call { supplies, .. } if supplies.len() == 2
    ));
}

#[test]
fn a_member_of_a_type_member_lowers_to_a_field_of_its_definition() {
    let module = parse(lex(
        "type P { x: Integer }\nimpl P { origin = Self 1 }\nP.origin.x",
    ))
    .unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    let ir::Kind::Field { value, index: 0 } = tables.nodes[unit.main[0]].kind
    else {
        panic!("{:?}", tables.nodes[unit.main[0]].kind)
    };
    assert!(matches!(
        tables.nodes[value].kind,
        ir::Kind::Definition { .. }
    ));
}

#[test]
fn indexed_documentation_examples() {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    for (name, source) in [
        ("fs", include_str!("../fixtures/host/fs.fipe")),
        ("sys", include_str!("../fixtures/host/sys.fipe")),
        ("cli", include_str!("../fixtures/host/cli.fipe")),
        ("text", include_str!("../fixtures/host/text.fipe")),
    ] {
        declarations.module(name, source).unwrap();
    }
    let geometry = parse(lex(include_str!(
        "../../Documentation/examples/geometry.fipe"
    )))
    .unwrap();
    let files = [(String::from("geometry"), geometry)];
    for source in [
        include_str!("../../Documentation/examples/01-files.fipe"),
        include_str!("../../Documentation/examples/02-outcomes.fipe"),
        include_str!("../../Documentation/examples/03-types.fipe"),
        include_str!("../../Documentation/examples/04-text.fipe"),
        include_str!("../../Documentation/examples/05-modules.fipe"),
        include_str!("../../Documentation/examples/geometry.fipe"),
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &declarations,
            &files,
            check::Options::default(),
        )
        .unwrap();
        check_indices(&unit);
    }
}

#[test]
fn indexed_conversions_keep_result_adapters() {
    let declarations = Declarations::new();
    for (source, mode) in [
        ("type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n1 | try<Box>", ir::ConversionMode::Wrap),
        ("type Box: TryFrom<Integer, String> { value: Integer }\nimpl Box: TryFrom<Integer, String> { try_from = n => Ok (Self n) }\n1 | to<Box>", ir::ConversionMode::Unwrap),
        ("1 | try<U8>", ir::ConversionMode::TryCheck),
        ("1 | to<U8>", ir::ConversionMode::Check),
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(&module, &declarations, &[], check::Options::default()).unwrap();
        assert!(unit.tables().nodes.iter().any(|node| matches!(node.kind, ir::Kind::Convert { mode: found, .. } if found == mode)), "{source}");
    }
}

#[test]
fn indexed_local_defaults_keep_each_call_and_lexical_scope() {
    {
        let source = "copy = x (other = x) => other\n(copy 1, copy \"text\")";
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap();
        let tables = unit.tables();
        let mut calls = Vec::new();
        for node in &tables.nodes {
            if let ir::Kind::Call {
                slots, defaults, ..
            } = &node.kind
            {
                for (spot, value) in defaults {
                    if let ir::Kind::Local { slot, .. } =
                        tables.nodes[*value].kind
                    {
                        assert!(slots[..*spot].contains(&slot));
                        calls.push(slot);
                    }
                }
            }
        }
        assert_eq!(calls.len(), 2);
        assert_ne!(calls[0], calls[1]);
    }
}

#[test]
fn defaults_reject_enclosing_function_locals_at_the_name() {
    let source = "outer = x => (copy = (other = x) => other\ncopy)";
    let module = parse(lex(source)).unwrap();
    let errors = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_err();
    assert!(errors.iter().any(|error| matches!(error, check::Error::Resolve(error) if error.reason == fipe::resolve::Reason::DefaultScope && &source[error.span.start..error.span.end] == "x")));
    for source in [
        "value = 7\ncopy = (other = value) => other\ncopy",
        "copy = x (other = x) => other\ncopy 1",
    ] {
        let module = parse(lex(source)).unwrap();
        check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap();
    }
}

#[test]
fn a_default_naming_its_own_binding_is_refused_before_lowering() {
    for source in [
        "f = (n = f) => n\nf",
        "h = (n = 1) (m = h) => n + m\nh",
        "a = (n = 1) (m = (a 2)) => n + m",
    ] {
        let module = parse(lex(source)).unwrap();
        let errors = check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| matches!(
                error,
                check::Error::Resolve(error)
                    if error.reason == fipe::resolve::Reason::SelfDefault
            )),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn no_derived_constructor_outside_over_a_private_field_without_default() {
    let file = parse(lex("pub type Hidden { pub x: I64, secret: U64 }\n\
         pub type Filled { pub x: I64, secret: U64 = 1 }\n\
         pub type Listed { pub x: I64, secret: Vec<U8> }\n\
         pub w = Hidden 1 2"))
    .unwrap();
    let files = [("pq".to_string(), file)];
    let refused = |source: &str| {
        let module = parse(lex(source)).unwrap();
        let errors = check::check(
            &module,
            &Declarations::new(),
            &files,
            check::Options::default(),
        )
        .unwrap_err();
        let [check::Error::Type(error)] = errors.as_slice() else {
            panic!("{source}: {errors:?}")
        };
        assert_eq!(
            error.reason,
            fipe::infer::Reason::NoConstructor {
                field: "secret".into()
            },
            "{source}"
        );
        source[error.span.start..error.span.end].to_string()
    };
    assert_eq!(refused("use pq\npq.Hidden 1"), "pq.Hidden");
    assert_eq!(refused("use pq\nhh = pq.Hidden 1"), "pq.Hidden");
    assert_eq!(refused("use pq\npq.Hidden { x: 1 }"), "pq.Hidden { x: 1 }");
    // Every private field defaults, by declaration or by type: the
    // constructor exists outside, and the module's own value is fine.
    for source in ["use pq\npq.Filled 1", "use pq\npq.Listed 1", "use pq\npq.w"]
    {
        let module = parse(lex(source)).unwrap();
        check::check(
            &module,
            &Declarations::new(),
            &files,
            check::Options::default(),
        )
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    }
}

#[test]
fn indexed_constant_defaults_share_nodes() {
    let mut declarations = Declarations::new();
    declarations
        .commands("copy: (value: Integer = 7) -> Integer")
        .unwrap();
    let module = parse(lex("copy\ncopy")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let tables = unit.tables();
    let defaults: Vec<_> = unit
        .main
        .iter()
        .map(|index| {
            let ir::Kind::Call { defaults, .. } = &tables.nodes[*index].kind
            else {
                panic!("default call")
            };
            defaults[0].1
        })
        .collect();
    assert_eq!(defaults.len(), 2);
    assert_eq!(defaults[0], defaults[1]);
}

#[test]
fn indexed_primitives_keep_declaration_order() {
    let mut declarations = Declarations::new();
    declarations
        .commands("alpha: Integer -> Integer\nbeta: Integer -> Integer")
        .unwrap();
    let module = parse(lex("beta 2\nalpha 1")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let tables = unit.tables();
    for (name, ordinal) in [("alpha", 0), ("beta", 1)] {
        let declaration = tables
            .declarations
            .iter()
            .find(|declaration| declaration.label == name)
            .unwrap();
        assert_eq!(declaration.index, ordinal);
        assert_eq!(tables.namespaces[declaration.namespace], "commands");
    }
}

fn check_indices(unit: &ir::Unit) {
    let tables = unit.tables();
    for (index, entry) in tables.entries.iter().enumerate() {
        assert!(tables.nodes.iter().any(|node| matches!(node.kind, ir::Kind::Enter { entry, .. } if entry == index)), "unattached entry: {entry:?}");
    }
    for index in &unit.main {
        assert!(*index < tables.nodes.len());
    }
    for definition in &tables.definitions {
        assert!(definition.body < tables.nodes.len());
        assert!(definition.slot < tables.slots.len());
        assert!(definition.namespace < tables.namespaces.len());
        assert!(definition
            .hidden
            .iter()
            .all(|p| definition.parameters.contains(p)));
    }
    check_tables(&tables);
    for node in &tables.nodes {
        if let ir::Kind::Definition { meanings, .. }
        | ir::Kind::Local { meanings, .. }
        | ir::Kind::Capture { meanings, .. } = &node.kind
        {
            assert!(meanings
                .iter()
                .all(|ty| !uses_parameters(ty, &unit.parameters)));
        }
        check_edges(&tables, &node.kind);
        check_type(&tables, &node.ty);
        assert!(node.namespace < tables.namespaces.len());
        match &node.kind {
            ir::Kind::Call {
                callee,
                slots,
                supplies,
                defaults,
                open,
            } => check_call(
                &tables, node, *callee, slots, supplies, defaults, open,
            ),
            ir::Kind::Local { slot, meanings }
            | ir::Kind::Capture { slot, meanings } => {
                assert!(*slot < tables.slots.len());
                assert_eq!(meanings.len(), tables.slots[*slot].hidden.len());
            }
            ir::Kind::Definition { index, meanings } => {
                assert!(*index < tables.definitions.len());
                assert_eq!(
                    meanings.len(),
                    tables.definitions[*index].hidden.len()
                );
            }
            ir::Kind::Declared(index) => {
                assert!(*index < tables.declarations.len());
            }
            ir::Kind::Construct {
                meaning,
                constructor,
                fields,
            } => {
                let layout =
                    &tables.meanings[*meaning].constructors[*constructor];
                assert_eq!(
                    fields.len(),
                    layout.fields.len(),
                    "{}",
                    layout.label
                );
                for field in fields {
                    assert!(*field < tables.nodes.len());
                }
            }
            ir::Kind::Function {
                parameters,
                hidden,
                captures,
                arms,
            } => {
                assert!(hidden.iter().all(|p| parameters.contains(p)));
                for slot in captures {
                    assert!(*slot < tables.slots.len());
                }
                for arm in arms {
                    assert!(arm.body < tables.nodes.len());
                    assert!(arm
                        .guard
                        .is_none_or(|guard| guard < tables.nodes.len()));
                }
            }
            _ => (),
        }
    }
}

#[test]
fn indexed_empty_outcomes_keep_unit_payloads() {
    for source in [
        "value: Maybe<()> = [:]",
        "value: Maybe<()> = [!]",
        "value: Result<(), String> = [:]",
    ] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap();
        check_indices(&unit);
    }
}

#[test]
fn indexed_modules_initialize_dependencies_before_first_use() {
    let files: Vec<_> = [
        (
            "z",
            "use b
pub z = b.value",
        ),
        ("b", "pub value = 2"),
        ("a", "pub value = 1"),
    ]
    .into_iter()
    .map(|(name, text)| (name.into(), parse(lex(text)).unwrap()))
    .collect();
    let module = parse(lex("use z a b
z.z"))
    .unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .unwrap();
    assert_eq!(
        unit.tables()
            .modules
            .iter()
            .map(|module| module.label.as_str())
            .collect::<Vec<_>>(),
        ["b", "z", "a"]
    );
    check_indices(&unit);
}

fn check_call(
    tables: &ir::Tables,
    node: &ir::Node,
    callee: usize,
    slots: &[usize],
    supplies: &[ir::Supply],
    defaults: &[(usize, usize)],
    open: &[usize],
) {
    assert!(callee < tables.nodes.len());
    for slot in slots {
        assert!(*slot < tables.slots.len());
    }
    let mut filled = vec![false; slots.len()];
    for supply in supplies {
        assert!(supply.value < tables.nodes.len());
        assert!(supply.spot < slots.len());
        assert!(!filled[supply.spot] || supply.element);
        filled[supply.spot] = true;
    }
    for (spot, value) in defaults {
        assert!(*spot < slots.len());
        assert!(*value < tables.nodes.len());
        assert!(!filled[*spot]);
        filled[*spot] = true;
    }
    for spot in open {
        assert!(*spot < slots.len());
        assert!(!filled[*spot]);
        filled[*spot] = true;
    }
    assert!(filled.iter().all(|filled| *filled));
    let mut ty = &tables.nodes[callee].ty;
    let mut arity = 0;
    while let ir::Type::Function { result, .. } = ty {
        arity += 1;
        ty = result;
    }
    assert!(
        arity >= slots.len(),
        "call {:?}: {} spots, callee {:?} has {arity}",
        node.span,
        slots.len(),
        tables.nodes[callee]
    );
}

#[test]
fn indexed_field_defaults_select_written_default_members() {
    let module = parse(lex("type D: Default { value: Integer }\nimpl D: Default { default = Self 7 }\ntype Box { value: D }\nBox {}" )).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    let defaults: Vec<_> = tables
        .nodes
        .iter()
        .filter_map(|node| {
            let ir::Kind::Call { defaults, .. } = &node.kind else {
                return None;
            };
            defaults
                .first()
                .map(|(_, index)| &tables.nodes[*index].kind)
        })
        .collect();
    assert!(defaults.iter().any(|kind| matches!(kind, ir::Kind::Definition { index, .. } if tables.definitions[*index].label == "default")));
}

#[test]
fn indexed_nested_defaults_keep_field_dependencies() {
    let source = "type Inner { x: Integer = 7, y: Integer = x }\ntype Outer { inner: Inner, copies: [Inner; 2] }\nOuter {}";
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    check_indices(&unit);
    let tables = unit.tables();
    assert!(tables
        .nodes
        .iter()
        .any(|node| matches!(node.kind, ir::Kind::Repeat { .. })));
    let mut records = 0;
    for node in &tables.nodes {
        let ir::Kind::Call {
            slots,
            defaults,
            callee,
            ..
        } = &node.kind
        else {
            continue;
        };
        let ir::Kind::Constructor { meaning, .. } = tables.nodes[*callee].kind
        else {
            continue;
        };
        if tables.meanings[meaning].label != "Inner" {
            continue;
        }
        records += 1;
        assert_eq!(defaults.len(), 2);
        let value = defaults[1].1;
        assert!(
            matches!(tables.nodes[value].kind, ir::Kind::Local { slot, .. } if slot == slots[0])
        );
    }
    assert_eq!(records, 2);
}

#[test]
fn indexed_private_constructor_fields_keep_defaults() {
    let file = parse(lex(
        "pub type Record { secret: Integer = 7, pub value: Integer }",
    ))
    .unwrap();
    let module = parse(lex("use data\ndata.Record 3")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[("data".into(), file)],
        check::Options::default(),
    )
    .unwrap();
    check_indices(&unit);
    let tables = unit.tables();
    let node = &tables.nodes[*unit.main.last().unwrap()];
    let ir::Kind::Call {
        slots,
        defaults,
        callee,
        ..
    } = &node.kind
    else {
        panic!("constructor call")
    };
    assert_eq!(slots.len(), 2);
    assert_eq!(defaults.len(), 1);
    let ir::Kind::Constructor { fields, .. } = &tables.nodes[*callee].kind
    else {
        panic!("raw constructor")
    };
    assert_eq!(fields, &[1, 0]);
}

#[test]
fn indexed_derived_defaults_construct_written_field_values() {
    let module =
        parse(lex("type D: Default { value: Integer = 7 }\nD.default"))
            .unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    check_indices(&unit);
    let tables = unit.tables();
    assert!(tables.nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Call { callee, defaults, .. } if defaults.len() == 1 && matches!(tables.nodes[*callee].kind, ir::Kind::Constructor { .. }))));
}

#[test]
fn indexed_nested_entries_keep_projection_paths() {
    let mut declarations = Declarations::new();
    declarations
        .commands("input: Vec<Integer>\nnumber: Integer")
        .unwrap();
    let module = parse(lex(
        "values: Vec<U8> = input\npair: U8 * U8 = (number, number)",
    ))
    .unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    assert!(unit
        .plan
        .iter()
        .any(|entry| entry.path == [ir::Projection::Element]));
    let tables = unit.tables();
    for node in &tables.nodes {
        if let ir::Kind::Enter { entry, value } = node.kind {
            assert!(entry < tables.entries.len());
            assert!(value < tables.nodes.len());
        }
    }
}

#[test]
fn indexed_text_casts_keep_selected_implementations() {
    let source = "type D: Read & Show { value: Integer }\nimpl D: Read { read = s => Ok (Self 7) }\nimpl D: Show { show = self => \"d\" }\nvalue: D = \"7\"\ntext: String = value";
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    let tables = unit.tables();
    for row in [ir::CastRow::Read, ir::CastRow::Show] {
        assert!(tables.nodes.iter().any(|node| matches!(&node.kind, ir::Kind::Cast { row: actual, operation: Some(operation), .. } if *actual == row && matches!(tables.nodes[*operation].kind, ir::Kind::Definition { .. }))));
    }
}

fn check_type(tables: &ir::Tables, ty: &ir::Type) {
    match ty {
        ir::Type::Meaning { index, arguments } => {
            assert!(*index < tables.meanings.len());
            for ty in arguments {
                check_type(tables, ty);
            }
        }
        ir::Type::Session(index) => {
            assert!(*index < tables.session_meanings.len());
        }
        ir::Type::Function { parameter, result }
        | ir::Type::Array {
            element: parameter,
            length: result,
        } => {
            check_type(tables, parameter);
            check_type(tables, result);
        }
        ir::Type::Tuple(parts)
        | ir::Type::Packed(parts)
        | ir::Type::All(parts)
        | ir::Type::Sum(parts) => {
            for ty in parts {
                check_type(tables, ty);
            }
        }
        ir::Type::Not(ty) => check_type(tables, ty),
        ir::Type::Unit | ir::Type::Parameter(_) | ir::Type::Number(_) => (),
    }
}

fn check_tables(tables: &ir::Tables) {
    for meaning in &tables.meanings {
        assert!(meaning.namespace < tables.namespaces.len());
        for field in meaning
            .constructors
            .iter()
            .flat_map(|constructor| &constructor.fields)
        {
            check_type(tables, &field.ty);
        }
    }
    for slot in &tables.slots {
        check_type(tables, &slot.ty);
        assert!(slot
            .definition
            .is_none_or(|index| index < tables.definitions.len()));
    }
    for declaration in &tables.declarations {
        assert!(declaration.namespace < tables.namespaces.len());
        check_type(tables, &declaration.ty);
    }
    for module in &tables.modules {
        for node in &module.lines {
            assert!(*node < tables.nodes.len());
        }
    }
    for test in &tables.tests {
        assert!(test.namespace < tables.namespaces.len());
        assert!(test.body < tables.nodes.len());
        assert!(test.self_slot < tables.slots.len());
    }
    for entry in &tables.entries {
        assert!(entry.namespace < tables.namespaces.len());
        check_predicate(tables, &entry.predicate);
    }
    for row in tables.dispatch.iter().flat_map(|dispatch| &dispatch.rows) {
        check_type(tables, &row.receiver);
        if let Some(argument) = &row.argument {
            check_type(tables, argument);
        }
        match &row.target {
            ir::Implementation::Field(index) => {
                let ir::Type::Meaning { index: meaning, .. } = &row.receiver
                else {
                    panic!("field receiver")
                };
                assert!(
                    *index
                        < tables.meanings[*meaning].constructors[0]
                            .fields
                            .len()
                );
            }
            ir::Implementation::Definition { index, meanings } => {
                assert!(*index < tables.definitions.len());
                assert_eq!(
                    meanings.len(),
                    tables.definitions[*index].hidden.len()
                );
                for ty in meanings {
                    check_type(tables, ty);
                }
            }
            ir::Implementation::Declared(index) => {
                assert!(*index < tables.declarations.len());
            }
            ir::Implementation::Derived { ty, .. } => check_type(tables, ty),
        }
    }
    for ty in tables.session_meanings.iter().flatten() {
        check_type(tables, ty);
    }
}

fn check_predicate(tables: &ir::Tables, predicate: &ir::Predicate) {
    match predicate {
        ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
            for part in parts {
                check_predicate(tables, part);
            }
        }
        ir::Predicate::Not(part) => check_predicate(tables, part),
        ir::Predicate::Test { index, meanings } => {
            assert!(*index < tables.tests.len());
            assert_eq!(meanings.len(), tables.tests[*index].parameters.len());
            for ty in meanings {
                check_type(tables, ty);
            }
        }
        ir::Predicate::Function { parameter, result } => {
            check_predicate(tables, parameter);
            check_predicate(tables, result);
        }
        ir::Predicate::Nonzero { .. } => (),
    }
}

fn check_pattern(tables: &ir::Tables, pattern: &ir::Pattern) {
    match &pattern.kind {
        ir::PatternKind::Any => (),
        ir::PatternKind::Slot(slot) => assert!(*slot < tables.slots.len()),
        ir::PatternKind::Literal {
            value,
            equality,
            self_spot,
        } => {
            assert!(*value < tables.nodes.len());
            assert!(*equality < tables.nodes.len());
            assert!(*self_spot < 2);
        }
        ir::PatternKind::Constructor {
            meaning,
            constructor,
            parts,
        } => {
            assert!(
                *constructor < tables.meanings[*meaning].constructors.len()
            );
            for part in parts {
                check_pattern(tables, part);
            }
        }
        ir::PatternKind::Record {
            meaning,
            constructor,
            fields,
        } => {
            let layout = &tables.meanings[*meaning].constructors[*constructor];
            for (index, part) in fields {
                assert!(*index < layout.fields.len());
                check_pattern(tables, part);
            }
        }
        ir::PatternKind::Side { payload, .. } => {
            if let Some(payload) = payload {
                check_pattern(tables, payload);
            }
        }
        ir::PatternKind::Gate { predicate, payload } => {
            check_predicate(tables, predicate);
            check_pattern(tables, payload);
        }
        ir::PatternKind::Tag { meaning, payload } => {
            assert!(*meaning < tables.meanings.len());
            check_pattern(tables, payload);
        }
        ir::PatternKind::Tuple(parts) | ir::PatternKind::List(parts) => {
            for part in parts {
                check_pattern(tables, part);
            }
        }
    }
}

fn check_value_edges(tables: &ir::Tables, kind: &ir::Kind) {
    let node = |index: usize| assert!(index < tables.nodes.len());
    match kind {
        ir::Kind::Timing { value, .. }
        | ir::Kind::Ordering { value, .. }
        | ir::Kind::Field { value, .. } => node(*value),
        ir::Kind::Literal { reader, .. } => {
            if let Some(reader) = reader {
                node(*reader);
            }
        }
        ir::Kind::Local { meanings, .. }
        | ir::Kind::Capture { meanings, .. }
        | ir::Kind::Definition { meanings, .. } => {
            for ty in meanings {
                check_type(tables, ty);
            }
        }
        ir::Kind::Declared(_)
        | ir::Kind::Call { .. }
        | ir::Kind::Construct { .. }
        | ir::Kind::Convert { .. }
        | ir::Kind::Cast { .. }
        | ir::Kind::Function { .. } => (),
        ir::Kind::Derived { ty, .. } => check_type(tables, ty),
        ir::Kind::Constructor {
            meaning,
            constructor,
            fields,
        } => {
            let layout = &tables.meanings[*meaning].constructors[*constructor];
            for field in fields {
                assert!(*field < layout.fields.len());
            }
        }
        ir::Kind::Tuple(parts)
        | ir::Kind::Array(parts)
        | ir::Kind::Block(parts) => {
            for part in parts {
                node(*part);
            }
        }
        ir::Kind::Repeat { value, length } => {
            node(*value);
            check_type(tables, length);
        }
        ir::Kind::Dispatch { receiver, member } => {
            assert!(*member < tables.dispatch.len());
            match receiver {
                ir::Receiver::Value(value) => node(*value),
                ir::Receiver::Meaning(ty) => check_type(tables, ty),
            }
        }
        ir::Kind::If {
            condition,
            then_body,
            else_body,
        } => {
            node(*condition);
            node(*then_body);
            node(*else_body);
        }
        ir::Kind::Bind { pattern, value } => {
            node(*value);
            check_pattern(tables, pattern);
        }
        ir::Kind::With { key, value, body } => {
            assert!(*key < tables.keys.len());
            node(*value);
            node(*body);
        }
        ir::Kind::Lift {
            pattern,
            value,
            body,
            ..
        } => {
            check_pattern(tables, pattern);
            node(*value);
            node(*body);
        }
        ir::Kind::Pipe { left, stage }
        | ir::Kind::Marked { left, stage, .. } => {
            node(*left);
            node(*stage);
        }
        ir::Kind::Or { left, right, .. }
        | ir::Kind::And { left, right, .. } => {
            node(*left);
            node(*right);
        }
        ir::Kind::Pack { tag, value } => {
            check_type(tables, tag);
            node(*value);
        }
        ir::Kind::Enter { entry, value } => {
            assert!(*entry < tables.entries.len());
            node(*value);
        }
    }
}

fn check_edges(tables: &ir::Tables, kind: &ir::Kind) {
    let node = |index: usize| assert!(index < tables.nodes.len());
    match kind {
        ir::Kind::Convert {
            callee,
            from,
            to,
            parameter,
            target,
            ..
        } => {
            if let Some(callee) = callee {
                node(*callee);
            }
            check_type(tables, from);
            check_type(tables, to);
            check_predicate(tables, parameter);
            check_predicate(tables, target);
        }
        ir::Kind::Cast {
            operation,
            from,
            to,
            value,
            ..
        } => {
            node(*value);
            if let Some(operation) = operation {
                node(*operation);
            }
            check_type(tables, from);
            check_type(tables, to);
        }
        ir::Kind::Function { arms, .. } => {
            for pattern in arms.iter().flat_map(|arm| &arm.patterns) {
                check_pattern(tables, pattern);
            }
        }
        _ => check_value_edges(tables, kind),
    }
}

#[test]
fn indexed_host_constants_supply_member_receivers() {
    let mut declarations = Declarations::new();
    declarations
        .commands("type Point { pub x: Integer }\npoint: Point")
        .unwrap();
    let module = parse(lex("point.x")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    check_indices(&unit);
    let tables = unit.tables();
    assert!(tables.nodes.iter().any(|node| matches!(node.kind, ir::Kind::Field { value, .. } if matches!(tables.nodes[value].kind, ir::Kind::Declared(_)))));
}

#[test]
fn indexed_sum_entries_visit_only_the_matching_constructor() {
    let module = parse(lex("type Choice<A> { Pick(value: A), Empty }\nf: Choice<Integer> -> Choice<U8> = x => x")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    check_indices(&unit);
    assert!(unit.plan.iter().any(|entry| entry.path
        == [ir::Projection::CaseField {
            constructor: 0,
            field: 0
        }]));
}

#[test]
fn indexed_nested_lifts_keep_the_remainder_type() {
    let module = parse(lex("Just x = Just 1\nJust y = Just 2\nx + y")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    check_indices(&unit);
    let tables = unit.tables();
    for node in &tables.nodes {
        if let ir::Kind::Lift { body, .. } = node.kind {
            let ir::Kind::Block(parts) = &tables.nodes[body].kind else {
                panic!("lift remainder")
            };
            assert_eq!(
                tables.nodes[body].ty,
                tables.nodes[*parts.last().unwrap()].ty
            );
        }
    }
}

#[test]
fn indexed_bare_argument_words_keep_text_values() {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    let module = parse(lex("find x |: cat")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    check_indices(&unit);
}

fn uses_parameters(ty: &ir::Type, parameters: &[usize]) -> bool {
    match ty {
        ir::Type::Parameter(index) => parameters.contains(index),
        ir::Type::Meaning { arguments, .. }
        | ir::Type::Tuple(arguments)
        | ir::Type::Packed(arguments)
        | ir::Type::All(arguments)
        | ir::Type::Sum(arguments) => {
            arguments.iter().any(|ty| uses_parameters(ty, parameters))
        }
        ir::Type::Function { parameter, result }
        | ir::Type::Array {
            element: parameter,
            length: result,
        } => {
            uses_parameters(parameter, parameters)
                || uses_parameters(result, parameters)
        }
        ir::Type::Not(inner) => uses_parameters(inner, parameters),
        ir::Type::Unit | ir::Type::Session(_) | ir::Type::Number(_) => false,
    }
}

#[test]
fn indexed_main_free_meanings_are_not_hidden_arguments() {
    for source in ["[]", "Nothing", "([], Nothing)"] {
        let module = parse(lex(source)).unwrap();
        let unit = check::check(
            &module,
            &Declarations::new(),
            &[],
            check::Options::default(),
        )
        .unwrap();
        assert_ne!(unit.parameters, [] as [usize; 0]);
        check_indices(&unit);
    }
}
