use fipe::{
    check, compile,
    decl::Declarations,
    ir,
    lex::lex,
    mir::{Body, Callee, Operand, Program, Rvalue, Statement, Terminator},
    parse::parse,
};

fn lower(source: &str) -> Program {
    super::program(source).1
}

fn main_body(program: &Program) -> &Body {
    &program.bodies[program.main]
}

/// The body of the definition with this label.
fn definition<'a>(program: &'a Program, label: &str) -> &'a Body {
    let index = program
        .tables
        .borrow()
        .definitions
        .iter()
        .position(|definition| definition.label == label)
        .unwrap();
    &program.bodies[program.definitions[index].unwrap()]
}

/// The first switch of a body: its cases and its otherwise.
fn switch(body: &Body) -> (Vec<(usize, usize)>, Option<usize>) {
    body.blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Terminator::Switch {
                cases, otherwise, ..
            } => Some((cases.clone(), *otherwise)),
            _ => None,
        })
        .unwrap()
}

fn rvalues(body: &Body, block: usize) -> Vec<&Rvalue> {
    body.blocks[block]
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Assign(_, rvalue) => Some(rvalue),
            _ => None,
        })
        .collect()
}

fn all_rvalues(body: &Body) -> Vec<&Rvalue> {
    (0..body.blocks.len())
        .flat_map(|block| rvalues(body, block))
        .collect()
}

/// The declaration label of a native or host callee.
fn declared(program: &Program, callee: &Callee) -> Option<String> {
    match callee {
        Callee::Native(index) | Callee::Host(index) => {
            Some(program.tables.borrow().declarations[*index].label.clone())
        }
        _ => None,
    }
}

fn calls(body: &Body) -> Vec<(&Callee, &[Operand], usize)> {
    body.blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Terminator::Call {
                callee, args, dest, ..
            } => Some((callee, args.as_slice(), *dest)),
            _ => None,
        })
        .collect()
}

/// A predicate every value passes: `All` of trivial parts.
fn trivial(predicate: &ir::Predicate) -> bool {
    match predicate {
        ir::Predicate::All(parts) => parts.iter().all(trivial),
        _ => false,
    }
}

fn meaning_label(program: &Program, meaning: usize) -> String {
    program.tables.borrow().meanings[meaning].label.clone()
}

#[test]
fn a_marked_pipe_switches_on_the_side_and_rebuilds_as_recorded() {
    let program = lower("Just 1 |: (_ => Just 2)");
    let main = main_body(&program);
    let (cases, otherwise) = switch(main);
    assert_eq!(cases.iter().map(|(c, _)| *c).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(otherwise, None);
    let (good, bad) = (cases[0].1, cases[1].1);
    assert!(matches!(rvalues(main, bad)[..], [Rvalue::Use(_)]));
    assert!(
        matches!(main.blocks[good].terminator, Terminator::Call { callee: Callee::Value(_), ref args, .. } if args.len() == 1)
    );
    assert!(!all_rvalues(main)
        .iter()
        .any(|r| matches!(r, Rvalue::Wrap { .. })));

    let program = lower("Just [1, 2] |: map (n => n + 1)");
    let main = main_body(&program);
    let payload = all_rvalues(main)
        .iter()
        .any(|r| matches!(r, Rvalue::Field(_, 0)));
    assert!(payload, "the payload is read");
    let (callee, args, _) = calls(main)
        .into_iter()
        .find(|(callee, ..)| {
            declared(&program, callee).as_deref() == Some("map")
        })
        .unwrap();
    assert!(matches!(callee, Callee::Native(_)));
    assert!(args.iter().all(|arg| matches!(arg, Operand::Local(_))));
    assert!(all_rvalues(main)
        .iter()
        .any(|r| matches!(r, Rvalue::Wrap { constructor: 0, .. })));

    let program = lower("Nothing |! 1");
    let main = main_body(&program);
    let (cases, _) = switch(main);
    assert_eq!(cases[0].0, 1, "the bad side is marked");
    assert!(all_rvalues(main).iter().any(|r| matches!(
        r,
        Rvalue::Construct { constructor: 1, fields, .. } if fields.is_empty()
    )));

    let program = lower("Ok 1 |! (e => 3)");
    let main = main_body(&program);
    assert!(all_rvalues(main)
        .iter()
        .any(|r| matches!(r, Rvalue::Wrap { constructor: 1, .. })));
}

#[test]
fn fallback_keeps_the_left_or_runs_the_right_on_its_own_edge() {
    let program = lower("Just 1 || Nothing");
    let main = main_body(&program);
    let (cases, otherwise) = switch(main);
    assert_eq!(otherwise, None);
    let (kept, taken) = (cases[0].1, cases[1].1);
    assert_eq!((cases[0].0, cases[1].0), (0, 1));
    assert!(matches!(rvalues(main, kept)[..], [Rvalue::Use(_)]));
    let nothing = |block: usize| {
        rvalues(main, block)
            .iter()
            .any(|r| matches!(r, Rvalue::Construct { constructor: 1, .. }))
    };
    assert!(nothing(taken), "the right runs on its edge");
    assert!(!nothing(0) && !nothing(kept), "and nowhere else");
    let charge = |block: usize| match main.blocks[block].statements.first() {
        Some(Statement::Charge(count)) => *count,
        _ => 0,
    };
    // `Just 1`: the literal, the call and its constructor; `||` on each
    // edge; `Nothing`: the call and its constructor.
    assert_eq!((charge(0), charge(kept), charge(taken)), (3, 1, 3));

    let program = lower("Just 1 || 2");
    let main = main_body(&program);
    let (cases, _) = switch(main);
    assert!(matches!(
        rvalues(main, cases[0].1)[..],
        [Rvalue::Field(_, 0)]
    ));

    let program = lower("(1 == 1) || (2 == 2)");
    let main = main_body(&program);
    let (cases, _) = switch(main);
    let labels: Vec<_> = {
        let tables = program.tables.borrow();
        let bool = tables
            .meanings
            .iter()
            .find(|meaning| meaning.label == "Bool")
            .unwrap();
        cases
            .iter()
            .map(|(c, _)| bool.constructors[*c].label.clone())
            .collect()
    };
    assert_eq!(labels, ["True", "False"]);
    assert!(matches!(rvalues(main, cases[0].1)[..], [Rvalue::Use(_)]));
    assert!(matches!(
        &main.blocks[cases[1].1].terminator,
        Terminator::Call { callee, .. } if declared(&program, callee).as_deref() == Some("eq")
    ));
}

#[test]
fn sequencing_runs_the_right_on_the_good_side() {
    let program = lower("Just 1 && Just 2");
    let main = main_body(&program);
    let (cases, _) = switch(main);
    assert_eq!((cases[0].0, cases[1].0), (0, 1));
    let (taken, kept) = (cases[0].1, cases[1].1);
    assert!(matches!(rvalues(main, kept)[..], [Rvalue::Use(_)]));
    assert!(rvalues(main, taken)
        .iter()
        .any(|r| matches!(r, Rvalue::Construct { constructor: 0, .. })));
    assert!(!all_rvalues(main)
        .iter()
        .any(|r| matches!(r, Rvalue::Wrap { .. })));

    let program = lower("Just 1 && 2");
    let main = main_body(&program);
    assert!(all_rvalues(main).iter().any(|r| matches!(
        r,
        Rvalue::Wrap {
            constructor: 0,
            value: Operand::Const(_),
            ..
        }
    )));

    let program = lower("(1 == 1) && (2 == 2)");
    let main = main_body(&program);
    let (cases, _) = switch(main);
    assert!(matches!(
        &main.blocks[cases[0].1].terminator,
        Terminator::Call { callee, .. } if declared(&program, callee).as_deref() == Some("eq")
    ));
    assert!(matches!(rvalues(main, cases[1].1)[..], [Rvalue::Use(_)]));
}

#[test]
fn a_lift_binds_the_payload_and_passes_a_miss_through() {
    for (source, constructor, wrapped) in [
        ("{\n    [:v] = Just 1\n    v\n}", 0, "Maybe"),
        ("{\n    Just x = Just 1\n    x + 1\n}", 0, "Maybe"),
        ("{\n    [:v] = Ok 1\n    Just v\n}", 0, "Result"),
    ] {
        let program = lower(source);
        let main = main_body(&program);
        let (cases, otherwise) = switch(main);
        assert_eq!(cases.len(), 1, "{source}");
        assert_eq!(cases[0].0, constructor, "{source}");
        let miss = otherwise.expect("a miss edge");
        assert!(
            matches!(rvalues(main, miss)[..], [Rvalue::Use(_)]),
            "{source}"
        );
        let hit = cases[0].1;
        assert!(
            matches!(rvalues(main, hit)[0], Rvalue::Field(_, 0)),
            "{source}: the payload is bound first"
        );
        let wrap = all_rvalues(main).into_iter().find_map(|r| match r {
            Rvalue::Wrap { meaning, .. } => {
                Some(meaning_label(&program, *meaning))
            }
            _ => None,
        });
        assert_eq!(wrap.as_deref(), Some(wrapped), "{source}");
    }
}

#[test]
fn conversions_adapt_after_their_member() {
    use ir::ConversionMode as Mode;
    let from = "type Box: From<Integer> { value: Integer }\nimpl Box: From<Integer> { from = n => Self n }\n";
    let try_from = "type Box: TryFrom<Integer, String> { value: Integer }\nimpl Box: TryFrom<Integer, String> { try_from = n => Ok (Self n) }\n";
    for (source, mode, called) in [
        ("1 | to<U8>".to_string(), Mode::Check, false),
        ("1 | try<U8>".to_string(), Mode::TryCheck, false),
        (format!("{from}1 | to<Box>"), Mode::Direct, true),
        (format!("{from}1 | try<Box>"), Mode::Wrap, true),
        (format!("{try_from}1 | to<Box>"), Mode::Unwrap, true),
    ] {
        let program = lower(&source);
        let main = main_body(&program);
        let converts: Vec<_> = all_rvalues(main)
            .into_iter()
            .filter_map(|r| match r {
                Rvalue::Convert {
                    mode, predicate, ..
                } => Some((*mode, predicate.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(converts.len(), 1, "{source}");
        assert_eq!(converts[0].0, mode, "{source}");
        let tested = !trivial(&converts[0].1);
        assert_eq!(tested, !called, "{source}: a check tests the target");
        assert_eq!(
            calls(main)
                .iter()
                .any(|(callee, ..)| matches!(callee, Callee::Body(_))),
            called,
            "{source}"
        );
    }
}

#[test]
fn a_read_is_the_declared_reader() {
    let program = lower("\"3\" | read<I64>");
    let main = main_body(&program);
    let labels: Vec<_> = calls(main)
        .iter()
        .filter_map(|(callee, ..)| declared(&program, callee))
        .collect();
    assert_eq!(labels, ["read"]);
    assert!(all_rvalues(main)
        .iter()
        .all(|r| !matches!(r, Rvalue::Cast { .. })));
}

#[test]
fn text_casts_call_the_selected_operation_then_cast_by_the_row() {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    let module = parse(lex("\"a\" | put ./f")).unwrap();
    let unit =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap();
    let program = compile::compile(&unit);
    super::walk::check_program(&program);
    let main = main_body(&program);
    let rows: Vec<_> = all_rvalues(main)
        .into_iter()
        .filter_map(|r| match r {
            Rvalue::Cast { row, .. } => Some(*row),
            _ => None,
        })
        .collect();
    assert!(rows.contains(&ir::CastRow::Read), "{rows:?}");
    assert!(rows.contains(&ir::CastRow::Bytes), "{rows:?}");
    let hosts: Vec<_> = calls(main)
        .iter()
        .filter(|(callee, ..)| matches!(callee, Callee::Host(_)))
        .filter_map(|(callee, ..)| declared(&program, callee))
        .collect();
    assert_eq!(hosts, ["read", "put"]);
}

#[test]
fn an_ordering_switches_the_comparison_to_bool() {
    {
        let source = "1 < 2";
        let program = lower(source);
        let main = main_body(&program);
        let (cases, otherwise) = switch(main);
        assert!(otherwise.is_some(), "{source}");
        let accepted = cases[0].1;
        assert!(
            cases.iter().all(|(_, block)| *block == accepted),
            "{source}"
        );
        let bools: Vec<_> = all_rvalues(main)
            .into_iter()
            .filter_map(|r| match r {
                Rvalue::Construct {
                    meaning,
                    constructor,
                    fields,
                } if fields.is_empty() => {
                    Some((meaning_label(&program, *meaning), *constructor))
                }
                _ => None,
            })
            .collect();
        assert_eq!(bools.len(), 2, "{source}: {bools:?}");
        assert!(bools.iter().all(|(label, _)| label == "Bool"), "{source}");
        assert_ne!(bools[0].1, bools[1].1, "{source}");
    }
    let program = lower("1 < 2");
    let main = main_body(&program);
    assert!(calls(main)
        .iter()
        .any(
            |(callee, ..)| declared(&program, callee).as_deref() == Some("cmp")
        ));
}

#[test]
fn a_number_cast_keeps_its_types() {
    let program = lower("g: Integer -> F64 = n => n\ng 1");
    let body = definition(&program, "g");
    let cast = all_rvalues(body).into_iter().find_map(|r| match r {
        Rvalue::Cast { row, from, to, .. } => {
            Some((*row, from.clone(), to.clone()))
        }
        _ => None,
    });
    let (row, from, to) = cast.expect("a number cast");
    assert_eq!(row, ir::CastRow::Number);
    let label = |ty: &ir::Type| match ty {
        ir::Type::Meaning { index, .. } => meaning_label(&program, *index),
        other => panic!("{other:?}"),
    };
    assert_eq!((label(&from), label(&to)), ("Integer".into(), "F64".into()));
}

#[test]
fn an_entry_enters_the_value_it_wraps() {
    let program = lower("f = n => {\n    x: U8 = n\n    x\n}\nf 1");
    let body = definition(&program, "f");
    let entered: Vec<_> = {
        let tables = program.tables.borrow();
        body.blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement {
                Statement::Enter { entry, value } => {
                    Some((tables.entries[*entry].label.clone(), *value))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(entered.len(), 2, "{entered:?}");
    assert!(
        entered.iter().any(|(label, _)| label == "U8"),
        "{entered:?}"
    );
    assert!(entered
        .iter()
        .all(|(_, value)| matches!(value, Operand::Local(_))));
}

#[test]
fn a_pack_tags_its_value() {
    let program = lower("p: Integer | String = 1");
    let main = main_body(&program);
    let tag = all_rvalues(main).into_iter().find_map(|r| match r {
        Rvalue::Pack {
            tag: ir::Type::Meaning { index, .. },
            value: Operand::Const(_),
        } => Some(meaning_label(&program, *index)),
        _ => None,
    });
    assert_eq!(tag.as_deref(), Some("Integer"));
}

#[test]
fn modules_initialize_in_the_recorded_order() {
    let files: Vec<_> = [
        ("z", "use b\npub z = b.value"),
        ("b", "pub value = 2"),
        ("a", "pub value = 1"),
    ]
    .into_iter()
    .map(|(name, text)| (name.into(), parse(lex(text)).unwrap()))
    .collect();
    let module = parse(lex("use z a b\nz.z")).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .unwrap();
    let program = compile::compile(&unit);
    super::walk::check_program(&program);
    let labels: Vec<_> = {
        let tables = program.tables.borrow();
        program
            .inits
            .iter()
            .map(|(module, _)| tables.modules[*module].label.clone())
            .collect()
    };
    assert_eq!(labels, ["b", "z", "a"]);
    for (_, body) in &program.inits {
        let body = &program.bodies[*body];
        assert_eq!(body.arity, 0);
        assert!(body
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(statement, Statement::Store { .. })));
    }
    assert!(all_rvalues(main_body(&program))
        .iter()
        .any(|r| matches!(r, Rvalue::Use(Operand::Global(_)))));
}
