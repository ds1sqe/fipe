use super::{check, ty};
use fipe::{decl::Declarations, infer::Reason, typed::Supply};

#[test]
fn flags_collect_and_preserve_supplied_order() {
    let mut declarations = Declarations::new();
    declarations.commands("f: (prefix: String) -> (long: Bool = False) -> (recursive: Bool = False) -> (values: Vec<String>) -> Integer").unwrap();
    for text in [
        "f x a b c",
        "f x [\"a\", \"b\"]",
        "f -lr x a",
        "f -long -recursive x a",
        "f -values [\"a\"] x",
    ] {
        let (_, typed, errors) = check(text, &declarations);
        assert!(errors.is_empty(), "{text}: {errors:?}");
        assert_eq!(typed.bound.get(&(0, text.len())).unwrap().open, []);
    }
    let text = "f x a b c";
    let (_, typed, _) = check(text, &declarations);
    assert_eq!(
        typed.bound[&(0, text.len())]
            .supplies
            .iter()
            .filter(|(slot, value)| *slot == 3
                && matches!(value, Supply::Element(_)))
            .count(),
        3
    );
    for (text, reason) in [
        ("f -lr=True", Reason::NonBoolChaining),
        ("f -lp", Reason::NonBoolChaining),
        ("f -ll", Reason::FilledSpot),
    ] {
        let (_, _, errors) = check(text, &declarations);
        assert!(
            errors.iter().any(|e| e.reason == reason),
            "{text}: {errors:?}"
        );
    }
    let (_, _, errors) = check("f -z", &declarations);
    assert!(errors
        .iter()
        .any(|e| matches!(e.reason, Reason::UnknownSpot { .. })));
}

#[test]
fn defaults_open_spots_and_pipe_are_recorded() {
    assert_eq!(
        ty("type Point { x: Integer, y: Integer = 0 }\nPoint -y 4 -x 3"),
        "Point"
    );
    assert_eq!(
        ty("type Point { x: Integer, y: Integer = 0 }\nPoint 3"),
        "Point"
    );
    assert_eq!(
        ty("type Point { x: Integer, y: Integer = 0 }\n1 | Point 1"),
        "Point"
    );
    assert_eq!(
        ty("dir = (width = 80) (cols = (width / 8)) => cols\ndir"),
        "Integer"
    );
    assert_eq!(
        ty("add = a b => a + b\nadd2 = add 2"),
        "Add<a> b, Literal b => a -> b"
    );
    let source = "f = a b c => c\n3 | f 1 2";
    let (_, typed, errors) = check(source, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    let bound = typed
        .bound
        .values()
        .find(|b| b.supplies.iter().any(|(_, s)| matches!(s, Supply::Pipe(_))))
        .unwrap();
    assert!(matches!(bound.supplies.first(), Some((2, Supply::Pipe(_)))));
    assert_eq!(bound.open, []);
}

#[test]
fn invalid_host_default_is_reported_when_needed() {
    let mut declarations = Declarations::new();
    declarations
        .commands("f: (depth: U64 = True) -> Integer")
        .unwrap();
    let (_, _, errors) = check("f", &declarations);
    assert!(
        errors.iter().any(
            |e| matches!(&e.reason, Reason::Default { name } if name == "depth")
        ),
        "{errors:?}"
    );
    assert_eq!(check("f 2", &declarations).2, []);
}

#[test]
fn generic_whole_literal_is_checked_at_its_use() {
    let text = "big = _ => 9007199254740993\nx: F64 = big ()";
    let (_, _, errors) = check(text, &Declarations::new());
    assert!(
        errors.iter().any(|e| e.reason == Reason::OutOfRange),
        "{errors:?}"
    );
}

#[test]
fn declaration_spots_are_unambiguous() {
    for (text, expected) in [
        (
            "f: (x: Integer) -> (x: Integer) -> ()",
            fipe::decl::SpotProblem::Duplicate("x".into()),
        ),
        (
            "f: (long: Bool) -> (links: Bool) -> ()",
            fipe::decl::SpotProblem::AmbiguousFlag('l'),
        ),
        (
            "type A\nimpl A { f: (self: Self) -> Integer -> () }",
            fipe::decl::SpotProblem::SelfNotLast,
        ),
    ] {
        let errors = Declarations::new().commands(text).unwrap_err();
        assert!(errors.iter().any(|e| matches!(&e.reason, fipe::decl::Reason::Spots { problem } if problem == &expected)), "{text}: {errors:?}");
    }
    for text in ["f: Bool -> Bool -> () = long links => ()", "f = x x => x"] {
        let (_, _, errors) = check(text, &Declarations::new());
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.reason, Reason::Spots { .. })),
            "{text}: {errors:?}"
        );
    }
}

#[test]
fn field_type_defaults_do_not_close_function_spots() {
    for text in [
        "type R { flag: Bool, xs: Vec<Integer>, queue: Deque<Integer>, maybe: Maybe<Integer>, unit: (), flags: [Bool; 2] }\nR",
        "type Inner { x: Integer = 2 }\ntype R { inner: Inner }\nR",
        "type Off: Bool where !self\ntype R { off: Off }\nR",
    ] {
        assert_eq!(ty(text), "R");
    }
    assert_eq!(
        ty("type On: Bool where self\ntype R { on: On }\nR"),
        "On -> R"
    );
    assert_eq!(ty("type R { n: Integer }\nR"), "Integer -> R");
    let mut declarations = Declarations::new();
    declarations.commands("f: (values: Vec<Integer>) -> Integer\ng: (values: Vec<Integer> = []) -> Integer").unwrap();
    for (source, open) in [("f", vec![0]), ("g", vec![])] {
        let (_, typed, errors) = check(source, &declarations);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(typed.bound[&(0, source.len())].open, open);
    }
}

#[test]
fn recursive_literal_error_names_the_use_and_literal() {
    let text = "f = x => {\n    _ = f \"a\"\n    f True\n}";
    let (_, _, errors) = check(text, &Declarations::new());
    assert!(errors.iter().any(|e| matches!(e.reason, Reason::Mismatch { other, .. } if &text[other.start..other.end] == "\"a\"" && &text[e.span.start..e.span.end] == "True")), "{errors:?}");
}

#[test]
fn host_defaults_use_the_declaring_module_and_earlier_spots() {
    let mut declarations = Declarations::new();
    declarations.module("config", "pub base: Integer\npub f: (width: Integer = base) -> (cols: Integer = (width / 8)) -> Integer").unwrap();
    let (_, typed, errors) = check("use config\nconfig.f", &declarations);
    assert!(errors.is_empty(), "{errors:?}");
    let call = typed
        .bound
        .values()
        .find(|b| b.defaults.len() == 2)
        .unwrap();
    assert_eq!(call.open, []);
    declarations
        .module("config", "pub f: (width: Integer = missing) -> Integer")
        .unwrap();
    assert!(check("use config\nconfig.f", &declarations).2.iter().any(
        |e| matches!(&e.reason, Reason::Default { name } if name == "width")
    ));
}

#[test]
fn named_values_and_collected_elements_keep_source_order() {
    let mut declarations = Declarations::new();
    declarations
        .commands("f: (prefix: String) -> (values: Vec<String>) -> Integer")
        .unwrap();
    let text = "f a -prefix x b";
    let (module, typed, errors) = check(text, &declarations);
    assert!(errors.is_empty(), "{errors:?}");
    let supplies = &typed.bound[&(0, text.len())].supplies;
    let words: Vec<_> = supplies
        .iter()
        .map(|(_, source)| match source {
            Supply::Expression(id) | Supply::Element(id) => {
                let span = module.expr(*id).unwrap().span;
                &text[span.start..span.end]
            }
            _ => panic!(),
        })
        .collect();
    assert_eq!(words, ["a", "x", "b"]);
}

#[test]
fn outcome_rules_prefer_the_same_family() {
    assert_eq!(ty("Just 1 |: (_ => Just 2)"), "Maybe<Integer>");
    assert_eq!(ty("Just 1 |: (_ => fault \"stop\")"), "Maybe<a>");
    assert_eq!(ty("Just 1 || Just 2"), "Maybe<Integer>");
    assert_eq!(ty("Just 1 || 2"), "Integer");
    assert_eq!(ty("Just 1 |! ()"), "Maybe<Integer>");
    assert_eq!(ty("Just 1 |! Just 2"), "Maybe<Integer>");
    for (text, reason) in [
        ("Just 1 |! (x => x)", Reason::OpenSpot),
        (
            "e: Either<(), Integer> = Right 1\ne || 0",
            Reason::NeedsMaybeOrResult,
        ),
        (
            "e: Either<(), Integer> = Right 1\ne && 0",
            Reason::NeedsMaybeOrResult,
        ),
    ] {
        assert!(check(text, &Declarations::new())
            .2
            .iter()
            .any(|e| e.reason == reason));
    }
}

fn fixture_host() -> Declarations {
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
    declarations
}

#[test]
fn prelude_and_host_function_pipes_fill_the_final_spot() {
    let declarations = fixture_host();
    let mut count = 0;
    for namespace in std::iter::once(&declarations.prelude)
        .chain(declarations.commands.iter())
        .chain(&declarations.modules)
    {
        for value in &namespace.values {
            let fipe::decl::Kind::Function { spots, .. } = &value.kind else {
                continue;
            };
            let name =
                if matches!(namespace.name.as_str(), "prelude" | "commands") {
                    value.name.clone()
                } else {
                    format!("{}.{}", namespace.name, value.name)
                };
            let source = match name.as_str() {
                "to" => "probe: Integer -> U8 = input => input | to<U8>".to_string(),
                "try" => "probe: Integer -> Result<U8, LimitError> = input => input | try<U8>".to_string(),
                "read" => "probe: String -> Result<Integer, ParseError> = input => input | read<Integer>".to_string(),
                _ => format!("probe = input => input | {name}"),
            };
            let (_, typed, errors) = check(&source, &declarations);
            assert!(
                errors.iter().all(|e| matches!(
                    e.reason,
                    Reason::Conversion { .. } | Reason::Ambiguous { .. }
                )),
                "{name}: {errors:?}"
            );
            let bound = typed
                .bound
                .values()
                .find(|b| {
                    b.supplies.iter().any(|(_, s)| matches!(s, Supply::Pipe(_)))
                })
                .unwrap();
            assert!(
                bound
                    .supplies
                    .iter()
                    .any(|(slot, s)| *slot + 1 == spots.len()
                        && matches!(s, Supply::Pipe(_))),
                "{name}: {bound:?}"
            );
            count += 1;
        }
    }
    assert!(count > 30, "{count}");
}

#[test]
fn nested_file_references_share_checking() {
    use fipe::{
        ast::Entry, infer::infer, lex::lex, parse::parse, resolve::resolve,
    };
    let declarations = Declarations::new();
    let mut files =
        vec![("m0".to_string(), parse(lex("pub id = x => x")).unwrap())];
    for depth in 1..=18 {
        let previous = depth - 1;
        let source = format!(
            "use m{previous}\npub id = x => m{previous}.id (m{previous}.id x)"
        );
        files.push((format!("m{depth}"), parse(lex(&source)).unwrap()));
    }
    let module = parse(lex("use m18\nm18.id 3")).unwrap();
    let resolved = resolve(&module, &declarations, &files).unwrap();
    let typed = infer(&module, &resolved, &declarations, &files).unwrap();
    let Some(Entry::Line(id)) = module.entries.last() else {
        panic!()
    };
    assert_eq!(typed.expr(*id).unwrap().to_string(), "Integer");
}

#[test]
fn every_example_call_has_a_binding_record() {
    use fipe::{
        infer::infer_partial, lex::lex, parse::parse, resolve::resolve,
    };
    let declarations = fixture_host();
    let files: Vec<_> = std::fs::read_dir("Documentation/examples")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "fipe"))
        .map(|p| {
            let source = std::fs::read_to_string(&p).unwrap();
            (
                p.file_stem().unwrap().to_str().unwrap().to_string(),
                parse(lex(&source)).unwrap(),
            )
        })
        .collect();
    assert_eq!(files.len(), 6);
    let mut calls = 0;
    for (stem, module) in &files {
        let resolved = resolve(module, &declarations, &files).unwrap();
        let (typed, _) =
            infer_partial(module, &resolved, &declarations, &files);
        for id in call_ids(module) {
            let span = module.expr(id).unwrap().span;
            assert!(
                typed.bound.contains_key(&(span.start, span.end)),
                "{stem}: {span:?}"
            );
            calls += 1;
        }
    }
    assert!(calls > 60, "{calls}");
}

fn call_ids(module: &fipe::ast::Module) -> Vec<fipe::ast::ExprId> {
    use fipe::ast::{
        ArgumentKind, Entry, ExprKind, ImplMember, ItemKind, Member,
    };
    let mut pending = Vec::new();
    for entry in &module.entries {
        match entry {
            Entry::Line(id) => pending.push(*id),
            Entry::Item(item) => match &item.kind {
                ItemKind::Impl { members, .. } => {
                    for member in members {
                        match member {
                            ImplMember::Named { value, .. } => {
                                pending.extend(value);
                            }
                            ImplMember::Constructor { arm, .. } => {
                                pending.push(arm.body);
                            }
                        }
                    }
                }
                ItemKind::Type {
                    members: Some(members),
                    ..
                } => {
                    for member in members {
                        if let Member::Field(field) = member {
                            pending.extend(field.default);
                        }
                    }
                }
                _ => {}
            },
        }
    }
    let mut calls = Vec::new();
    while let Some(id) = pending.pop() {
        match &module.expr(id).unwrap().kind {
            ExprKind::Call {
                callee, arguments, ..
            } => {
                calls.push(id);
                pending.push(*callee);
                for argument in arguments {
                    match &argument.kind {
                        ArgumentKind::Positional(id) => pending.push(*id),
                        ArgumentKind::Flag { value, .. } => {
                            pending.extend(value);
                        }
                    }
                }
            }
            ExprKind::Group(ids)
            | ExprKind::Block(ids)
            | ExprKind::List(ids)
            | ExprKind::Tuple(ids) => pending.extend(ids),
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipe { left, right, .. } => {
                pending.extend([left, right]);
            }
            ExprKind::Bind { value, .. }
            | ExprKind::Field { value, .. }
            | ExprKind::Not { value, .. } => pending.push(*value),
            ExprKind::With { value, body, .. } => pending.extend([value, body]),
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                pending.extend([condition, then_body]);
                pending.extend(else_body);
            }
            ExprKind::Arms(arms) => {
                for arm in arms {
                    pending.push(arm.body);
                    pending.extend(arm.guard);
                }
            }
            ExprKind::Record { fields, .. } => {
                pending.extend(fields.iter().map(|f| f.value));
            }
            ExprKind::Outcome { payload, .. } => pending.extend(payload),
            _ => {}
        }
    }
    calls
}

#[test]
fn program_field_defaults_are_checked_at_their_declaration() {
    for text in [
        "type R { n: Integer = True }",
        "x = True\ntype R { n: Integer = x }",
        "type R { width: Integer = 80, cols: Integer = (width / 8) }\nR",
    ] {
        let (_, _, errors) = check(text, &Declarations::new());
        if text.contains("True") {
            assert!(
                errors
                    .iter()
                    .any(|e| matches!(e.reason, Reason::Mismatch { .. })),
                "{text}: {errors:?}"
            );
        } else {
            assert!(errors.is_empty(), "{text}: {errors:?}");
        }
    }
}

#[test]
fn a_function_value_keeps_open_spot_order_but_no_defaults() {
    assert_eq!(ty("f = a b c => c\nh = f\n3 | h 1"), "a -> Integer");
    assert_eq!(ty("f = a (b = 2) c => c\nh = f\nh 1"), "a -> a");
    let source = "f = a b c => c\nh = f\n3 | h 1";
    let (_, typed, errors) = check(source, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    let bound = typed
        .bound
        .values()
        .find(|b| b.supplies.iter().any(|(_, s)| matches!(s, Supply::Pipe(_))))
        .unwrap();
    assert_eq!(bound.open, [1]);
    assert!(matches!(bound.supplies.first(), Some((2, Supply::Pipe(_)))));
}

#[test]
fn host_written_values_are_typed_once_in_their_home_module() {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "config",
            "pub base = True\npub f: (n: Integer = base) -> Integer",
        )
        .unwrap();
    let (_, _, errors) = check("use config\nconfig.f", &declarations);
    assert!(
        errors.iter().any(
            |e| matches!(&e.reason, Reason::Default { name } if name == "n")
        ),
        "{errors:?}"
    );
    let source = "use config\nconfig.base";
    let (module, typed, errors) = check(source, &declarations);
    assert!(errors.is_empty(), "{errors:?}");
    let Some(fipe::ast::Entry::Line(id)) = module.entries.last() else {
        panic!()
    };
    assert_eq!(typed.expr(*id).unwrap().to_string(), "Bool");
    declarations.module("config", "pub bad = 1 + True").unwrap();
    let (_, _, errors) =
        check("use config\nconfig.bad\nconfig.bad", &declarations);
    assert_eq!(errors.iter().filter(|e| matches!(&e.reason, Reason::DeclarationValue { name, .. } if name == "bad")).count(), 1, "{errors:?}");
    declarations
        .module(
            "config",
            "pub base = 32\npub f: (n: Integer = base) -> Integer",
        )
        .unwrap();
    assert_eq!(check("use config\nconfig.f", &declarations).2, []);
}

#[test]
fn bare_flags_enter_limited_bool_spots() {
    let (_, _, errors) = check(
        "type Off: Bool where !self\ntype R { off: Off }\nR -off",
        &Declarations::new(),
    );
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.reason, Reason::Limit { .. })),
        "{errors:?}"
    );
    assert_eq!(
        ty("type Off: Bool where !self\ntype R { off: Off }\nR -off=False"),
        "R"
    );
}

#[test]
fn constructor_facts_use_the_pipe_before_a_field_default() {
    let source = "type Point { x: Integer, y: Integer = 0 }\n1 | Point 1";
    let (module, typed, errors) = check(source, &Declarations::new());
    assert!(errors.is_empty(), "{errors:?}");
    let Some(fipe::ast::Entry::Line(id)) = module.entries.last() else {
        panic!()
    };
    let known = typed.known(*id).unwrap();
    assert_eq!(known.parts[0].1.range, known.parts[1].1.range);
    assert_eq!(known.parts[1].1.range.as_ref().unwrap().lo.to_string(), "1");
}

#[test]
fn collected_and_defaulted_fields_keep_known_facts() {
    let prefix =
        "type R { x: Integer, xs: Vec<Integer> } where self.xs.len == 3\n";
    assert_eq!(ty(&format!("{prefix}R 0 1 2 3")), "R");
    for call in ["R 0", "R 0 1 2"] {
        let (_, _, errors) =
            check(&format!("{prefix}{call}"), &Declarations::new());
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.reason, Reason::Limit { .. })),
            "{call}: {errors:?}"
        );
    }
    assert_eq!(ty("type Pair<T> { x: T, y: T }\nPair True"), "Pair<Bool>");
}

#[test]
fn calls_retain_implementations_and_instantiated_meanings() {
    let (module, typed, errors) =
        super::check("inc = n => n + 1\ninc 2.0", &Declarations::new());
    assert_eq!(errors, []);
    let fipe::ast::Entry::Line(call) = module.entries.last().unwrap() else {
        panic!()
    };
    let span = module.expr(*call).unwrap().span;
    let site = (span.start, span.end);
    assert!(matches!(
        typed.selected.get(&site),
        Some(fipe::typed::Selected::Value { .. })
    ));
    assert_eq!(
        typed.meaning_arguments[&site]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["F64"]
    );
    assert_eq!(typed.callee_types[&site].to_string(), "F64 -> F64");
}

#[test]
fn function_references_record_instantiation_without_a_call() {
    let source = "inc = n => n + 1\nf: F64 -> F64 = inc";
    let (module, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let fipe::ast::Entry::Line(binding) = module.entries.last().unwrap() else {
        panic!()
    };
    let fipe::ast::ExprKind::Bind { value, .. } =
        module.expr(*binding).unwrap().kind
    else {
        panic!()
    };
    let reference = module.expr(value).unwrap();
    let site = (reference.span.start, reference.span.end);
    assert_eq!(
        typed.meaning_arguments[&site]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["F64"]
    );
    assert!(matches!(
        typed.selected[&site],
        fipe::typed::Selected::Value { .. }
    ));
}

#[test]
fn member_chains_keep_each_field_index() {
    let source = "type Inner { first: Bool, n: Integer }\ntype Outer { inner: Inner }\no = Outer (Inner { first: True, n: 3 })\no.inner.n";
    let (module, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let fipe::ast::Entry::Line(value) = module.entries.last().unwrap() else {
        panic!()
    };
    let span = module.expr(*value).unwrap().span;
    let steps = &typed.accesses[&(span.start, span.end)];
    assert_eq!(steps.len(), 2);
    assert!(matches!(
        steps[0].selected,
        fipe::typed::Selected::Field { index: 0, .. }
    ));
    assert!(matches!(
        steps[1].selected,
        fipe::typed::Selected::Field { index: 1, .. }
    ));
    assert_eq!(steps[1].result.to_string(), "Integer");
}

#[test]
fn operators_record_concrete_and_generic_implementations() {
    for (source, generic) in [("1 + 2", false), ("add = x y => x + y", true)] {
        let (_, typed, errors) = super::check(source, &Declarations::new());
        assert_eq!(errors, []);
        let start = source.find('+').unwrap();
        let selected = &typed.selected[&(start, start + 1)];
        assert_eq!(
            matches!(selected, fipe::typed::Selected::Member { .. }),
            generic
        );
        if !generic {
            assert!(matches!(selected, fipe::typed::Selected::Declared { .. }));
            let binding = &typed.bound[&(start, start + 1)];
            assert_eq!(
                binding
                    .supplies
                    .iter()
                    .map(|(spot, _)| *spot)
                    .collect::<Vec<_>>(),
                [1, 0]
            );
            assert_eq!(
                typed.callee_types[&(start, start + 1)].to_string(),
                "Integer -> Integer -> Integer"
            );
        }
    }
}

#[test]
fn recursive_references_supply_the_generalized_meanings() {
    let source = "f = n => if n == 0 { n } else { f (n - 1) }";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let recursive = source.rfind("f (").unwrap();
    let selected = &typed.selected[&(recursive, recursive + 1)];
    let fipe::typed::Selected::Value { site, .. } = selected else {
        panic!()
    };
    let scheme = &typed.schemes[&(site.start, site.end)];
    assert_ne!(scheme.variables.len(), 0);
    assert_eq!(
        typed.meaning_arguments[&(recursive, recursive + 1)].len(),
        scheme.variables.len()
    );
}

#[test]
fn type_layouts_keep_parameters_distinct_from_fixed_fields() {
    let source = "type Pair<T> { fixed: Integer, value: T }\na = Pair 1 2\nb = Pair 3 \"text\"";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let shapes: Vec<_> = typed.shapes.iter().filter(|s| matches!(&s.ty, fipe::types::Type::Named { name, .. } if name.name == "Pair")).collect();
    assert_eq!(shapes.len(), 1);
    let fipe::types::Type::Named { arguments, .. } = &shapes[0].ty else {
        panic!()
    };
    let fields = &shapes[0].constructors[0].fields;
    assert_eq!(fields[0].ty.to_string(), "Integer");
    assert_eq!(fields[1].ty, arguments[0]);
    assert_ne!(fields[0].ty, fields[1].ty);
}

#[test]
fn hidden_meanings_are_only_for_receiverless_members() {
    let source = "id = x => x\ninc = x => x + 1\nshow = x => x.show";
    let (module, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let hidden: Vec<_> = module
        .entries
        .iter()
        .filter_map(|entry| {
            let fipe::ast::Entry::Line(id) = entry else {
                return None;
            };
            let fipe::ast::ExprKind::Bind { pattern, .. } =
                module.expr(*id)?.kind
            else {
                return None;
            };
            let span = module.pattern(pattern)?.span;
            Some(typed.hidden[&(span.start, span.end)].len())
        })
        .collect();
    assert_eq!(hidden, [0, 1, 0]);
}

#[test]
fn program_literals_retain_the_selected_reader() {
    let source = "type Mask: Literal { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nm: Mask = 0b0000_1111";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let start = source.rfind("0b").unwrap();
    let fipe::typed::Selected::Value { site, .. } =
        &typed.selected[&(start, source.len())]
    else {
        panic!()
    };
    assert_eq!(&source[site.start..site.end], "literal");
}

#[test]
fn defaults_retain_each_receiving_instantiation() {
    let mut declarations = Declarations::new();
    declarations
        .commands("copy: (value: a) -> (other: a = value) -> a")
        .unwrap();
    let (_, typed, errors) =
        super::check("copy 1\ncopy \"text\"", &declarations);
    assert_eq!(errors, []);
    let meanings: Vec<_> = typed
        .default_instances
        .values()
        .map(|instance| {
            instance.typed.expr(instance.value).unwrap().to_string()
        })
        .collect();
    assert_eq!(meanings, ["Integer", "String"]);
    assert!(typed
        .default_instances
        .keys()
        .all(|(_, _, spot)| *spot == 1));
}

#[test]
fn supplied_values_keep_their_definition_references() {
    let mut declarations = Declarations::new();
    declarations.commands("answer = 42").unwrap();
    let (_, typed, errors) = super::check("answer\nanswer", &declarations);
    assert_eq!(errors, []);
    let first = &typed.selected[&(0, 6)];
    assert!(matches!(
        first,
        fipe::typed::Selected::Value {
            origin: fipe::resolve::Origin::Commands,
            ..
        }
    ));
    assert_eq!(first, &typed.selected[&(7, 13)]);
    assert_eq!(typed.declared.len(), 1);
}

#[test]
fn generic_members_record_field_and_member_dispatch_rows() {
    let source = "type Record { n: Integer }\nget = r => r.n\na = get (Record 4)\ninc = n => n + 1\ninc a";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let field = typed.dispatch.iter().find(|d| d.name == "n").unwrap();
    assert!(field.rows.iter().any(|r| matches!(
        r.selected,
        fipe::typed::Selected::Field { index: 0, .. }
    )));
    let add = typed.dispatch.iter().find(|d| d.name == "add").unwrap();
    let integer = add
        .rows
        .iter()
        .find(|r| r.receiver.to_string() == "Integer")
        .unwrap();
    assert_eq!(integer.receiver_spot, Some(1));
    assert!(matches!(
        integer.selected,
        fipe::typed::Selected::Declared { .. }
    ));
}

#[test]
fn dispatch_keeps_each_overload_argument_meaning() {
    let source = "type W { n: F64 }\ntype V: Mul<F64> & Mul<W> { x: F64 }\nimpl V: Mul<F64> { mul = k self => Self (self.x * k) }\nimpl V: Mul<W> { mul = w self => Self (self.x * w.n) }\nscale = x y => x * y\nv = V 1.0\nk = 2.0\nscale v k";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    let dispatch = typed.dispatch.iter().find(|d| d.name == "mul").unwrap();
    let arguments: Vec<_> = dispatch
        .rows
        .iter()
        .filter(|row| row.receiver.to_string() == "V")
        .map(|row| row.argument.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(arguments, ["F64", "W"]);
}

#[test]
fn limits_retain_local_and_supplied_test_functions() {
    let mut declarations = Declarations::new();
    declarations.commands("accept: I64 -> ()").unwrap();
    let (module, typed, errors) = super::check(
        "type Positive: Integer where self > 0\nf = n => accept n",
        &declarations,
    );
    assert_eq!(errors, []);
    let local = typed
        .tests
        .iter()
        .find(|test| test.owner.to_string() == "Positive")
        .unwrap();
    assert!(local.expression.is_none());
    assert_eq!(typed.expr(local.value).unwrap().to_string(), "Bool");
    assert!(module.expr(local.value).is_some());
    let supplied: Vec<_> = typed
        .tests
        .iter()
        .filter(|test| test.owner.to_string() == "I64")
        .collect();
    assert_eq!(supplied.len(), 2);
    for test in supplied {
        let expression = test.expression.as_ref().unwrap();
        assert_eq!(
            expression.typed.expr(expression.value).unwrap().to_string(),
            "Bool"
        );
    }
}

#[test]
fn supplied_generic_limit_tests_keep_their_parameter_assumptions() {
    let mut declarations = Declarations::new();
    declarations.commands("type Box<T: Eq> { v: T } where self.v == self.v\nkeep: Box<Integer> -> ()\nget: Box<Integer>").unwrap();
    let (_, typed, errors) = super::check("keep get", &declarations);
    assert_eq!(errors, []);
    let functions: Vec<_> = typed
        .tests
        .iter()
        .filter(|test| test.owner.to_string().starts_with("Box"))
        .collect();
    assert_eq!(functions.len(), 1);
    assert!(functions[0].expression.is_some());
}

#[test]
fn unused_static_constraints_do_not_need_hidden_meanings() {
    let (_, typed, errors) =
        super::check("id: Literal a => a -> a = x => x", &Declarations::new());
    assert_eq!(errors, []);
    assert_eq!(typed.hidden[&(0, 2)], []);
}

#[test]
fn hidden_meanings_follow_references_and_written_body_parameters() {
    let source = "id: Literal a => a -> a = x => x\nuse_id = x => id x\nread_text: Read a => String -> Result<a, ParseError> = s => read s\nget = s => read_text s";
    let (_, typed, errors) = super::check(source, &Declarations::new());
    assert_eq!(errors, []);
    for (name, count) in
        [("id", 0), ("use_id", 0), ("read_text", 1), ("get", 1)]
    {
        let start = source.find(name).unwrap();
        let site = (start, start + name.len());
        assert_eq!(typed.hidden[&site].len(), count, "{name}");
    }
    assert_eq!(
        typed.body_parameters[&(0, 2)],
        [fipe::types::Type::Rigid("a".into())]
    );
}

#[test]
fn literal_patterns_record_equality_and_preparation() {
    for source in [
        "f: Integer -> String = { 0 => \"a\", _ => \"b\" }",
        "f: String -> String = { \"x\" => \"a\", _ => \"b\" }",
        "type Mask: Literal & Eq { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nf: Mask -> String = { 0 => \"a\", _ => \"b\" }",
    ] {
        let (module, typed, errors) = super::check(source, &Declarations::new());
        assert_eq!(errors, [], "{source}");
        assert_eq!(typed.pattern_equals.len(), 1);
        let comparison = typed.pattern_equals.values().next().unwrap();
        let fipe::ast::Entry::Line(line) = module.entries.last().unwrap() else { panic!() };
        let fipe::ast::ExprKind::Bind { value, .. } = module.expr(*line).unwrap().kind else { panic!() };
        let fipe::ast::ExprKind::Arms(arms) = &module.expr(value).unwrap().kind else { panic!() };
        let span = module.pattern(arms[0].patterns[0]).unwrap().span;
        assert!(typed.selected.contains_key(&(span.start, span.end)));
        assert!(!matches!(comparison.selected, fipe::typed::Selected::Static { .. }));
    }
    let source = "type Mask: Literal { bits: U8 }\nimpl Mask: Literal { literal = f => Self 0 }\nf: Mask -> String = { 0 => \"a\", _ => \"b\" }";
    let (_, _, errors) = super::check(source, &Declarations::new());
    assert!(errors.iter().any(|error| matches!(&error.reason, Reason::MissingClaim { claim, .. } if claim.to_string() == "Eq")));
}

#[test]
fn a_limited_container_type_defaults_when_the_empty_value_is_proved() {
    assert_eq!(
        ty("type V8: Vec<U8> where self.len < 5\ntype R4 { v: V8 }\nR4"),
        "R4"
    );
    assert_eq!(
        ty("type V9: Vec<U8> where self.len > 0\ntype R8 { v: V9 }\nR8"),
        "V9 -> R8"
    );
}
