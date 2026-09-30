use fipe::{
    ast::ExprKind,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    resolve::{Origin, Reason, Target},
    session::{Diagnostic, Report, Session},
    token::Span,
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

fn done(outcome: Outcome<Report>) -> Report {
    match outcome {
        Outcome::Done(report) => report,
        Outcome::Request(request) => panic!("unexpected {request:?}"),
    }
}

/// A line that must check.
fn ok(session: &mut Session, text: &str) -> Report {
    let report = done(session.line(text));
    assert_eq!(report.diagnostics, [], "{text:?}");
    report
}

/// The target of the line's name expression at `span`, in line bytes.
fn target(report: &Report, span: Span) -> &Target {
    let module = report.module.as_ref().unwrap();
    let resolved = report.resolved.as_ref().unwrap();
    resolved
        .expressions()
        .find(|(id, _)| {
            let expr = module.expr(*id).unwrap();
            matches!(expr.kind, ExprKind::Name(_))
                && expr.span.start == report.offset + span.start
                && expr.span.end == report.offset + span.end
        })
        .map_or_else(|| panic!("no name at {span:?}"), |(_, target)| target)
}

const fn span(start: usize, end: usize) -> Span {
    Span { start, end }
}

fn unknown_name(report: &Report) -> Vec<Span> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            Diagnostic::Resolve(error)
                if error.reason == Reason::UnknownName =>
            {
                error.span
            }
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn an_arm_binding_resolves_in_a_later_line() {
    let mut session = Session::new(host());
    ok(&mut session, "add = a b => a + b");
    let report = ok(&mut session, "add 2");
    assert_eq!(report.entries().len(), 1);
    // An accepted line's binding is frozen at its site.
    assert_eq!(
        target(&report, span(0, 3)),
        &Target::Local {
            site: Span { start: 0, end: 3 }
        }
    );
}

#[test]
fn an_erroneous_line_is_located_in_its_text_and_not_accepted() {
    let mut session = Session::new(host());
    ok(&mut session, "x = 1");
    let before = session.source().to_string();
    let report = done(session.line("x | nope 1"));
    assert_eq!(unknown_name(&report), [span(4, 8)]);
    assert!(report.resolved.is_none());
    assert_eq!(session.source(), before);
    assert_eq!(unknown_name(&done(session.line("nope 1"))), [span(0, 4)]);
    assert_eq!(unknown_name(&done(session.line("nope"))), [span(0, 4)]);
}

#[test]
fn a_syntax_error_is_located_in_the_line() {
    let mut session = Session::new(host());
    ok(&mut session, "x = 1");
    let report = done(session.line("| x"));
    assert!(report.module.is_none());
    let [Diagnostic::Syntax(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(error.span.start, 0);
    ok(&mut session, "x");
}

#[test]
fn use_requests_a_file_module_once() {
    let mut session = Session::new(host());
    let Outcome::Request(request) = session.line("use geometry.Point") else {
        panic!()
    };
    assert_eq!(
        request,
        Request::File {
            stem: "geometry".into()
        }
    );
    let text = include_str!("../Documentation/examples/geometry.fipe");
    let report = done(session.reply(Reply::File(Some(text.into()))));
    // The file's own errors, `hypot p.x p.y` and `atan2 p.y p.x`, are
    // 07's cast rows; the line itself checks.
    assert_eq!(
        file_errors(&report, text),
        [
            ("geometry".to_string(), "p.x"),
            ("geometry".to_string(), "p.y"),
            ("geometry".to_string(), "p.y"),
            ("geometry".to_string(), "p.x"),
        ]
    );
    assert_eq!(session.source(), "use geometry.Point\n\n");
    let report = ok(&mut session, "Point 1");
    assert_eq!(
        target(&report, span(0, 5)),
        &Target::Constructor {
            origin: Origin::File("geometry".into()),
            ty: "Point".into(),
            name: "Point".into(),
        }
    );
    ok(&mut session, "use geometry.Polar");
}

#[test]
fn an_absent_file_module_is_unknown() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use missing") else {
        panic!()
    };
    let report = done(session.reply(Reply::File(None)));
    let [Diagnostic::Resolve(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(error.reason, Reason::UnknownModule);
    assert_eq!(error.span, span(4, 11));
    assert_eq!(session.source(), "");
    assert!(matches!(session.line("use missing"), Outcome::Request(_)));
}

#[test]
fn an_unparsable_file_module_is_reported_in_its_text() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use broken") else {
        panic!()
    };
    let report = done(session.reply(Reply::File(Some("x = (".into()))));
    assert!(matches!(
        report.diagnostics.as_slice(),
        [
            Diagnostic::File { stem, .. },
            Diagnostic::Resolve(error),
        ] if stem == "broken" && error.reason == Reason::UnknownModule
    ));
}

#[test]
fn a_query_is_not_accepted() {
    let mut session = Session::new(host());
    let report = done(session.query("y = 1"));
    assert_eq!(report.diagnostics, []);
    assert_eq!(session.source(), "");
    assert_eq!(unknown_name(&done(session.line("y"))), [span(0, 1)]);
}

#[test]
fn a_later_binding_shadows() {
    let mut session = Session::new(host());
    ok(&mut session, "x = 1");
    let second = ok(&mut session, "x = 2");
    let report = ok(&mut session, "x");
    let Target::Local { site } = target(&report, span(0, 1)) else {
        panic!()
    };
    assert_eq!(site.start, second.offset);
}

#[test]
fn a_line_carries_its_types() {
    let mut session = Session::new(host());
    let report = ok(&mut session, "add = a b => a + b");
    let [fipe::ast::Entry::Line(id)] = report.entries() else {
        panic!()
    };
    let typed = report.typed.as_ref().unwrap();
    let scheme = typed.binding(*id).unwrap();
    assert_eq!(scheme.to_string(), "Num a => a -> a -> a");
    for (line, ty) in [("add 2", "i64 -> i64"), ("add 2.0", "f64 -> f64")] {
        let report = ok(&mut session, line);
        let [fipe::ast::Entry::Line(id)] = report.entries() else {
            panic!()
        };
        let typed = report.typed.as_ref().unwrap();
        assert_eq!(typed.expr(*id).unwrap().to_string(), ty);
    }
    let report = done(session.line("add \"a\" \"b\""));
    let [Diagnostic::Type(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(error.span, span(0, 3));
    assert!(report.typed.is_none());
}

/// The scheme a query reports for its one line.
fn query(session: &mut Session, text: &str) -> String {
    let report = done(session.query(text));
    assert_eq!(report.diagnostics, [], "{text:?}");
    let [fipe::ast::Entry::Line(id)] = report.entries() else {
        panic!("{text:?}")
    };
    let typed = report.typed.as_ref().unwrap();
    typed.binding(*id).unwrap().to_string()
}

#[test]
fn a_query_reports_a_scheme() {
    let mut session = Session::new(host());
    ok(&mut session, "add = a b => a + b");
    ok(&mut session, "rr = Ok 1");
    assert_eq!(query(&mut session, "add"), "Num a => a -> a -> a");
    assert_eq!(query(&mut session, ".name"), "a.name: b => a -> b");
    assert_eq!(query(&mut session, "(x => x)"), "a -> a");
    assert_eq!(query(&mut session, "[]"), "Vec<a>");
    assert_eq!(query(&mut session, "rr"), "Result<i64, a>");
    assert_eq!(query(&mut session, "add 2"), "Num a => a -> a");
    assert_eq!(query(&mut session, "1 + 1"), "i64");
    assert_eq!(
        query(&mut session, "read \"8080\""),
        "Read a => Result<a, ParseError>"
    );
    // A line of main still defaults where its variable leaves.
    let report = ok(&mut session, "add");
    let [fipe::ast::Entry::Line(id)] = report.entries() else {
        panic!()
    };
    let ty = report.typed.as_ref().unwrap().expr(*id).unwrap();
    assert_eq!(ty.to_string(), "i64 -> i64 -> i64");
}

#[test]
fn a_line_whose_read_target_is_unknown_is_ambiguous() {
    let mut session = Session::new(host());
    let report = done(session.line("read \"8080\""));
    let [Diagnostic::Type(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(
        error.reason,
        fipe::infer::Reason::Ambiguous {
            class: "Read".into()
        }
    );
    assert_eq!(error.span, span(0, 11));
    ok(&mut session, "port = read \"8080\"");
    ok(&mut session, "read<i64> \"8080\"");
}

const BAD: &str = "pub k: i64 = 2
pub f: i64 = \"x\"
pub g = 1 + \"a\"
pub h = nosuch 1
";

/// Each file diagnostic of a report: (stem, spelling in `text`).
fn file_errors<'a>(report: &Report, text: &'a str) -> Vec<(String, &'a str)> {
    let source = fipe::token::Source::new(text);
    report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let (stem, span) = match diagnostic {
                Diagnostic::File { stem, error } => (stem, error.span),
                Diagnostic::FileResolve { stem, error } => (stem, error.span),
                Diagnostic::FileType { stem, error } => (stem, error.span),
                _ => return None,
            };
            Some((stem.clone(), source.slice(span).unwrap()))
        })
        .collect()
}

#[test]
fn a_loaded_file_is_checked_once_and_its_values_are_typed() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use bad") else {
        panic!()
    };
    let report = done(session.reply(Reply::File(Some(BAD.into()))));
    let bad = |text: &'static str| ("bad".to_string(), text);
    assert_eq!(
        file_errors(&report, BAD),
        [bad("nosuch"), bad("\"x\""), bad("\"a\"")]
    );
    assert!(matches!(
        report.diagnostics[1],
        Diagnostic::FileType {
            error: fipe::infer::Error {
                reason: fipe::infer::Reason::Mismatch { .. },
                ..
            },
            ..
        }
    ));
    // The file's errors are not the line's: the line is accepted.
    assert_eq!(report.diagnostics.len(), 3);
    assert_eq!(session.source(), "use bad\n\n");
    assert_eq!(query(&mut session, "bad.k"), "i64");
    assert_eq!(query(&mut session, "bad.f"), "i64");
    let report = done(session.line("w1: String = bad.k"));
    assert!(matches!(
        report.diagnostics.as_slice(),
        [Diagnostic::Type(fipe::infer::Error {
            reason: fipe::infer::Reason::Mismatch { .. },
            ..
        })]
    ));
    // Loaded once: a later line that reaches the file reports nothing.
    ok(&mut session, "k2 = bad.k + 1");
}

#[test]
fn a_loaded_file_s_own_use_is_requested() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use shapes") else {
        panic!()
    };
    let shapes = "use points.Point\npub origin = Point 0\n";
    let Outcome::Request(Request::File { stem }) =
        session.reply(Reply::File(Some(shapes.into())))
    else {
        panic!()
    };
    assert_eq!(stem, "points");
    let points = "pub type Point { pub x: i64 }\n";
    let report = done(session.reply(Reply::File(Some(points.into()))));
    assert_eq!(report.diagnostics, []);
    assert_eq!(query(&mut session, "shapes.origin"), "Point");
}

#[test]
fn a_long_flat_chain_does_not_overflow() {
    let mut session = Session::new(host());
    for term in [" + 1", " | (x => x)"] {
        let text = format!("1{}", term.repeat(20_000));
        assert_eq!(query(&mut session, &text), "i64");
    }
}

/// The type of a line's one expression, or a query's scheme.
fn line_type(session: &mut Session, text: &str) -> String {
    let report = ok(session, text);
    let [fipe::ast::Entry::Line(id)] = report.entries() else {
        panic!("{text:?}")
    };
    report
        .typed
        .as_ref()
        .unwrap()
        .expr(*id)
        .unwrap()
        .to_string()
}

fn only_resolve(report: &Report) -> (Reason, Span) {
    let [Diagnostic::Resolve(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    (error.reason.clone(), error.span)
}

#[test]
fn an_accepted_line_is_frozen_and_a_redefinition_shadows_after_it() {
    let mut session = Session::new(host());
    ok(&mut session, "f = n => n + 1");
    ok(&mut session, "g = n => f n");
    ok(&mut session, "f = s => s | len");
    assert_eq!(line_type(&mut session, "g 3"), "i64");
    assert_eq!(line_type(&mut session, "f \"abc\""), "u64");
    assert_eq!(query(&mut session, "g"), "Num a => a -> a");
    assert_eq!(query(&mut session, "f"), "Len a => a -> u64");
    // A value, a type and an import are redefined the same way.
    ok(&mut session, "x = 1");
    ok(&mut session, "y = x + 1");
    ok(&mut session, "x = \"one\"");
    assert_eq!(query(&mut session, "y"), "i64");
    assert_eq!(query(&mut session, "x"), "String");
    ok(&mut session, "type P { x: i64 }");
    ok(&mut session, "p = P 1");
    ok(&mut session, "type P { name: String }");
    assert_eq!(query(&mut session, "p.x"), "i64");
    assert_eq!(query(&mut session, "(P \"a\").name"), "String");
    let report = done(session.query("(P \"a\").x"));
    assert!(matches!(
        report.diagnostics.as_slice(),
        [Diagnostic::Type(fipe::infer::Error {
            reason: fipe::infer::Reason::NoField { .. },
            ..
        })]
    ));
    let Outcome::Request(_) = session.line("use points.origin") else {
        panic!()
    };
    let points = "pub origin: i64 = 0\npub label: String = \"o\"\n";
    done(session.reply(Reply::File(Some(points.into()))));
    ok(&mut session, "o = origin");
    ok(&mut session, "use points.label");
    ok(&mut session, "origin = \"shadowed\"");
    assert_eq!(query(&mut session, "o"), "i64");
    assert_eq!(query(&mut session, "origin"), "String");
}

#[test]
fn a_line_sees_no_later_line() {
    let mut session = Session::new(host());
    let report = done(
        session.line("even = n => if n == 0 { True } else { odd (n - 1) }"),
    );
    assert_eq!(only_resolve(&report), (Reason::UnknownName, span(38, 41)));
    let report = done(session.query("later"));
    assert_eq!(only_resolve(&report), (Reason::UnknownName, span(0, 5)));
    // Within one line the rules are a file's: bindings are sequential.
    let report = done(session.line(
        "even = n => if n == 0 { True } else { odd (n - 1) }\n\
         odd = n => if n == 0 { False } else { even (n - 1) }",
    ));
    assert_eq!(only_resolve(&report), (Reason::UnknownName, span(38, 41)));
    // Mutual recursion is written as the members of one impl.
    ok(&mut session, "type N {}");
    ok(
        &mut session,
        "impl N {\n\
         even = n => if n == 0 { True } else { odd (n - 1) },\n\
         odd = n => if n == 0 { False } else { even (n - 1) },\n}",
    );
    assert_eq!(line_type(&mut session, "N.even 4"), "Bool");
}

#[test]
fn a_type_and_its_impls_may_be_accepted_on_separate_lines() {
    let mut session = Session::new(host());
    ok(&mut session, "type P2 { x: i64 }");
    ok(&mut session, "impl P2 { dist = self q => self.x - q.x }");
    ok(
        &mut session,
        "impl P2: Display { display = self => show self.x }",
    );
    // The session is one module: a private field is readable.
    assert_eq!(line_type(&mut session, "(P2 1).x"), "i64");
    assert_eq!(line_type(&mut session, "(P2 1).dist (P2 2)"), "i64");
    // An impl's members generalize at its boundary: the dot on `q`
    // stays an obligation of the scheme.
    assert_eq!(query(&mut session, "P2.dist"), "a.x: i64 => P2 -> a -> i64");
    assert_eq!(query(&mut session, "P2.display"), "P2 -> String");
    let report = ok(&mut session, "P2 { x: 3 }");
    assert_eq!(
        report.offset,
        session.source().len() - "P2 { x: 3 }\n\n".len()
    );
}

#[test]
fn spans_stay_in_the_session_s_source() {
    let mut session = Session::new(host());
    ok(&mut session, "a = 1");
    let report = ok(&mut session, "b = a");
    assert_eq!(report.offset, "a = 1\n\n".len());
    assert_eq!(session.source(), "a = 1\n\nb = a\n\n");
    let Target::Local { site } = target(&report, span(4, 5)) else {
        panic!()
    };
    assert_eq!(*site, span(0, 1));
    // A resolve query of a name reaches the frame too.
    let report = done(session.query("b"));
    assert!(matches!(target(&report, span(0, 1)), Target::Local { .. }));
}

#[test]
fn retract_removes_the_last_accepted_line() {
    let mut session = Session::new(host());
    ok(&mut session, "e = 0");
    ok(&mut session, "f = 1");
    assert_eq!(session.retract().as_deref(), Some("f = 1"));
    assert_eq!(session.source(), "e = 0\n\n");
    assert_eq!(unknown_name(&done(session.line("f"))), [span(0, 1)]);
    ok(&mut session, "e");
    // A redefinition retracted uncovers the earlier one.
    ok(&mut session, "e = \"text\"");
    session.retract();
    assert_eq!(query(&mut session, "e"), "i64");
    assert_eq!(session.retract().as_deref(), Some("e"));
    assert_eq!(session.retract().as_deref(), Some("e = 0"));
    assert_eq!(session.retract(), None);
    assert_eq!(session.source(), "");
}

#[test]
fn a_line_costs_the_same_after_many_accepted_lines() {
    let mut session = Session::new(host());
    for n in 0..2_000 {
        ok(&mut session, &format!("v{n} = {n} + 1"));
    }
    assert_eq!(query(&mut session, "v1999 + v0"), "i64");
}

/// Each file diagnostic's stem and reason, resolve errors only.
fn file_reasons(report: &Report) -> Vec<(String, Reason)> {
    report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            Diagnostic::FileResolve { stem, error } => {
                Some((stem.clone(), error.reason.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_use_that_reaches_its_own_module_is_an_import_cycle() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use a") else {
        panic!()
    };
    let a = "use b\npub x = 1\n";
    let Outcome::Request(Request::File { stem }) =
        session.reply(Reply::File(Some(a.into())))
    else {
        panic!()
    };
    assert_eq!(stem, "b");
    let b = "use a\npub y = 2\n";
    let report = done(session.reply(Reply::File(Some(b.into()))));
    // Reported once, in `b`, whose `use a` closes the cycle.
    assert_eq!(file_reasons(&report), [("b".into(), Reason::ImportCycle)]);
    let Diagnostic::FileResolve { error, .. } = &report.diagnostics[0] else {
        panic!()
    };
    assert_eq!(error.span, span(4, 5));
    // A module that uses itself.
    let Outcome::Request(_) = session.line("use self_") else {
        panic!()
    };
    let report = done(session.reply(Reply::File(Some("use self_\n".into()))));
    assert_eq!(
        file_reasons(&report),
        [("self_".into(), Reason::ImportCycle)]
    );
}

#[test]
fn a_used_module_has_no_main() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use script") else {
        panic!()
    };
    let script = "pub x = 1\nls\ntype T {}\n";
    let report = done(session.reply(Reply::File(Some(script.into()))));
    assert_eq!(file_reasons(&report), [("script".into(), Reason::NoMain)]);
    let Diagnostic::FileResolve { error, .. } = &report.diagnostics[0] else {
        panic!()
    };
    assert_eq!(error.span, span(10, 12));
    // The line itself checks; the module's bindings are usable.
    assert_eq!(query(&mut session, "script.x"), "i64");
}

#[test]
fn a_module_exports_its_final_bindings() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use m") else {
        panic!()
    };
    let m = "pub x = 1\npub x = \"two\"\npub y = 1\ny = 2\n";
    done(session.reply(Reply::File(Some(m.into()))));
    assert_eq!(query(&mut session, "m.x"), "String");
    let report = done(session.query("m.y"));
    assert_eq!(only_resolve(&report), (Reason::UnknownMember, span(0, 3)));
}

#[test]
fn a_value_reached_through_an_import_cycle_has_no_type() {
    let mut session = Session::new(host());
    let Outcome::Request(_) = session.line("use a") else {
        panic!()
    };
    let a = "use b\npub x = b.y\n";
    let Outcome::Request(_) = session.reply(Reply::File(Some(a.into()))) else {
        panic!()
    };
    let b = "use a\npub y = a.x\n";
    done(session.reply(Reply::File(Some(b.into()))));
    // Not a fresh variable that would type `a.x` as anything.
    let report = done(session.query("a.x"));
    let cycle = report.diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, Diagnostic::Type(error)
            if error.reason == fipe::infer::Reason::ImportCycle
                && error.span == span(0, 3))
    });
    assert!(cycle, "{:?}", report.diagnostics);
}

/// The reasons of a line's type diagnostics.
fn type_reasons(report: &Report) -> Vec<fipe::infer::Reason> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            Diagnostic::Type(error) => error.reason.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn a_pattern_bound_binding_stays_monomorphic_across_lines() {
    // As in a file: the first use fixes `f`, the second fails.
    let mut session = Session::new(host());
    ok(&mut session, "[f] = [(x => x)]");
    ok(&mut session, "a = f 1");
    let report = done(session.line("b = f \"s\""));
    assert!(matches!(
        type_reasons(&report).as_slice(),
        [fipe::infer::Reason::Mismatch { .. }]
    ));
    assert_eq!(query(&mut session, "f"), "i64 -> i64");
    // Retracting the line that fixed it frees it again.
    assert_eq!(session.retract().as_deref(), Some("a = f 1"));
    ok(&mut session, "b = f \"s\"");
    // A binding over it shares its variable; a constructor pattern too.
    let mut session = Session::new(host());
    ok(&mut session, "Just g = Just (x => x)");
    ok(&mut session, "h = y => g y");
    ok(&mut session, "c = h \"s\"");
    assert_eq!(query(&mut session, "g"), "String -> String");
    assert!(matches!(
        type_reasons(&done(session.line("h 1"))).as_slice(),
        [fipe::infer::Reason::Mismatch { .. }, ..]
    ));
    // A generalized binding still freezes as a scheme.
    ok(&mut session, "id = x => x");
    ok(&mut session, "d = id 1");
    ok(&mut session, "e = id \"s\"");
}

#[test]
fn a_signature_above_its_value_is_one_line() {
    let mut session = Session::new(host());
    let report = ok(&mut session, "describe: i64 -> String\n  = x => show x");
    assert_eq!(report.entries().len(), 1);
    assert_eq!(query(&mut session, "describe"), "i64 -> String");
}

const PRIMES: &str = "pub type Prime { pub n: u64 }
impl Prime { pub check: Prime -> Bool = self => self.n > 1 }
pub next: Prime -> Prime = p => Prime { n: p.n + 1 }
pub make: u64 -> Prime = n => Prime { n }
";

/// A session with `primes` loaded by `use primes.Prime primes.next
/// primes.make`.
fn primes() -> Session {
    let mut session = Session::new(host());
    let Outcome::Request(_) =
        session.line("use primes.Prime primes.next primes.make")
    else {
        panic!()
    };
    let report = done(session.reply(Reply::File(Some(PRIMES.into()))));
    assert_eq!(report.diagnostics, []);
    session
}

#[test]
fn a_value_of_a_module_s_type_keeps_it_across_lines() {
    let mut session = primes();
    ok(&mut session, "p = make 3");
    assert_eq!(line_type(&mut session, "next p"), "Prime");
    assert_eq!(line_type(&mut session, "next (next p)"), "Prime");
    ok(&mut session, "q = Prime 7");
    assert_eq!(line_type(&mut session, "next q"), "Prime");
    // The line's own type of that name is not the module's.
    ok(&mut session, "type Prime { s: String }");
    let report = done(session.line("next (Prime \"x\")"));
    assert!(matches!(
        type_reasons(&report).as_slice(),
        [fipe::infer::Reason::Mismatch { .. }]
    ));
    // `q` was made before: still the module's.
    assert_eq!(line_type(&mut session, "next q"), "Prime");
}

#[test]
fn a_module_s_type_members_answer_queries() {
    let mut session = primes();
    assert_eq!(query(&mut session, "Prime.check"), "Prime -> Bool");
    ok(&mut session, "p = make 3");
    assert_eq!(query(&mut session, "p.check"), "Bool");
    assert_eq!(query(&mut session, "2 + 2.5"), "f64");
    let report = done(session.line("next (p.n + 2)"));
    let [Diagnostic::Type(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(error.span, span(6, 9));
}
