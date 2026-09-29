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
fn a_hoisted_binding_resolves_in_a_later_line() {
    let mut session = Session::new(host());
    ok(&mut session, "add = a b => a + b");
    let report = ok(&mut session, "add 2");
    assert_eq!(report.entries().len(), 1);
    assert_eq!(
        target(&report, span(0, 3)),
        &Target::Value {
            origin: Origin::This,
            name: "add".into(),
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
    assert_eq!(report.diagnostics, []);
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
