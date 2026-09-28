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

#[test]
fn the_first_two_examples_resolve() {
    for path in ["01-files.fipe", "02-outcomes.fipe"] {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Documentation/examples")
                .join(path),
        )
        .unwrap();
        let (parsed, resolved) = ok(&text);
        assert!(resolved.exports.is_empty(), "{path}");
        if path == "01-files.fipe" {
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
