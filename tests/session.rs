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
    assert_eq!(file_errors(&report, text), []);
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
    assert_eq!(scheme.to_string(), "Add<b> a => a -> b -> a");
    for (line, ty) in
        [("add 2", "Integer -> Integer"), ("add 2.0", "F64 -> F64")]
    {
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
    assert_eq!(query(&mut session, "add"), "Add<b> a => a -> b -> a");
    assert_eq!(query(&mut session, ".name"), "a.name: b => a -> b");
    assert_eq!(query(&mut session, "(x => x)"), "a -> a");
    assert_eq!(query(&mut session, "[]"), "[a; 0]");
    assert_eq!(query(&mut session, "rr"), "Result<Integer, a>");
    assert_eq!(
        query(&mut session, "add 2"),
        "Add<a> b, Literal b => a -> b"
    );
    assert_eq!(query(&mut session, "1 + 1"), "Integer");
    assert_eq!(
        query(&mut session, "read \"8080\""),
        "Read a => Result<a, ParseError>"
    );
    // A main line cannot choose a meaning from a member alone.
    let report = done(session.line("add"));
    assert!(matches!(
        report.diagnostics.as_slice(),
        [Diagnostic::Type(fipe::infer::Error {
            reason: fipe::infer::Reason::Ambiguous { .. },
            ..
        })]
    ));
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
    assert_eq!(error.span, span(0, 4));
    ok(&mut session, "port = read \"8080\"");
    ok(&mut session, "read<I64> \"8080\"");
}

const BAD: &str = "pub k: I64 = 2
pub f: I64 = True
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
        [bad("True"), bad("1 + \"a\""), bad("nosuch")]
    );
    assert!(matches!(
        report.diagnostics[0],
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
    assert_eq!(query(&mut session, "bad.k"), "I64");
    assert_eq!(query(&mut session, "bad.f"), "I64");
    ok(&mut session, "w1: String = bad.k");
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
    let points = "pub type Point { pub x: I64 }\n";
    let report = done(session.reply(Reply::File(Some(points.into()))));
    assert_eq!(report.diagnostics, []);
    assert_eq!(query(&mut session, "shapes.origin"), "Point");
}

#[test]
fn a_long_flat_chain_does_not_overflow() {
    let mut session = Session::new(host());
    for term in [" + 1", " | (x => x)"] {
        let text = format!("1{}", term.repeat(20_000));
        assert_eq!(query(&mut session, &text), "Integer");
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
    assert_eq!(line_type(&mut session, "g 3"), "Integer");
    assert_eq!(line_type(&mut session, "f \"abc\""), "Integer");
    assert_eq!(query(&mut session, "g"), "Add a, Literal a => a -> a");
    assert_eq!(query(&mut session, "f"), "Len a => a -> Integer");
    // A value, a type and an import are redefined the same way.
    ok(&mut session, "x = 1");
    ok(&mut session, "y = x + 1");
    ok(&mut session, "x = \"one\"");
    assert_eq!(query(&mut session, "y"), "Integer");
    assert_eq!(query(&mut session, "x"), "String");
    ok(&mut session, "type P { x: I64 }");
    ok(&mut session, "p = P 1");
    ok(&mut session, "type P { name: String }");
    assert_eq!(query(&mut session, "p.x"), "I64");
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
    let points = "pub origin: I64 = 0\npub label: String = \"o\"\n";
    done(session.reply(Reply::File(Some(points.into()))));
    ok(&mut session, "o = origin");
    ok(&mut session, "use points.label");
    ok(&mut session, "origin = \"shadowed\"");
    assert_eq!(query(&mut session, "o"), "I64");
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
         even = n => if n == 0 { True } else { Self.odd (n - 1) },\n\
         odd = n => if n == 0 { False } else { Self.even (n - 1) },\n}",
    );
    assert_eq!(line_type(&mut session, "N.even 4"), "Bool");
}

#[test]
fn a_type_and_its_impls_may_be_accepted_on_separate_lines() {
    let mut session = Session::new(host());
    ok(&mut session, "type P2 { x: I64 }");
    ok(&mut session, "impl P2 { dist = self q => self.x - q.x }");
    ok(&mut session, "impl P2 { show = self => show self.x }");
    // The session is one module: a private field is readable.
    assert_eq!(line_type(&mut session, "(P2 1).x"), "I64");
    assert_eq!(line_type(&mut session, "(P2 1).dist (P2 2)"), "Integer");
    // An impl's members generalize at its boundary: the dot on `q`
    // stays an obligation of the scheme.
    assert_eq!(
        query(&mut session, "P2.dist"),
        "a.x: Integer => P2 -> a -> Integer"
    );
    assert_eq!(query(&mut session, "P2.show"), "P2 -> String");
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
    assert_eq!(query(&mut session, "e"), "Integer");
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
    assert_eq!(query(&mut session, "v1999 + v0"), "Integer");
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
    assert_eq!(query(&mut session, "script.x"), "Integer");
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
    let report = done(session.line("b = f True"));
    assert!(matches!(
        type_reasons(&report).as_slice(),
        [fipe::infer::Reason::Mismatch { .. }]
    ));
    assert_eq!(query(&mut session, "f"), "Integer -> Integer");
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
        type_reasons(&done(session.line("h (x => x)"))).as_slice(),
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
    let report = ok(&mut session, "describe: I64 -> String\n  = x => show x");
    assert_eq!(report.entries().len(), 1);
    assert_eq!(query(&mut session, "describe"), "I64 -> String");
}

const PRIMES: &str = "pub type Prime { pub n: U64 }
impl Prime { pub check: Prime -> Bool = self => self.n > 1 }
pub next: Prime -> Prime = p => Prime { n: p.n + 1 }
pub make: U64 -> Prime = n => Prime { n }
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
    assert_eq!(query(&mut session, "2 + 2.5"), "F64");
    let report = done(session.line("next (p.n + 2)"));
    let [Diagnostic::Type(error)] = report.diagnostics.as_slice() else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(error.span, span(6, 13));
}

#[test]
fn conversion_overloads_and_conflicts_span_session_lines() {
    let mut session = Session::new(host());
    ok(&mut session, "type P: From<Integer> & From<String> & TryFrom<Integer, String> { n: Integer }");
    ok(
        &mut session,
        "impl P: From<Integer> { from: Integer -> Self = n => Self n }",
    );
    ok(
        &mut session,
        "impl P: From<String> { from: String -> Self = s => Self 0 }",
    );
    assert_eq!(line_type(&mut session, "1 | to<P>"), "P");
    assert_eq!(line_type(&mut session, "\"s\" | to<P>"), "P");
    let report = done(session.line("impl P: TryFrom<Integer, String> { try_from: Integer -> Result<Self, String> = n => Ok (Self n) }"));
    assert!(report.diagnostics.iter().any(|d| matches!(
        d,
        Diagnostic::Type(fipe::infer::Error {
            reason: fipe::infer::Reason::ConflictingConversion,
            ..
        })
    )));
}

#[test]
fn named_member_folds_span_session_lines() {
    let mut session = Session::new(host());
    ok(&mut session, "type Named where self.name: String");
    ok(&mut session, "f: Named a => a -> String = x => x.name");
    assert_eq!(query(&mut session, "f"), "Named a => a -> String");
}

#[test]
fn documentation_follows_resolved_owners_and_shadowing() {
    let mut session = Session::new(host());
    ok(&mut session, "## Original\nx = 1");
    let report = ok(&mut session, "# retained\nx");
    let fipe::ast::Entry::Line(id) = report.entries()[0] else {
        panic!()
    };
    assert_eq!(session.documentation(&report, id), Some("Original"));
    let comment = report.module.as_ref().unwrap().comments[0];
    assert_eq!(comment.start, report.offset);
    ok(&mut session, "x = 2");
    let report = ok(&mut session, "x");
    let fipe::ast::Entry::Line(id) = report.entries()[0] else {
        panic!()
    };
    assert_eq!(session.documentation(&report, id), None);
    let report = done(session.query("ls"));
    let fipe::ast::Entry::Line(id) = report.entries()[0] else {
        panic!()
    };
    assert!(session
        .documentation(&report, id)
        .unwrap()
        .contains("- depth: how far down to look"));
    let outcome = session.line("use documented.f");
    assert!(matches!(outcome, Outcome::Request(Request::File { .. })));
    let report = done(session.reply(Reply::File(Some(
        "## Imported\npub f: Integer -> Integer = x => x".into(),
    ))));
    assert_eq!(report.diagnostics, []);
    let report = done(session.query("f"));
    let fipe::ast::Entry::Line(id) = report.entries()[0] else {
        panic!()
    };
    assert_eq!(session.documentation(&report, id), Some("Imported"));
}

#[test]
fn program_docs_never_validate_spot_lists() {
    let mut session = Session::new(host());
    ok(&mut session, "## - absent: opaque Markdown\nf = x => x");
    let report = done(session.query("f"));
    let fipe::ast::Entry::Line(id) = report.entries()[0] else {
        panic!()
    };
    assert_eq!(
        session.documentation(&report, id),
        Some("- absent: opaque Markdown")
    );
}

#[test]
fn a_session_claim_holds_only_after_its_block() {
    let mut session = Session::new(host());
    ok(&mut session, "type V: Add { x: F64 }");
    let report = done(session.line("(V 1) + (V 2)"));
    assert!(report.diagnostics.iter().any(|d| matches!(d,Diagnostic::Type(e) if matches!(e.reason,fipe::infer::Reason::MissingClaim { .. }))));
    ok(
        &mut session,
        "impl V: Add { add = other self => Self (self.x + other.x) }",
    );
    ok(&mut session, "(V 1) + (V 2)");
}

#[test]
fn an_unwritten_static_claim_names_the_missing_claim() {
    let mut session = Session::new(host());
    ok(&mut session, "type V: Zero { x: F64 }");
    let report = done(session.line("V.zero"));
    assert!(report.diagnostics.iter().any(|d| matches!(d,Diagnostic::Type(e) if matches!(&e.reason,fipe::infer::Reason::MissingClaim { claim, .. } if claim.to_string() == "Zero"))),"{:?}",report.diagnostics);
    ok(&mut session, "impl V: Zero { zero = Self 0.0 }");
    assert_eq!(line_type(&mut session, "V.zero"), "V");
}

#[test]
fn the_mutually_claimed_example_checks_as_one_session_unit() {
    let mut session = Session::new(host());
    let report = done(
        session.line(include_str!("../Documentation/examples/03-types.fipe")),
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}

#[test]
fn a_shadowing_type_does_not_supply_an_earlier_types_claim() {
    let mut session = Session::new(host());
    ok(&mut session, "type V: Add { x: F64 }");
    ok(&mut session, "old = V 1");
    let report = done(session.line("type V: Add { x: F64 }\nimpl V: Add { add = other self => Self (self.x + other.x) }\nold + old"));
    assert!(report.diagnostics.iter().any(|d| matches!(d,Diagnostic::Type(e) if matches!(e.reason,fipe::infer::Reason::MissingClaim { .. }))),"{:?}",report.diagnostics);
}

#[test]
fn session_plain_and_claim_members_conflict_in_either_order() {
    for (first, second) in [
        (
            "impl W: Zero { zero = Self 0.0 }",
            "impl W { zero = Self 1.0 }",
        ),
        (
            "impl W { zero = Self 1.0 }",
            "impl W: Zero { zero = Self 0.0 }",
        ),
    ] {
        let mut session = Session::new(host());
        ok(&mut session, "type W: Zero { x: F64 }");
        ok(&mut session, first);
        let report = done(session.line(second));
        assert!(report.diagnostics.iter().any(|d| (matches!(d, Diagnostic::Type(e) if matches!(&e.reason, fipe::infer::Reason::ClaimBlock { problem, .. } if problem.contains("plain impl"))) || matches!(d, Diagnostic::Resolve(fipe::resolve::Error { reason: Reason::Duplicate { .. }, .. })))), "{:?}", report.diagnostics);
    }
}

#[test]
fn plain_member_cannot_be_replaced() {
    let mut session = Session::new(host());
    ok(&mut session, "type A { v: String }");
    ok(&mut session, "impl A { get = self => self.v }");
    assert_ne!(
        done(session.line("impl A { get = self => 1 }")).diagnostics,
        []
    );
}

#[test]
fn diagnostics_follow_source_order() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use bad"), Outcome::Request(_)));
    let report = done(session.reply(Reply::File(Some(
        "a: Bool = 1\nx = missing\nb: String = (x => x)".into(),
    ))));
    let spans: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| match d {
            Diagnostic::FileResolve { error, .. } => error.span,
            Diagnostic::FileType { error, .. } => error.span,
            _ => panic!("unexpected diagnostic"),
        })
        .collect();
    assert!(spans.len() >= 3, "{spans:?}");
    assert!(
        spans.windows(2).all(|p| p[0].start <= p[1].start),
        "{spans:?}"
    );
}

#[test]
fn private_field_is_named_through_direct_and_deferred_dots() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use shapes"), Outcome::Request(_)));
    assert_eq!(
        done(session.reply(Reply::File(Some(
            "pub type P { secret: Integer = 0 }\npub p = P {}".into()
        ))))
        .diagnostics,
        []
    );
    for line in ["shapes.p.secret", "get = p => p.secret\nget shapes.p"] {
        let report = done(session.line(line));
        assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::Type(fipe::infer::Error { reason: fipe::infer::Reason::PrivateField { field }, .. }) if field == "secret")), "{:?}", report.diagnostics);
    }
}

#[test]
fn underivable_claim_waits_for_its_session_block() {
    let mut session = Session::new(host());
    ok(&mut session, "type S: Show { f: Integer -> Integer }");
    ok(&mut session, "impl S: Show { show = self => \"s\" }");
    ok(&mut session, "type D: Default { n: Integer }");
    ok(&mut session, "impl D: Default { default = Self 0 }");
}

#[test]
fn imported_field_in_session_keeps_its_type() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use shape"), Outcome::Request(_)));
    let shape =
        "use geo\npub type Box { pub p: geo.Pt }\npub b = Box (geo.Pt 1)";
    assert!(matches!(
        session.reply(Reply::File(Some(shape.into()))),
        Outcome::Request(_)
    ));
    let geo = "pub type Pt { pub x: I64 }";
    let report = done(session.reply(Reply::File(Some(geo.into()))));
    assert_eq!(report.diagnostics, []);
    assert_eq!(query(&mut session, "shape.b.p"), "Pt");
}

#[test]
fn failed_module_bindings_carry_no_proven_facts() {
    let mut session = Session::new(Declarations::new());
    assert!(matches!(session.line("use badnet"), Outcome::Request(_)));
    let report = done(session.reply(Reply::File(Some(
        "pub type Port: Integer where self > 0\npub type Box { pub p: Port }\npub zero: Port = 0\npub boxed = Box 0".into(),
    ))));
    assert_ne!(report.diagnostics, []);
    for source in [
        "y: badnet.Port = badnet.zero",
        "z: badnet.Port = badnet.boxed.p",
    ] {
        let report = done(session.line(source));
        assert!(!report.checks.is_empty(), "{report:?}");
    }
}

#[test]
fn imported_limits_read_host_constants() {
    let mut declarations = Declarations::new();
    declarations
        .module("config", "pub max_port = 65535")
        .unwrap();
    let mut session = Session::new(declarations);
    assert!(matches!(session.line("use net"), Outcome::Request(_)));
    let report = done(session.reply(Reply::File(Some("pub type Port: Integer where self > 0, self <= config.max_port\npub type Conn { pub host: String, pub port: Port = 80 }".into()))));
    assert_eq!(report.diagnostics, []);
    assert_eq!(done(session.line("c = net.Conn \"web\"")).diagnostics, []);
    assert_ne!(done(session.line("p: net.Port = 70000")).diagnostics, []);
}

#[test]
fn module_checks_are_unique_and_owned_by_their_binding() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use net"), Outcome::Request(_)));
    let report =
        done(session.reply(Reply::File(Some("pub p: U8 = cli.port".into()))));
    assert_eq!(report.diagnostics, []);
    let count = session.checks(None).len();
    assert!(count > 0);
    ok(&mut session, "x = net.p");
    assert_eq!(session.checks(None).len(), count);
    assert_eq!(session.checks(Some("x")), []);
    assert_eq!(session.checks(Some("net.p")).len(), count);
}

#[test]
fn supplied_constants_infer_and_enter_their_declared_type() {
    let mut declarations = Declarations::new();
    declarations
        .module("config", "pub answer = 42\npub bad: U8 = 300")
        .unwrap();
    let mut session = Session::new(declarations);
    assert_eq!(query(&mut session, "config.answer"), "Integer");
    assert_ne!(
        done(session.line("x: Bool = config.answer")).diagnostics,
        []
    );
    assert_ne!(done(session.line("config.bad")).diagnostics, []);
}

#[test]
fn private_named_constructor_field_has_one_diagnostic() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use gen"), Outcome::Request(_)));
    assert_eq!(
        done(session.reply(Reply::File(Some(
            "pub type P { pub x: I64, len: I64 = 0 }".into()
        ))))
        .diagnostics,
        []
    );
    assert_eq!(done(session.line("gen.P -len 5 3")).diagnostics.len(), 1);
}
