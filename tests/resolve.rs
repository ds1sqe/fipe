use fipe::{
    ast::{Entry, ExprKind, Module},
    decl::Declarations,
    lex::lex,
    parse::parse,
    resolve::{resolve, Error, Origin, Reason, Resolved, Target, Unchecked},
    token::{Source, Span},
};

fn host() -> Declarations {
    let mut declarations = Declarations::new();
    declarations
        .commands(include_str!("fixtures/host/commands.fipe"))
        .unwrap();
    for (name, text) in [
        ("fs", include_str!("fixtures/host/fs.fipe")),
        ("sys", include_str!("fixtures/host/sys.fipe")),
        ("cli", include_str!("fixtures/host/cli.fipe")),
        ("text", include_str!("fixtures/host/text.fipe")),
    ] {
        declarations.module(name, text).unwrap();
    }
    declarations
}

fn module(text: &str) -> Module {
    parse(lex(text)).unwrap_or_else(|errors| panic!("{text:?}: {errors:?}"))
}

fn ok(text: &str) -> (Module, Resolved) {
    let parsed = module(text);
    let resolved = resolve(&parsed, &host(), &[]).unwrap_or_else(|errors| {
        let located: Vec<_> = errors
            .iter()
            .map(|e| (&e.reason, Source::new(text).slice(e.span)))
            .collect();
        panic!("{text:?}: {located:?}")
    });
    (parsed, resolved)
}

fn errors(text: &str) -> Vec<(Reason, Option<&str>)> {
    let parsed = module(text);
    let errors: Vec<Error> = resolve(&parsed, &host(), &[]).expect_err(text);
    errors
        .into_iter()
        .map(|e| (e.reason, Source::new(text).slice(e.span)))
        .collect()
}

/// The target of the first name expression spelled `spelling`.
fn target<'a>(
    text: &str,
    parsed: &Module,
    resolved: &'a Resolved,
    spelling: &str,
) -> &'a Target {
    let source = Source::new(text);
    resolved
        .expressions()
        .find(|(id, _)| {
            let expr = parsed.expr(*id).unwrap();
            matches!(expr.kind, ExprKind::Name(_) | ExprKind::Instance { .. })
                && source.slice(expr.span) == Some(spelling)
        })
        .map_or_else(|| panic!("no name {spelling}"), |(_, target)| target)
}

fn value(origin: Origin, name: &str) -> Target {
    Target::Value {
        origin,
        name: name.into(),
    }
}

/// Every example, parsed, keyed by stem: the module tree `use` reads.
fn examples() -> Vec<(String, Module)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fipe"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
            (stem, module(&std::fs::read_to_string(&path).unwrap()))
        })
        .collect()
}

fn example(stem: &str) -> (String, Module, Resolved) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Documentation/examples")
        .join(format!("{stem}.fipe"));
    let text = std::fs::read_to_string(path).unwrap();
    let parsed = module(&text);
    let resolved =
        resolve(&parsed, &host(), &examples()).unwrap_or_else(|errors| {
            let located: Vec<_> = errors
                .iter()
                .map(|e| {
                    (
                        &e.reason,
                        Source::new(&text).line_col(e.span.start),
                        Source::new(&text).slice(e.span),
                    )
                })
                .collect();
            panic!("{stem}: {located:?}")
        });
    (text, parsed, resolved)
}

#[test]
fn every_example_resolves() {
    let files = examples();
    assert!(files.len() >= 6);
    for (stem, _) in &files {
        example(stem);
    }
}

#[test]
fn the_first_two_examples_resolve() {
    for path in ["01-files", "02-outcomes"] {
        let (text, parsed, resolved) = example(path);
        assert!(resolved.exports.is_empty(), "{path}");
        if path == "01-files" {
            assert_eq!(
                target(&text, &parsed, &resolved, "ls"),
                &value(Origin::Commands, "ls")
            );
            assert_eq!(
                target(&text, &parsed, &resolved, "filter"),
                &value(Origin::Prelude, "filter")
            );
            assert_eq!(
                target(&text, &parsed, &resolved, "map"),
                &value(Origin::Prelude, "map")
            );
            // A binding above its use, at its site.
            assert!(matches!(
                target(&text, &parsed, &resolved, "older"),
                Target::Local { .. }
            ));
            let Target::Members { head, members } =
                target(&text, &parsed, &resolved, "e.size")
            else {
                panic!()
            };
            assert!(matches!(**head, Target::Local { .. }));
            assert_eq!(members, &["size"]);
        } else {
            assert!(matches!(
                target(&text, &parsed, &resolved, "Ok"),
                Target::Constructor {
                    origin: Origin::Prelude,
                    ..
                }
            ));
            assert_eq!(
                target(&text, &parsed, &resolved, "read<U16>"),
                &value(Origin::Prelude, "read")
            );
        }
    }
}

#[test]
fn bare_names_follow_the_lookup_order_and_shadow() {
    let text = "x = 1\nx\nfs\nls\nshow\ncat nope\nx = 2\nx";
    let (parsed, resolved) = ok(text);
    let source = Source::new(text);
    let [Entry::Line(_), Entry::Line(first), .., Entry::Line(last)] =
        parsed.entries.as_slice()
    else {
        panic!()
    };
    let Target::Local { site } = resolved.expr(*first).unwrap() else {
        panic!()
    };
    assert_eq!(*site, Span { start: 0, end: 1 });
    let Target::Local { site } = resolved.expr(*last).unwrap() else {
        panic!()
    };
    assert_eq!(source.slice(*site), Some("x"));
    assert_eq!(site.start, text.rfind("x = 2").unwrap());
    assert_eq!(
        target(text, &parsed, &resolved, "fs"),
        &Target::Module(Origin::Host("fs".into()))
    );
    assert_eq!(
        target(text, &parsed, &resolved, "ls"),
        &value(Origin::Commands, "ls")
    );
    assert_eq!(
        target(text, &parsed, &resolved, "show"),
        &value(Origin::Prelude, "show")
    );
    assert_eq!(target(text, &parsed, &resolved, "nope"), &Target::Word);

    assert_eq!(errors("nope 1"), [(Reason::UnknownName, Some("nope"))]);
    assert_eq!(errors("nope"), [(Reason::UnknownName, Some("nope"))]);
    assert_eq!(
        errors("cat nope.x"),
        [(Reason::UnknownName, Some("nope.x"))]
    );
    assert_eq!(errors("1 | nope"), [(Reason::UnknownName, Some("nope"))]);
}

#[test]
fn bindings_are_sequential_and_an_arm_binding_sees_itself() {
    // No hoisting: a use above the binding does not see it.
    assert_eq!(
        errors("f 1\nf = a => a + 1"),
        [(Reason::UnknownName, Some("f"))]
    );
    assert_eq!(errors("g 1\ng = 2"), [(Reason::UnknownName, Some("g"))]);
    // In argument position an unbound name is still text.
    let (parsed, resolved) = ok("cat f\nf = 1");
    let Entry::Line(call) = parsed.entries[0] else {
        panic!()
    };
    let ExprKind::Call { arguments, .. } = &parsed.expr(call).unwrap().kind
    else {
        panic!()
    };
    let fipe::ast::ArgumentKind::Positional(word) = &arguments[0].kind else {
        panic!()
    };
    assert_eq!(resolved.expr(*word), Some(&Target::Word));
    // An arm binding's own name is in scope in its arms.
    let (parsed, resolved) = ok("f = n => f n");
    let Entry::Line(line) = parsed.entries[0] else {
        panic!()
    };
    let ExprKind::Bind { value: arms, .. } = &parsed.expr(line).unwrap().kind
    else {
        panic!()
    };
    let ExprKind::Arms(arms) = &parsed.expr(*arms).unwrap().kind else {
        panic!()
    };
    let ExprKind::Call { callee, .. } =
        &parsed.expr(arms[0].body).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        resolved.expr(*callee),
        Some(&Target::Local {
            site: Span { start: 0, end: 1 }
        })
    );
    // Mutual recursion across bindings is an unknown name at the first.
    assert_eq!(
        errors("even = n => odd n\nodd = n => even n"),
        [(Reason::UnknownName, Some("odd"))]
    );
    // A second binding shadows, at file level as in a block.
    let (parsed, resolved) = ok("f = a => 1\nf = b => 2\nf 3");
    let Entry::Line(call) = parsed.entries[2] else {
        panic!()
    };
    let ExprKind::Call { callee, .. } = &parsed.expr(call).unwrap().kind else {
        panic!()
    };
    assert_eq!(
        resolved.expr(*callee),
        Some(&Target::Local {
            site: Span { start: 11, end: 12 }
        })
    );
    assert_eq!(
        errors("{ h 1\n h = a => a }"),
        [(Reason::UnknownName, Some("h"))]
    );
    // The members of one impl see each other in any order.
    let (parsed, resolved) = ok("type P {}\n\
        impl P { even = n => Self.odd n, odd = n => Self.even n }");
    let text =
        "type P {}\nimpl P { even = n => Self.odd n, odd = n => Self.even n }";
    assert_eq!(
        target(text, &parsed, &resolved, "Self.odd"),
        &Target::TypeMember {
            ty: Box::new(Target::SelfType),
            member: "odd".into()
        }
    );
}

#[test]
fn exports_are_final_bindings_and_fields_are_declared_once() {
    let (_, resolved) = ok("pub x = 1\npub x = 2");
    let exported: Vec<_> = resolved
        .exports
        .iter()
        .map(|e| (e.name.as_str(), e.span))
        .collect();
    assert_eq!(exported, [("x", Span { start: 14, end: 15 })]);
    // A later private binding un-exports the name.
    let (_, resolved) = ok("pub x = 1\nx = 2\npub y = 3");
    let exported: Vec<_> =
        resolved.exports.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(exported, ["y"]);
    let duplicate = Reason::Duplicate {
        first: Span { start: 9, end: 10 },
    };
    assert_eq!(
        errors("type A { x: I64, x: I64 }"),
        [(duplicate, Some("x"))]
    );
    assert_eq!(
        errors("type S { C(a: I64, a: I64) }")[0].0,
        Reason::Duplicate {
            first: Span { start: 11, end: 12 }
        }
    );
    // A field name may repeat across a sum's constructors.
    ok("type S { C(a: I64), D(a: I64) }");
}

#[test]
fn heads_bind_in_order_and_patterns_resolve_constructors() {
    ok("f = a (b = a) => a + b\nJust x = find ./a\nx\n[:e] = find ./b\ne\nEntry { name, .. } = e\nname\nm: Maybe<I64> = [:1]\nn = { Nothing => 0, Just v => v }");
    assert_eq!(
        errors("f = (b = a) a => b"),
        [(Reason::UnknownName, Some("a"))]
    );
    assert_eq!(errors("Nope x = 1"), [(Reason::UnknownName, Some("Nope"))]);
    assert_eq!(
        errors("Nope { x } = 1"),
        [(Reason::UnknownType, Some("Nope"))]
    );
    assert_eq!(errors("m: Nope = 1"), [(Reason::UnknownType, Some("Nope"))]);
    let (parsed, resolved) = ok("m: Maybe<t> = Nothing");
    let Entry::Line(line) = parsed.entries[0] else {
        panic!()
    };
    let ExprKind::Bind {
        annotation: Some(annotation),
        ..
    } = &parsed.expr(line).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        resolved.ty(*annotation),
        Some(&Target::Type {
            origin: Origin::Prelude,
            name: "Maybe".into()
        })
    );
}

#[test]
fn the_case_rule_duplicates_and_declaration_forms_are_located() {
    assert_eq!(
        errors("type point {}"),
        [(Reason::CaseRule { uppercase: true }, Some("point"))]
    );
    assert_eq!(
        errors("type S { circle(r: F64) }"),
        [(Reason::CaseRule { uppercase: true }, Some("circle"))]
    );
    assert_eq!(
        errors("type P { X: I64 }"),
        [(Reason::CaseRule { uppercase: false }, Some("X"))]
    );
    assert_eq!(
        errors("Point = 1"),
        [(Reason::CaseRule { uppercase: false }, Some("Point"))]
    );
    // A record type and its constructor share the name: one duplicate.
    let duplicate = Reason::Duplicate {
        first: Span { start: 5, end: 6 },
    };
    assert_eq!(
        errors("type A {}\ntype A {}"),
        [(duplicate.clone(), Some("A"))]
    );
    assert_eq!(
        errors("type A { x: I64 }\ntype A { x: I64 }"),
        [(duplicate, Some("A"))]
    );
    for text in [
        "pub len: Vec<a> -> U64",
        "type Rd",
        "x: (a: I64) -> I64 = y => y",
        "type P { x: I64 }\nimpl P { n: I64 }",
    ] {
        assert_eq!(errors(text)[0].0, Reason::DeclarationForm, "{text}");
    }
    ok("f: Show a => a -> a = x => x");
    ok("type T<A> {}");
    assert_eq!(
        errors("type Rd<Api>"),
        [(Reason::DeclarationForm, Some("Rd"))]
    );
    assert_eq!(errors("Self 1"), [(Reason::SelfOutsideBody, Some("Self"))]);
    let (parsed, resolved) = ok("pub type P { pub x: I64, y: I64 = x }\nimpl P { f = Self 1, g = self => self.x }\nP 1 | .g");
    assert_eq!(resolved.exports.len(), 2);
    assert_eq!(resolved.exports[0].name, "P");
    assert!(matches!(
        resolved.exports[1].target,
        Target::Constructor { .. }
    ));
    let text = "pub type P { pub x: I64, y: I64 = x }\nimpl P { f = Self 1, g = self => self.x }\nP 1 | .g";
    assert_eq!(target(text, &parsed, &resolved, "Self"), &Target::SelfType);
    assert!(errors("type P {}\nimpl P { g = self => self }\ng")
        .iter()
        .any(|(reason, _)| *reason == Reason::UnknownName));
}

#[test]
fn dots_classify_modules_types_and_values() {
    let (text, parsed, resolved) = example("05-modules");
    assert_eq!(
        target(&text, &parsed, &resolved, "sys.lookup"),
        &value(Origin::Host("sys".into()), "lookup")
    );
    assert_eq!(
        target(&text, &parsed, &resolved, "fs.size"),
        &value(Origin::Host("fs".into()), "size")
    );
    let Target::Members { head, members } =
        target(&text, &parsed, &resolved, "e.name")
    else {
        panic!()
    };
    assert!(matches!(**head, Target::Local { .. }));
    assert_eq!(members, &["name"]);
    assert_eq!(
        target(&text, &parsed, &resolved, "Polar.from"),
        &Target::TypeMember {
            ty: Box::new(Target::Type {
                origin: Origin::File("geometry".into()),
                name: "Polar".into()
            }),
            member: "from".into()
        }
    );
    assert_eq!(
        target(&text, &parsed, &resolved, "Rd<fs>"),
        &value(Origin::Commands, "Rd")
    );
    assert_eq!(resolved.exports.len(), 2);

    let (parsed, resolved) = ok("fs = 1\nfs.list");
    let Entry::Line(id) = parsed.entries[1] else {
        panic!()
    };
    let Target::Members { head, .. } = resolved.expr(id).unwrap() else {
        panic!()
    };
    assert!(matches!(**head, Target::Local { .. }));

    let (parsed, resolved) = ok("type P { x: I64 }\nimpl P { f = self => self.x }\nimpl P { show = self => show self.x }\nP.f\nP.show\nx: cli.Error = 1\nd = 4 | Rd<fs>");
    let text = "type P { x: I64 }\nimpl P { f = self => self.x }\nimpl P { show = self => show self.x }\nP.f\nP.show\nx: cli.Error = 1\nd = 4 | Rd<fs>";
    assert!(matches!(
        target(text, &parsed, &resolved, "P.show"),
        Target::TypeMember { .. }
    ));
    let source = Source::new(text);
    let mut kinds = Vec::new();
    for entry in &parsed.entries {
        let Entry::Line(id) = entry else { continue };
        if let ExprKind::Bind {
            annotation: Some(a),
            ..
        } = &parsed.expr(*id).unwrap().kind
        {
            kinds.push((
                source.slice(parsed.ty(*a).unwrap().span),
                resolved.ty(*a).cloned(),
            ));
        }
    }
    assert_eq!(
        kinds,
        [(
            Some("cli.Error"),
            Some(Target::Type {
                origin: Origin::Host("cli".into()),
                name: "Error".into()
            })
        )]
    );
    let ExprKind::Bind { value, .. } = &parsed
        .expr(match parsed.entries.last().unwrap() {
            Entry::Line(id) => *id,
            Entry::Item(_) => panic!(),
        })
        .unwrap()
        .kind
    else {
        panic!()
    };
    let ExprKind::Pipe { right, .. } = &parsed.expr(*value).unwrap().kind
    else {
        panic!()
    };
    let ExprKind::Instance { arguments, .. } =
        &parsed.expr(*right).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        resolved.ty(arguments[0]),
        Some(&Target::Module(Origin::Host("fs".into())))
    );
}

#[test]
fn use_forms_and_impl_placement_are_checked() {
    let geometry = ("geometry".to_string(), module("pub type Point { pub x: I64 }\nimpl Point { pub show = self => show self.x }\nhidden = 1\npub shown = 2"));
    let files = [geometry];
    let check =
        |text: &str| -> Result<Resolved, Vec<(Reason, Option<String>)>> {
            let parsed = module(text);
            resolve(&parsed, &host(), &files).map_err(|errors| {
                errors
                    .into_iter()
                    .map(|e| {
                        (
                            e.reason,
                            Source::new(text).slice(e.span).map(str::to_string),
                        )
                    })
                    .collect()
            })
        };
    assert!(check(
        "use geometry.Point geometry.shown\nPoint 1\nshown\nPoint.show"
    )
    .is_ok());
    assert!(
        check("use geometry\ngeometry.Point 1 | show geometry.shown").is_ok()
    );
    assert_eq!(
        check("use nowhere").unwrap_err(),
        [(Reason::UnknownModule, Some("nowhere".into()))]
    );
    assert_eq!(
        check("use geometry.Nope").unwrap_err(),
        [(Reason::NotExported, Some("geometry.Nope".into()))]
    );
    assert_eq!(
        check("use geometry.hidden").unwrap_err(),
        [(Reason::NotExported, Some("geometry.hidden".into()))]
    );
    assert_eq!(
        check("fs.nope 1").unwrap_err(),
        [(Reason::UnknownMember, Some("fs.nope".into()))]
    );
    assert_eq!(
        check("use geometry\ngeometry.display").unwrap_err(),
        [(Reason::UnknownMember, Some("geometry.display".into()))]
    );
    assert_eq!(
        check("use geometry.Point\nPoint.nope").unwrap_err(),
        [(Reason::UnknownMember, Some("Point.nope".into()))]
    );
    assert_eq!(
        check("x: cli.Nope = 1").unwrap_err(),
        [(Reason::UnknownType, Some("cli.Nope".into()))]
    );
    assert_eq!(
        check("impl Vec { len = self => 0 }").unwrap_err(),
        [(Reason::ImplOutsideModule, Some("Vec".into()))]
    );
    assert!(check("type P { x: I64 }\nimpl P { nope = self => 1 }").is_ok());
    assert!(check("type P { x: I64 }\nimpl P { try_from: String -> Result<Self, ParseError> = s => Ok (Self 0) }").is_ok());
    assert!(check("use text 2\nuse text.trim\ntrim \"x\"").is_ok());
}

#[test]
fn private_fields_are_located_outside_their_module() {
    let shapes = (
        "shapes".to_string(),
        module("pub type Point { pub x: I64, pub y: I64 = 0, len: U64 = 0 }\npub type Shape { Circle(r: F64), Rect(w: F64, hidden: F64) }\npub type Written { pub x: I64, secret: I64 = 0 }\nimpl Written { x => Self { x, secret: 1 } }"),
    );
    let files = [shapes];
    let check =
        |text: &str| -> Result<Resolved, Vec<(Reason, Option<String>)>> {
            let parsed = module(text);
            resolve(&parsed, &host(), &files).map_err(|errors| {
                errors
                    .into_iter()
                    .map(|e| {
                        (
                            e.reason,
                            Source::new(text).slice(e.span).map(str::to_string),
                        )
                    })
                    .collect()
            })
        };
    let private = |field: &str| Reason::PrivateField {
        field: field.into(),
    };
    assert_eq!(
        check("use shapes.Point\nPoint { x: 3, y: 4, len: 5 }").unwrap_err(),
        [(private("len"), Some("len".into()))]
    );
    assert_eq!(
        check("use shapes.Point\nPoint { x, len } = Point 1").unwrap_err(),
        [(private("len"), Some("len".into()))]
    );
    assert_eq!(
        check("use shapes.Point\nPoint { x: 3, nope: 1 }").unwrap_err(),
        [(Reason::UnknownField, Some("nope".into()))]
    );
    assert_eq!(
        check("use shapes.Point\nPoint 3 4 5").unwrap_err(),
        [(private("len"), Some("5".into()))]
    );
    assert_eq!(
        check("use shapes.Point\nPoint -len 5 3").unwrap_err(),
        [(private("len"), Some("-len".into()))]
    );
    // A sum constructor's fields are its payload, always visible.
    assert!(check("use shapes.Rect\nRect { w, hidden } = 1\nRect 1 2").is_ok());
    assert_eq!(
        check("use shapes.Rect\nRect { w, nope } = 1").unwrap_err(),
        [(Reason::UnknownField, Some("nope".into()))]
    );
    // A written constructor's spots are its own; a public field is fine.
    assert!(check("use shapes.Point shapes.Written\nPoint 3 4\nPoint -y 4 -x 3\nPoint { x: 1, y: 2 }\nWritten 1 2").is_ok());
    // Inside the module every field is visible.
    assert!(check("type P { pub x: I64, len: U64 = 0 }\nP { x: 1, len: 2 }\nP 1 2\nimpl P { f = self => Self { x: 1, len: 0 } }\nP { x, len } = P 1").is_ok());
    // Two definitions of one (type, name) collide, inherent or trait.
    let dup = check("type P { x: I64 }\nimpl P { show_x = self => 1 }\nimpl P { show_x = self => 2 }").unwrap_err();
    assert!(matches!(dup[0].0, Reason::Duplicate { .. }), "{dup:?}");
    assert!(check("type P { x: I64 }\ntype Q { y: I64 }\nimpl P { f = self => 1 }\nimpl Q { f = self => 2 }").is_ok());
}

#[test]
fn a_use_is_sequential_like_a_binding() {
    // A name a `use` brings is unknown above its line.
    assert_eq!(
        errors("y = lookup\nuse sys.lookup"),
        [(Reason::UnknownName, Some("lookup"))]
    );
    ok("use sys.lookup\ny = lookup");
    let geometry = (
        "geometry".to_string(),
        module("pub type Point { pub x: I64 }\npub shown = 2"),
    );
    let text = "x = geometry.shown\nuse geometry";
    let found = resolve(&module(text), &host(), &[geometry]).unwrap_err();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reason, Reason::UnknownName);
    assert_eq!(
        Source::new(text).slice(found[0].span),
        Some("geometry.shown")
    );
}

/// The reasons and spellings of `resolve` over `text` with `files`.
fn unit_errors(
    text: &str,
    files: &[(&str, &str)],
) -> Vec<(Reason, Option<String>)> {
    let files: Vec<_> = files
        .iter()
        .map(|(stem, text)| ((*stem).to_string(), module(text)))
        .collect();
    resolve(&module(text), &host(), &files).map_or_else(
        |errors| {
            errors
                .into_iter()
                .map(|e| {
                    let spelled = Source::new(text).slice(e.span);
                    (e.reason, spelled.map(str::to_string))
                })
                .collect()
        },
        |_| Vec::new(),
    )
}

#[test]
fn resolve_reports_an_import_cycle_without_a_session() {
    let a = "use b\npub x = 1\n";
    let b = "use a\npub y = 2\n";
    // `a`, which uses `b`, which uses `a`: at the `use` that reaches it.
    assert_eq!(
        unit_errors(a, &[("a", a), ("b", b)]),
        [(Reason::ImportCycle, Some("b".into()))]
    );
    // A module that uses itself, reported once for two `use` lines.
    assert_eq!(
        unit_errors("use s\nuse s.x", &[("s", "use s\npub x = 1\n")]),
        [(Reason::ImportCycle, Some("s".into()))]
    );
    assert_eq!(
        unit_errors("use b", &[("b", "use c\n"), ("c", "x = 1\n")]),
        []
    );
}

#[test]
fn resolve_reports_a_used_module_with_a_main_without_a_session() {
    let script = "pub x = 1\nls\ntype T {}\n";
    assert_eq!(
        unit_errors(
            "use via\nvia.y",
            &[("via", "use script\npub y = 1\n"), ("script", script)]
        ),
        [(Reason::NoMain, Some("via".into()))]
    );
}

const PRIMES: &str = "pub type Prime { pub n: U64 }
impl Prime {
    pub check: Prime -> Bool = self => self.n > 1,
    hidden: Prime -> Prime = self => self,
}
";

#[test]
fn a_module_s_type_names_its_members_through_the_module() {
    let text = "use primes\nprimes.Prime.check";
    let parsed = module(text);
    let files = [("primes".to_string(), module(PRIMES))];
    let resolved = resolve(&parsed, &host(), &files).unwrap();
    assert_eq!(
        target(text, &parsed, &resolved, "primes.Prime.check"),
        &Target::TypeMember {
            ty: Box::new(Target::Type {
                origin: Origin::File("primes".into()),
                name: "Prime".into(),
            }),
            member: "check".into(),
        }
    );
    // Outside its module a non-`pub` inherent member is private, not
    // unknown.
    for text in [
        "use primes\nprimes.Prime.hidden",
        "use primes.Prime\nPrime.hidden",
    ] {
        assert_eq!(
            unit_errors(text, &[("primes", PRIMES)]),
            [(
                Reason::PrivateMember {
                    member: "hidden".into()
                },
                text.lines().nth(1).map(Into::into)
            )],
            "{text}"
        );
    }
    // A member that does not exist at all is still unknown.
    assert_eq!(
        unit_errors("use primes.Prime\nPrime.nope", &[("primes", PRIMES)]),
        [(Reason::UnknownMember, Some("Prime.nope".into()))]
    );
}

/// A form the parser accepts and the checker cannot check yet is
/// one error that names it, never a panic and never silence.
#[test]
fn each_unchecked_form_is_reported_once() {
    use Unchecked as U;
    for text in [
        "pair = (1, 2)",
        "f = (a, b) => a",
        "f: I64 * I64 -> I64 = p => 1",
    ] {
        ok(text);
    }
    for (text, form, spelling) in [
        ("n = 10ms", U::Affix, "10ms"),
        ("r = re\"a+b\"i", U::Affix, "re\"a+b\"i"),
        ("f = n => n | { 0r7 => 1, _ => 0 }", U::Affix, "0r7"),
    ] {
        let reason = Reason::NotYetChecked { form };
        assert_eq!(errors(text), [(reason, Some(spelling))], "{text}");
    }
    for text in [
        "type U8: Integer where self <= 255",
        "type Odd\nwhere self % 2 == 1",
        "type Range { lo: I64 } where 0 <= self.lo",
        "f: I64 & U8 -> I64 = n => n",
        "f: !I64 -> I64 = n => 1",
        "f = n => n | { I64 m => m, _ => 0 }",
    ] {
        ok(text);
    }
    for text in [
        "type Pair<A, B> { first: A, second: B }",
        "f: Vec<8> -> I64 = n => 1",
        "f: [I64; n] -> I64 = a => 1",
        "f: (I64 & U8) * !I64 -> I64 = p => 1",
    ] {
        ok(text);
    }
    // `b"…"` stays bytes until the prelude claims its prefix.
    ok("b = b\"\\xFF\"");
}

/// A lowercase numeric name earns the hint of its uppercase type; a
/// misspelled uppercase one is unknown.
#[test]
fn a_lowercase_numeric_type_earns_its_hint() {
    assert_eq!(
        errors("x: i64 = 1"),
        [(
            Reason::NumericName {
                upper: "I64".into()
            },
            Some("i64")
        )]
    );
    assert_eq!(
        errors("g: I46 -> I46 = x => x"),
        [
            (Reason::UnknownType, Some("I46")),
            (Reason::UnknownType, Some("I46"))
        ]
    );
    ok("f: a -> a = x => x\nn: I64 = 1");
}

#[test]
fn gate_constructor_conflict_points_at_the_first_definition() {
    let parsed = module("type Gate: Integer where self > 0\ntype Sum { Gate }");
    let errors = resolve(&parsed, &host(), &[]).unwrap_err();
    assert!(errors.iter().any(|error| matches!(error.reason, Reason::Duplicate { first } if first.start == 5 && first != error.span)), "{errors:?}");
}

#[test]
fn a_default_may_not_name_the_binding_it_belongs_to() {
    for text in [
        "f = (n = f) => n",
        "h = (n = 1) (m = h) => n + m",
        "a = (n = 1) (m = (a 2)) => n + m",
    ] {
        let found = errors(text);
        assert!(
            found
                .iter()
                .any(|(reason, _)| *reason == Reason::SelfDefault),
            "{text}: {found:?}"
        );
    }
    // A local arm binding is the same case, not an enclosing local.
    assert_eq!(
        errors("g = x => { h = (m = h) => m\n  h 1 }"),
        [(Reason::SelfDefault, Some("h"))]
    );
    // The body may name its binding; a default may name another one.
    ok("f = n => f n");
    ok("k = 5\nf = (m = k) => m");
    ok("f = x => { g = (m = f) => m\n  g 1 }");
}
