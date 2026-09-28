use fipe::{
    ast::{Entry, ExprKind, Module},
    decl::Declarations,
    lex::lex,
    parse::parse,
    resolve::{resolve, Error, Origin, Reason, Resolved, Target},
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
            assert_eq!(
                target(&text, &parsed, &resolved, "older"),
                &value(Origin::This, "older")
            );
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
                target(&text, &parsed, &resolved, "read<u16>"),
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
fn file_level_arm_bindings_hoist_and_others_follow_their_line() {
    let (parsed, resolved) = ok("f 1\nf = a => a + 1\ng = 2\ng");
    let Entry::Line(call) = parsed.entries[0] else {
        panic!()
    };
    let ExprKind::Call { callee, .. } = &parsed.expr(call).unwrap().kind else {
        panic!()
    };
    assert_eq!(resolved.expr(*callee), Some(&value(Origin::This, "f")));
    assert_eq!(errors("g 1\ng = 2"), [(Reason::UnknownName, Some("g"))]);
    let dup = errors("f = a => 1\nf = b => 2");
    assert_eq!(
        dup[0].0,
        Reason::Duplicate {
            first: Span { start: 0, end: 1 }
        }
    );
    assert_eq!(dup[0].1, Some("f"));
    // Inside a block nothing hoists.
    assert_eq!(
        errors("{ h 1\n h = a => a }"),
        [(Reason::UnknownName, Some("h"))]
    );
}

#[test]
fn heads_bind_in_order_and_patterns_resolve_constructors() {
    ok("f = a (b = a) => a + b\nJust x = find ./a\nx\n[:e] = find ./b\ne\nEntry { name, .. } = e\nname\nm: Maybe<i64> = [:1]\nn = { Nothing => 0, Just v => v }");
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
        errors("type S { circle(r: f64) }"),
        [(Reason::CaseRule { uppercase: true }, Some("circle"))]
    );
    assert_eq!(
        errors("type P { X: i64 }"),
        [(Reason::CaseRule { uppercase: false }, Some("X"))]
    );
    assert_eq!(
        errors("Point = 1"),
        [(Reason::CaseRule { uppercase: false }, Some("Point"))]
    );
    let errors_ = errors("type A {}\ntype A {}");
    assert_eq!(
        errors_[0].0,
        Reason::Duplicate {
            first: Span { start: 5, end: 6 }
        }
    );
    for text in [
        "pub len: Vec<a> -> u64",
        "type Rd<api>",
        "type T<a> {}",
        "f: Show a => a -> a = x => x",
        "x: (a: i64) -> i64 = y => y",
    ] {
        assert_eq!(errors(text)[0].0, Reason::DeclarationForm, "{text}");
    }
    assert_eq!(errors("Self 1"), [(Reason::SelfOutsideBody, Some("Self"))]);
    let (parsed, resolved) = ok("pub type P { pub x: i64, y: i64 = x }\nimpl P { f = Self 1, g = self => self.x }\nP 1 | g");
    assert_eq!(resolved.exports.len(), 2);
    assert_eq!(resolved.exports[0].name, "P");
    assert!(matches!(
        resolved.exports[1].target,
        Target::Constructor { .. }
    ));
    let text = "pub type P { pub x: i64, y: i64 = x }\nimpl P { f = Self 1, g = self => self.x }\nP 1 | g";
    assert_eq!(target(text, &parsed, &resolved, "Self"), &Target::SelfType);
    assert_eq!(
        target(text, &parsed, &resolved, "g"),
        &value(Origin::This, "g")
    );
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

    let (parsed, resolved) = ok("type P { x: i64 }\nimpl P { f = self => self.x }\nimpl P: Display { display = self => show self.x }\nP.f\nP.display\nx: cli.Error = 1\nd = 4 | Rd<fs>");
    let text = "type P { x: i64 }\nimpl P { f = self => self.x }\nimpl P: Display { display = self => show self.x }\nP.f\nP.display\nx: cli.Error = 1\nd = 4 | Rd<fs>";
    assert!(matches!(
        target(text, &parsed, &resolved, "P.display"),
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
    let geometry = ("geometry".to_string(), module("pub type Point { pub x: i64 }\nimpl Point: Display { display = self => show self.x }\nhidden = 1\npub shown = 2"));
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
        "use geometry.Point geometry.shown\nPoint 1\nshown\nPoint.display"
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
        check("impl Vec: Len {}").unwrap_err(),
        [(Reason::ImplOutsideModule, Some("Vec".into()))]
    );
    assert_eq!(
        check("type P { x: i64 }\nimpl P: Display { nope = self => 1 }")
            .unwrap_err(),
        [(Reason::UnknownMember, Some("nope".into()))]
    );
    assert!(check("type P { x: i64 }\nimpl P: TryFrom<String> { Error = ParseError, try_from = s => Self.Error }").is_ok());
    assert!(check("use text 2\nuse text.trim\ntrim \"x\"").is_ok());
}

#[test]
fn private_fields_are_located_outside_their_module() {
    let shapes = (
        "shapes".to_string(),
        module("pub type Point { pub x: i64, pub y: i64 = 0, len: u64 = 0 }\npub type Shape { Circle(r: f64), Rect(w: f64, hidden: f64) }\npub type Written { pub x: i64, secret: i64 = 0 }\nimpl Written { x => Self { x, secret: 1 } }"),
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
    assert!(check("type P { pub x: i64, len: u64 = 0 }\nP { x: 1, len: 2 }\nP 1 2\nimpl P { f = self => Self { x: 1, len: 0 } }\nP { x, len } = P 1").is_ok());
    // Two definitions of one (type, name) collide, inherent or trait.
    let dup = check("type P { x: i64 }\nimpl P { show_x = self => 1 }\nimpl P: Display { show_x = self => 2 }").unwrap_err();
    assert!(matches!(dup[0].0, Reason::Duplicate { .. }), "{dup:?}");
    let dup = check("type P { x: i64 }\ntype Q { y: i64 }\nimpl P { f = self => 1 }\nimpl Q { f = self => 2 }").unwrap_err();
    assert!(matches!(dup[0].0, Reason::Duplicate { .. }), "{dup:?}");
}
