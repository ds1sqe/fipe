use fipe::{
    ast::{Entry, ExprKind},
    compile,
    decl::Declarations,
    embed::{Outcome, Reply, Request},
    resolve::{Origin, Reason, Target},
    session::{Diagnostic, Report, Session},
    token::Span,
    types::Type,
    value::Value,
    vm::Machine,
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

const FILE_IDENTITY_SOURCE: &str = "pub type Point { pub x: Integer }\n\
pub first: Integer -> Point = x => Point x\n\
pub second: Integer -> Point = x => Point x\n";

fn run_file_identity_line(
    session: &mut Session,
    machine: &mut Machine,
    text: &str,
    loads: &mut usize,
) -> (Report, Value) {
    let mut outcome = session.line(text);
    let report = loop {
        match outcome {
            Outcome::Done(report) => break report,
            Outcome::Request(Request::File { stem }) => {
                assert!(["geometry", "other"].contains(&stem.as_str()));
                *loads += 1;
                outcome = session
                    .reply(Reply::File(Some(FILE_IDENTITY_SOURCE.into())));
            }
            other @ Outcome::Request(_) => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(report.diagnostics, [], "{text}");
    let program = compile::compile(report.unit.as_ref().unwrap());
    let Outcome::Done(Ok(value)) = machine.run(&program) else {
        panic!("failed run: {text}")
    };
    (report, value)
}

fn file_record_identity(report: &Report, value: &Value) -> (usize, String) {
    let Value::Record(record) = value else {
        panic!("expected record: {value:?}")
    };
    let tables = report.unit.as_ref().unwrap().tables();
    let meaning = &tables.meanings[record.meaning];
    (
        record.meaning,
        format!("{}.{}", tables.namespaces[meaning.namespace], meaning.label),
    )
}

fn assert_file_function_results(session: &mut Session) {
    let signatures = done(session.query("(geometry.first, geometry.second)"));
    assert_eq!(signatures.diagnostics, []);
    let Entry::Line(id) = signatures.entries()[0] else {
        panic!()
    };
    let Type::Tuple(functions) =
        signatures.typed.as_ref().unwrap().expr(id).unwrap()
    else {
        panic!()
    };
    assert_eq!(functions.len(), 2);
    for function in functions {
        let Type::Function { result, .. } = function else {
            panic!()
        };
        let Type::Named { name, .. } = &**result else {
            panic!()
        };
        assert_eq!(name.origin, Origin::File("geometry".into()));
        assert_eq!(name.name, "Point");
    }
}

#[test]
fn file_module_record_identity_is_independent_of_session_lines() {
    for prefix in [
        &[][..],
        &[][..],
        &["warmup = 7"][..],
        &["type Point { label: String }", "local = Point \"local\""][..],
    ] {
        let mut session = Session::new(Declarations::new());
        let mut machine = Machine::new(Some(100_000));
        let mut loads = 0;
        for text in prefix {
            run_file_identity_line(
                &mut session,
                &mut machine,
                text,
                &mut loads,
            );
        }
        for _ in 0..2 {
            run_file_identity_line(
                &mut session,
                &mut machine,
                "use geometry",
                &mut loads,
            );
        }
        assert_file_function_results(&mut session);
        let (report, value) = run_file_identity_line(
            &mut session,
            &mut machine,
            "(geometry.first 1, geometry.second 2)",
            &mut loads,
        );
        let Value::Tuple(values) = value else {
            panic!()
        };
        assert_eq!(values.len(), 2);
        let first = file_record_identity(&report, &values[0]);
        assert_eq!(first.1, "geometry.Point");
        assert_eq!(file_record_identity(&report, &values[1]), first);
        for (value, expected) in values.iter().zip([1, 2]) {
            let Value::Record(record) = value else {
                panic!()
            };
            assert_eq!(record.fields, [Value::Int(expected)]);
        }
        let (report, value) = run_file_identity_line(
            &mut session,
            &mut machine,
            "geometry.second 3",
            &mut loads,
        );
        assert_eq!(file_record_identity(&report, &value), first);
        assert_eq!(loads, 1);
        run_file_identity_line(
            &mut session,
            &mut machine,
            "use other",
            &mut loads,
        );
        let (report, value) = run_file_identity_line(
            &mut session,
            &mut machine,
            "other.first 4",
            &mut loads,
        );
        let other = file_record_identity(&report, &value);
        assert_eq!(other.1, "other.Point");
        assert_ne!(other.0, first.0);
        assert_eq!(loads, 2);
    }

    for prefix in [&[][..], &["warmup = 7"][..]] {
        let mut session = Session::new(Declarations::new());
        let mut machine = Machine::new(Some(100_000));
        let mut loads = 0;
        for text in prefix {
            run_file_identity_line(
                &mut session,
                &mut machine,
                text,
                &mut loads,
            );
        }
        run_file_identity_line(
            &mut session,
            &mut machine,
            FILE_IDENTITY_SOURCE,
            &mut loads,
        );
        let (report, value) = run_file_identity_line(
            &mut session,
            &mut machine,
            "first 5",
            &mut loads,
        );
        assert_eq!(
            file_record_identity(&report, &value).1,
            format!("line {}.Point", prefix.len() + 1)
        );
        assert_eq!(loads, 0);
    }
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
    // Each stage sits one above the last and above its operand: `1` is
    // one deep, `(x => x)` three.
    let deepest = fipe::parse::MAX_DEPTH;
    for (term, operand) in [(" + 1", 1), (" | (x => x)", 3)] {
        let text = format!("1{}", term.repeat(deepest - operand));
        assert_eq!(query(&mut session, &text), "Integer");
        // Past the bound the line is a syntax error, never an overflow.
        let report = done(session.query(&format!("1{}", term.repeat(20_000))));
        let [Diagnostic::Syntax(error)] = report.diagnostics.as_slice() else {
            panic!("{:?}", report.diagnostics)
        };
        assert_eq!(error.reason, fipe::parse::Reason::DepthLimit);
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
fn a_session_is_rebuilt_from_its_lines() {
    let mut session = Session::new(host());
    ok(&mut session, "e = 0");
    ok(&mut session, "e = \"text\"\nf = e");
    let lines: Vec<String> = session.lines().map(String::from).collect();
    assert_eq!(lines, ["e = 0", "e = \"text\"\nf = e"]);
    // A host that edits an accepted line starts over from the lines
    // before it; the result is the session those lines make.
    let mut rebuilt = Session::new(host());
    ok(&mut rebuilt, &lines[0]);
    assert_eq!(rebuilt.source(), "e = 0\n\n");
    assert_eq!(query(&mut rebuilt, "e"), "Integer");
    assert_eq!(unknown_name(&done(rebuilt.line("f"))), [span(0, 1)]);
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
    // Without the line that fixed it, the variable is still free.
    let mut session = Session::new(host());
    ok(&mut session, "[f] = [(x => x)]");
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

#[test]
fn missing_transitive_imports_retry_only_for_written_uses() {
    let mut session = Session::new(host());
    assert!(matches!(session.line("use outer"), Outcome::Request(_)));
    assert!(
        matches!(session.reply(Reply::File(Some("use missing\npub n = 1".into()))), Outcome::Request(Request::File { stem }) if stem == "missing")
    );
    let report = done(session.reply(Reply::File(None)));
    assert_ne!(report.diagnostics, []);
    for _ in 0..10 {
        ok(&mut session, "1");
    }
    assert!(
        matches!(session.line("use outer"), Outcome::Request(Request::File { stem }) if stem == "missing")
    );
    done(session.reply(Reply::File(Some("pub n = 2".into()))));
    ok(&mut session, "use outer");
}

#[test]
fn weak_meaning_metadata_keeps_the_session_slot() {
    let mut session = Session::new(host());
    let first = done(session.line("[empty] = [[]]"));
    assert_eq!(first.diagnostics, []);
    let first = first.typed.unwrap();
    assert_eq!(
        first
            .session_variables
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [0]
    );
    let second = done(session.line("empty"));
    assert_eq!(second.diagnostics, []);
    let second = second.typed.unwrap();
    // The earlier weak variable keeps its number; only a new one is listed.
    assert!(second.session_variables.is_empty());
    let unit = ok(&mut session, "fixed: [Integer; 0] = empty")
        .unit
        .unwrap();
    assert!(unit.tables().session_meanings[0].is_some());
}

#[test]
fn later_lines_record_receivers_for_earlier_generic_members() {
    let mut session = Session::new(Declarations::new());
    ok(&mut session, "inc = n => n + 1");
    let report = ok(&mut session, "inc 2.0");
    let typed = report.typed.unwrap();
    let dispatch = typed.dispatch.iter().find(|d| d.name == "add").unwrap();
    assert!(dispatch
        .rows
        .iter()
        .any(|row| row.receiver.to_string() == "F64"));
}

#[test]
fn indexed_session_meanings_follow_accept_and_query() {
    let mut session = Session::new(host());
    let unit = ok(&mut session, "[empty] = [[]]").unit.unwrap();
    assert_eq!(unit.tables().session_meanings, [None]);
    let nodes = unit.tables().nodes.len();
    let queried = done(session.query("fixed: [Integer; 0] = empty"));
    assert_eq!(queried.diagnostics, []);
    assert_eq!(unit.tables().session_meanings, [None]);
    assert_eq!(unit.tables().nodes.len(), nodes);
    let next = ok(&mut session, "fixed: [Integer; 0] = empty")
        .unit
        .unwrap();
    assert_eq!(unit.tables().nodes.len(), next.tables().nodes.len());
    let tables = unit.tables();
    let Some(fipe::ir::Type::Meaning { index, .. }) =
        &tables.session_meanings[0]
    else {
        panic!("fixed meaning")
    };
    assert_eq!(tables.meanings[*index].label, "Integer");
}

#[test]
fn indexed_dispatch_rows_follow_accepted_lines_not_queries() {
    let mut session = Session::new(Declarations::new());
    let unit = ok(&mut session, "inc = n => n + 1").unit.unwrap();
    let before: Vec<_> = unit
        .tables()
        .dispatch
        .iter()
        .map(|d| d.rows.clone())
        .collect();
    assert_eq!(done(session.query("inc 2.0")).diagnostics, []);
    for (dispatch, rows) in unit.tables().dispatch.iter().zip(&before) {
        assert_eq!(&dispatch.rows, rows);
    }
    ok(&mut session, "inc 2.0");
    assert_ne!(
        unit.tables()
            .dispatch
            .iter()
            .map(|d| d.rows.clone())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn a_default_names_an_earlier_line() {
    let mut session = Session::new(host());
    ok(&mut session, "k = 5");
    ok(&mut session, "f = (m = k) => m");
    assert_eq!(query(&mut session, "x = f"), "Integer");
    ok(
        &mut session,
        "type R { a: Integer = k, b: Integer = (a + k) }",
    );
    assert_eq!(query(&mut session, "r = R"), "R");
    ok(&mut session, "id = v => v");
    ok(&mut session, "g = (n = (id 1)) => n");
    assert_eq!(query(&mut session, "y = g"), "Integer");
}

#[test]
fn only_the_last_accepted_line_can_be_withdrawn() {
    let mut session = Session::new(host());
    let first = ok(&mut session, "a = 1").accepted.unwrap();
    let second = ok(&mut session, "b = 2").accepted.unwrap();
    assert_eq!(session.withdraw(first), None, "a later line was accepted");
    assert!(session.withdraw(second).is_some());
    assert_eq!(session.withdraw(second), None, "already withdrawn");
    assert_eq!(session.withdraw(first), None, "never the last again");
    assert_eq!(session.lines().collect::<Vec<_>>(), ["a = 1"]);
    // A new line takes the withdrawn line's place, not its token.
    let third = ok(&mut session, "c = 3").accepted.unwrap();
    assert_ne!(third, second);
    assert_eq!(session.withdraw(second), None);
}

#[test]
fn a_refused_line_or_a_query_keeps_the_last_line_withdrawable() {
    let mut session = Session::new(host());
    let line = ok(&mut session, "a = 1").accepted.unwrap();
    let refused = done(session.line("a + \"x\""));
    assert_ne!(refused.diagnostics, []);
    assert_eq!(refused.accepted, None);
    assert_eq!(query(&mut session, "z = a"), "Integer");
    assert!(session.withdraw(line).is_some());
    assert_eq!(session.source(), "");
    assert_eq!(unknown_name(&done(session.line("a"))), [span(0, 1)]);
}

#[test]
#[should_panic(expected = "no line awaits a reply")]
fn withdrawing_abandons_a_line_awaiting_a_reply() {
    let mut session = Session::new(host());
    let line = ok(&mut session, "a = 1").accepted.unwrap();
    assert!(matches!(
        session.line("use missing"),
        Outcome::Request(Request::File { .. })
    ));
    assert!(session.withdraw(line).is_some());
    let _ = session.reply(Reply::File(None));
}

#[test]
fn an_operand_past_the_spots_types_as_an_application_of_the_result() {
    let mut session = Session::new(host());
    ok(&mut session, "mk = c => (x => x + c)");
    ok(&mut session, "k = n => mk 1 n");
    ok(&mut session, "j = n => (mk 1) n");
    assert_eq!(query(&mut session, "k"), query(&mut session, "j"));
    assert_eq!(query(&mut session, "k"), "Add<b> a, Literal b => a -> a");
}

#[test]
fn separate_entries_can_rebind_constructor_names() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Released { pub keys: Vec<U64> }",
        "type Outcome { Released(value: Released), Nothing }",
        "pub resolve: U64 -> Outcome = key => Released (Released { keys: [key] })",
    ] {
        let report = ok(&mut session, text);
        assert!(report.accepted.is_some());
        assert!(report.unit.is_some());
    }
}

#[test]
fn resource_markers_in_main_session_query_and_files_are_not_host_registration()
{
    use fipe::{check, lex::lex, parse::parse};
    for text in [
        "type resource Table",
        "type resource Table {}",
        "type resource Table: Integer",
        "type resource Table where self > 0",
    ] {
        let module = parse(lex(text)).unwrap();
        assert!(
            check::check(
                &module,
                &Declarations::new(),
                &[],
                check::Options::default()
            )
            .is_err(),
            "{text}"
        );
        let mut session = Session::new(Declarations::new());
        for report in [done(session.line(text)), done(session.query(text))] {
            assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::Resolve(e) if e.reason == Reason::DeclarationForm)), "{report:?}");
            assert!(report.accepted.is_none());
        }
    }
    for stem in ["csv", "main", "prelude"] {
        let mut session = Session::new(Declarations::new());
        let outcome = session.line(&format!("use {stem}"));
        let Outcome::Request(Request::File { stem: requested }) = outcome
        else {
            panic!("{stem}: {outcome:?}")
        };
        assert_eq!(requested, stem);
        let report = done(
            session
                .reply(Reply::File(Some("pub type resource Table {}".into()))),
        );
        assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::FileResolve { error, .. } if error.reason == Reason::DeclarationForm)), "{stem}: {report:?}");
        let report = done(
            session.line(&format!("f: {stem}.Table -> {stem}.Table = x => x")),
        );
        assert!(!report.diagnostics.is_empty(), "marked File export must not become a checked type: {stem}: {report:?}");
        assert!(report.accepted.is_none());
    }
}

#[test]
fn resource_private_unused_environment_fact_survives_session_and_query_fork() {
    let mut declarations = Declarations::new();
    declarations
        .module("csv.private", "type resource Hidden")
        .unwrap();
    let mut session = Session::new(declarations);
    let first = ok(&mut session, "1");
    assert!(first.unit.as_ref().unwrap().tables().has_resources);
    assert!(first
        .unit
        .as_ref()
        .unwrap()
        .tables()
        .meanings
        .iter()
        .all(|m| m.label != "Hidden"));
    let mut fork = session.clone();
    assert!(
        ok(&mut fork, "2")
            .unit
            .as_ref()
            .unwrap()
            .tables()
            .has_resources
    );
    let query = done(session.query("3"));
    assert!(query.diagnostics.is_empty());
    assert!(query.unit.as_ref().unwrap().tables().has_resources);
    assert!(
        ok(&mut session, "4")
            .unit
            .as_ref()
            .unwrap()
            .tables()
            .has_resources
    );
}

#[test]
fn resource_forbidden_file_cannot_export_indirect_or_unrelated_members() {
    let source = "type resource Secret\npub type resource Table {}\npub get: Table\npub make: () -> Table = _ => Table {}\npub type Alias: Table where True\npub type Wrap { value: Table }\npub plain = 7";
    let mut session = Session::new(Declarations::new());
    assert!(matches!(
        session.line("use bad"),
        Outcome::Request(Request::File { .. })
    ));
    let report = done(session.reply(Reply::File(Some(source.into()))));
    assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::FileResolve { error, .. } if error.reason == Reason::DeclarationForm)));
    for source in [
        "f: bad.Table -> bad.Table = x => x",
        "f = _ => bad.get",
        "f = _ => bad.make ()",
        "f: bad.Alias -> bad.Alias = x => x",
        "f: bad.Wrap -> bad.Wrap = x => x",
        "f = _ => bad.Wrap { value: bad.get }",
        "bad.plain",
    ] {
        let report = done(session.line(source));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::Resolve(_))),
            "{source}: {report:?}"
        );
        assert!(report.accepted.is_none());
    }
}

#[test]
fn resource_forbidden_file_does_not_leak_through_raw_host_signatures() {
    let mut declarations = Declarations::new();
    declarations.commands("pub get: csv.Table").unwrap();
    let mut session = Session::new(declarations);
    assert!(matches!(
        session.line("use csv"),
        Outcome::Request(Request::File { .. })
    ));
    let report = done(
        session.reply(Reply::File(Some("pub type resource Table {}".into()))),
    );
    assert!(!report.diagnostics.is_empty());
    let source = "f = _ => get";
    let mut fork = session.clone();
    for report in [
        done(session.line(source)),
        done(session.query(source)),
        done(fork.line(source)),
    ] {
        assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::Type(fipe::infer::Error { span, reason: fipe::infer::Reason::ResourceInFile { module } }) if module == "csv" && source.get(span.start..span.end) == Some("get"))), "{report:?}");
        assert!(report.accepted.is_none());
        assert!(report.unit.is_none());
    }
}

#[test]
fn resource_forbidden_file_cannot_leak_through_unmarked_wrapper_types() {
    let mut session = Session::new(Declarations::new());
    assert!(
        matches!(session.line("use bridge"), Outcome::Request(Request::File { ref stem }) if stem == "bridge")
    );
    let outcome = session.reply(Reply::File(Some("use bad\npub type Wrap { pub value: bad.Table }\npub type Alias: bad.Table where True".into())));
    assert!(
        matches!(outcome, Outcome::Request(Request::File { ref stem }) if stem == "bad")
    );
    let report = done(
        session.reply(Reply::File(Some("pub type resource Table {}".into()))),
    );
    assert!(!report.diagnostics.is_empty());
    for source in [
        "f: bridge.Wrap -> bridge.Wrap = x => x",
        "f: bridge.Alias -> bridge.Alias = x => x",
    ] {
        let mut fork = session.clone();
        for report in [
            done(session.line(source)),
            done(session.query(source)),
            done(fork.line(source)),
        ] {
            assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::Type(fipe::infer::Error { span, reason: fipe::infer::Reason::ResourceInFile { module } }) if module == "bad" && source.get(span.start..span.end).is_some_and(|text| text.starts_with("bridge.")))), "{source}: {report:?}");
            assert!(report.accepted.is_none());
            assert!(report.unit.is_none());
        }
    }
}

#[test]
fn resource_quarantine_preserves_ordinary_file_error_recovery() {
    let mut session = Session::new(Declarations::new());
    assert!(matches!(
        session.line("use ordinary"),
        Outcome::Request(Request::File { .. })
    ));
    let report = done(session.reply(Reply::File(Some(
        "pub okay = 7\npub broken = missing".into(),
    ))));
    assert!(!report.diagnostics.is_empty());
    let report = ok(&mut session, "f: () -> Integer = _ => ordinary.okay");
    assert!(report.accepted.is_some());
    assert!(report
        .unit
        .as_ref()
        .is_none_or(|unit| !unit.tables().has_resources));
}

#[test]
fn resource_valid_host_type_is_allowed_inside_ordinary_file_records() {
    let mut declarations = Declarations::new();
    declarations
        .module("csv", "pub type resource Table")
        .unwrap();
    let mut session = Session::new(declarations);
    assert!(matches!(
        session.line("use bridge"),
        Outcome::Request(Request::File { .. })
    ));
    let report = done(session.reply(Reply::File(Some(
        "use csv\npub type Wrap { pub value: csv.Table }".into(),
    ))));
    assert!(report.diagnostics.is_empty(), "{report:?}");
    let report = ok(&mut session, "f: bridge.Wrap -> bridge.Wrap = x => x");
    let tables = report.unit.as_ref().unwrap().tables();
    assert!(tables.has_resources);
    let resource = tables
        .meanings
        .iter()
        .find(|meaning| meaning.resource)
        .unwrap();
    assert_eq!(resource.label, "Table");
    assert_eq!(tables.namespaces[resource.namespace], "csv");
    assert!(resource.constructors.is_empty());
}

#[test]
fn claim_qualifiers_failed_local_query_and_line_preserve_prior_facts() {
    let mut session = Session::new(Declarations::new());
    ok(&mut session, "keep = 7");
    let before = session.source().to_owned();
    let invalid = "type Key where self.inspect: Integer\ntype Cell<T>: Key { value: T }\nimpl Cell<U>: Key { inspect: Eq U => Self -> Integer = self => 1 }";
    for read_only in [true, false] {
        let report = done(if read_only {
            session.query(invalid)
        } else {
            session.line(invalid)
        });
        assert!(
            report.diagnostics.iter().any(|d| matches!(
                d,
                Diagnostic::Type(fipe::infer::Error {
                    reason: fipe::infer::Reason::ClaimBlock { .. },
                    ..
                })
            )),
            "{:?}",
            report.diagnostics
        );
        assert!(report.accepted.is_none());
        assert_eq!(session.source(), before);
        assert_eq!(query(&mut session, "keep"), "Integer");
    }
    let report = done(session.query("Cell"));
    assert!(!report.diagnostics.is_empty());
    ok(&mut session, "keep");
}

#[test]
fn claim_qualifiers_loaded_unused_members_keep_foreign_spans() {
    for (bound, valid) in [("", false), (": Eq", true)] {
        let mut session = Session::new(Declarations::new());
        ok(&mut session, "keep = 7");
        assert!(
            matches!(session.line("use foreign"), Outcome::Request(Request::File { stem }) if stem == "foreign")
        );
        let source = format!("pub type Key where self.inspect: Integer\npub type Cell<T{bound}>: Key {{ pub value: T }}\nimpl Cell<U>: Key {{ inspect: Eq U => Self -> Integer = self => 1 }}\npub accept: Key -> Key = x => x");
        let report = done(session.reply(Reply::File(Some(source.clone()))));
        let failures: Vec<_> = report
            .diagnostics
            .iter()
            .filter_map(|d| match d {
                Diagnostic::FileType { stem, error }
                    if matches!(
                        error.reason,
                        fipe::infer::Reason::ClaimBlock { .. }
                    ) =>
                {
                    Some((stem, error))
                }
                _ => None,
            })
            .collect();
        if valid {
            assert_eq!(report.diagnostics, []);
        } else {
            assert!(!failures.is_empty(), "{:?}", report.diagnostics);
            for (stem, error) in failures {
                assert_eq!(stem, "foreign");
                assert_eq!(
                    fipe::token::Source::new(&source).slice(error.span),
                    Some("inspect")
                );
            }
        }
        // Foreign diagnostics are reported once, while invalid source still
        // prevents an executable unit. Recovery typing remains available.
        for read_only in [true, false] {
            let text = if read_only {
                "foreign.accept (foreign.Cell 1)"
            } else {
                "packed = foreign.accept (foreign.Cell 1)"
            };
            let report = done(if read_only {
                session.query(text)
            } else {
                session.line(text)
            });
            assert_eq!(report.diagnostics, []);
            assert_eq!(report.unit.is_some(), valid);
            assert_eq!(report.accepted.is_none(), read_only);
        }
        assert_eq!(query(&mut session, "keep"), "Integer");
    }
}

const RECOVERY_FOREIGN: &str = "pub type Key where self.inspect: Integer\npub type Cell<T>: Key { pub value: T }\nimpl Cell<U>: Key { inspect: Eq U => Self -> Integer = self => 1 }\npub accept: Key -> Key = x => x";

fn recovery_session() -> (Session, Machine) {
    let mut session = Session::new(Declarations::new());
    let mut machine = Machine::new(Some(100_000));
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "keep = 7"),
        Value::Unit
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "same = a b => a == b"),
        Value::Unit
    );
    assert!(
        matches!(session.line("use foreign"), Outcome::Request(Request::File { stem }) if stem == "foreign")
    );
    let report =
        done(session.reply(Reply::File(Some(RECOVERY_FOREIGN.into()))));
    assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::FileType { stem, error } if stem == "foreign" && matches!(error.reason, fipe::infer::Reason::ClaimBlock { .. }))));
    assert!(report.unit.is_none());
    assert!(report.accepted.is_some());
    (session, machine)
}

fn run_recovery_line(
    session: &mut Session,
    machine: &mut Machine,
    text: &str,
) -> Value {
    let report = ok(session, text);
    let program = compile::compile(
        report
            .unit
            .as_ref()
            .unwrap_or_else(|| panic!("executable healthy line: {text}")),
    );
    match machine.run(&program) {
        Outcome::Done(Ok(value)) => value,
        outcome => panic!("{text}: {outcome:?}"),
    }
}

#[test]
fn recovery_only_dependencies_withhold_units_without_repeating_diagnostics() {
    for (setup, use_value) in [
        ("packed = foreign.accept (foreign.Cell 1)", "packed"),
        ("packed = foreign.accept (foreign.Cell 1)", "alias = packed"),
        ("packed = foreign.accept (foreign.Cell 1)", "f = _ => packed"),
        ("(left, right) = (foreign.accept (foreign.Cell 1), 2)", "left"),
        ("type Local { n: Integer }\nimpl Local { get = self => self.n }\nblocked = foreign.accept (foreign.Cell 1)", "(Local 1).get"),
        ("type Local { n: Integer }\nimpl Local { n => Self { n: n } }\nblocked = foreign.accept (foreign.Cell 1)", "Local 1"),
        ("type Local { n: Integer = 1 }\nblocked = foreign.accept (foreign.Cell 1)", "Local"),
        ("type Local: Eq { n: Integer }\nimpl Local: Eq { eq = other self => True }\nblocked = foreign.accept (foreign.Cell 1)", "(Local 1) == (Local 2)"),
        ("type Local: Eq { n: Integer }\nimpl Local: Eq { eq = other self => True }\nblocked = foreign.accept (foreign.Cell 1)", "same (Local 1) (Local 2)"),
        ("type Local: Show { n: Integer }\nimpl Local: Show { show = self => \"local\" }\nblocked = foreign.accept (foreign.Cell 1)", "rendered: String = Local 1"),
    ] {
        let (mut session, mut machine) = recovery_session();
        let report = ok(&mut session, setup);
        assert!(report.unit.is_none(), "{setup}");
        assert!(report.accepted.is_some());
        let before = session.source().to_owned();
        let report = done(session.query(use_value));
        assert_eq!(report.diagnostics, [], "{use_value}");
        let typed = report.typed.as_ref().unwrap();
        if use_value.starts_with("rendered:") {
            assert!(typed.casts.iter().flatten().filter_map(|cast| cast.operation.as_ref()).any(|op| matches!(op.selected, fipe::typed::Selected::Value { origin: Origin::Line(_), .. })), "real prior-line cast operation required");
        }
        if use_value.starts_with("same ") {
            assert!(typed.dispatch.iter().flat_map(|d| &d.rows).any(|row| matches!(row.selected, fipe::typed::Selected::Value { origin: Origin::Line(_), .. })), "real prior-line dispatch row required");
        }
        assert!(report.unit.is_none(), "{use_value}");
        assert!(report.accepted.is_none());
        assert_eq!(session.source(), before);
        let report = ok(&mut session, use_value);
        assert!(report.unit.is_none(), "{use_value}");
        assert!(report.accepted.is_some());
        assert_eq!(run_recovery_line(&mut session, &mut machine, "keep"), Value::Int(7));
    }
}

#[test]
fn recovery_type_data_and_unused_defaults_do_not_block_healthy_emission() {
    let (mut session, mut machine) = recovery_session();
    let report = ok(&mut session, "type Local { n: Integer = 1 }\nimpl Local { get = self => self.n }\nblocked = foreign.accept (foreign.Cell 1)");
    assert!(report.unit.is_none());
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "(Local 2).n"),
        Value::Int(2)
    );
    run_recovery_line(&mut session, &mut machine, "identity = x => x");
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "identity keep"),
        Value::Int(7)
    );
}

#[test]
fn recovery_readiness_survives_clone_withdraw_and_redefinition() {
    let (mut session, mut machine) = recovery_session();
    let recovered =
        ok(&mut session, "packed = foreign.accept (foreign.Cell 1)");
    let mut cloned = session.clone();
    let report = ok(&mut cloned, "packed");
    assert!(report.unit.is_none());
    assert!(session.withdraw(recovered.accepted.unwrap()).is_some());
    let report = done(session.query("packed"));
    assert!(!report.diagnostics.is_empty());
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "packed = keep"),
        Value::Unit
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "packed"),
        Value::Int(7)
    );
    assert!(done(cloned.query("packed")).unit.is_none());
}

#[test]
fn recovery_preparation_preserves_earlier_defaults_and_weak_queries() {
    let (mut session, mut machine) = recovery_session();
    run_recovery_line(
        &mut session,
        &mut machine,
        "type R { a: Integer = keep, b: Integer = (a + keep), c: Integer = 0 }",
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "(R).b"),
        Value::Int(14)
    );
    let reordered = ok(&mut session, "r = R { c: 0, a: 3 }");
    let program = compile::compile(reordered.unit.as_ref().unwrap());
    assert!(matches!(
        machine.run(&program),
        Outcome::Done(Ok(Value::Unit))
    ));
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "r.b"),
        Value::Int(10)
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "f = (width = keep) (cols = (width + 1)) => cols",
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "f"),
        Value::Int(8)
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "type Inner { n: Integer = keep }",
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "type Outer { inner: Inner = Inner }",
    );
    run_recovery_line(&mut session, &mut machine, "outer = Outer");
    run_recovery_line(&mut session, &mut machine, "inner = outer.inner");
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "inner.n"),
        Value::Int(7)
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "g = (n = { id = x => x\nid keep }) => n",
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "g"),
        Value::Int(7)
    );
    run_recovery_line(&mut session, &mut machine, "[empty] = [[]]");
    let original = done(session.query("empty")).unit.unwrap();
    let weak = original.tables().session_meanings.clone();
    ok(&mut session, "packed = foreign.accept (foreign.Cell 1)");
    let before = session.source().to_owned();
    let report = done(session.query("fixed: [Integer; 0] = empty\npacked"));
    assert_eq!(report.diagnostics, []);
    assert!(report.unit.is_none());
    assert_eq!(session.source(), before);
    assert_eq!(original.tables().session_meanings, weak);
    assert_eq!(
        done(session.query("empty"))
            .unit
            .unwrap()
            .tables()
            .session_meanings,
        weak
    );
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "keep"),
        Value::Int(7)
    );
}

#[test]
fn recovery_preparation_keeps_physical_default_order_for_private_fields() {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "config",
            "pub type R { hidden: Integer = 7, pub value: Integer }",
        )
        .unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(Some(100_000));
    run_recovery_line(&mut session, &mut machine, "use config");
    let report = ok(&mut session, "r = config.R 8");
    assert!(
        report
            .typed
            .as_ref()
            .unwrap()
            .constructor_fields
            .values()
            .any(|fields| fields == &[1, 0]),
        "actual private/public physical reordering required"
    );
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert!(matches!(
        machine.run(&program),
        Outcome::Done(Ok(Value::Unit))
    ));
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "r.value"),
        Value::Int(8)
    );
}

#[test]
fn recovery_pattern_with_live_reader_refuses_unemitted_equality() {
    let (mut session, mut machine) = recovery_session();
    run_recovery_line(&mut session, &mut machine, "type Local: Eq & Literal { n: Integer }\nimpl Local: Literal { literal: Form -> Self = form => Self 0 }");
    run_recovery_line(&mut session, &mut machine, "literal_value: Local = 1");
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "literal_value.n"),
        Value::Int(0)
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "choose: Local -> Integer = { 1 => 7, _ => 0 }",
    );
    let earlier = ok(&mut session, "choose (Local 0)");
    let earlier_program = compile::compile(earlier.unit.as_ref().unwrap());
    assert!(matches!(
        machine.run(&earlier_program),
        Outcome::Done(Ok(Value::Int(7)))
    ));
    let report = ok(&mut session, "impl Local: Eq { eq = other self => False }\nblocked = foreign.accept (foreign.Cell 1)");
    assert!(report.unit.is_none());
    let report =
        done(session.query("later: Local -> Integer = { 1 => 7, _ => 0 }"));
    assert_eq!(report.diagnostics, []);
    let typed = report.typed.as_ref().unwrap();
    assert!(
        typed.pattern_equals.values().any(|op| matches!(
            op.selected,
            fipe::typed::Selected::Value {
                origin: Origin::Line(_),
                ..
            }
        )),
        "actual unavailable equality required"
    );
    let equality = typed
        .pattern_equals
        .values()
        .find(|op| {
            matches!(
                op.selected,
                fipe::typed::Selected::Value {
                    origin: Origin::Line(_),
                    ..
                }
            )
        })
        .unwrap();
    assert!(
        typed
            .dispatch
            .iter()
            .flat_map(|d| &d.rows)
            .any(|row| row.selected == equality.selected),
        "the checker records equality in both consumed paths"
    );
    assert!(report.unit.is_none());
    assert!(report.accepted.is_none());
    assert_eq!(
        run_recovery_line(&mut session, &mut machine, "literal_value.n"),
        Value::Int(0)
    );
    // A fresh call may request the newly unavailable Eq dispatch row. The
    // already emitted program and its frozen equality remain executable.
    assert!(matches!(
        machine.run(&earlier_program),
        Outcome::Done(Ok(Value::Int(7)))
    ));
}

const OWNER_MEMBERS: &str="impl W<a> { probe: () -> Integer = _ => { x: a = csv.get ()\n1 }, call: Self -> Integer = self => Self.probe () }";
fn owner_declarations() -> Declarations {
    let mut declarations = Declarations::new();
    declarations.module("csv", "pub get: () -> a").unwrap();
    declarations
}
fn owner_answer(
    machine: &mut Machine,
    program: &fipe::mir::Program,
    reply: Value,
) -> Outcome<Result<Value, fipe::vm::Fault>> {
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(program)
    else {
        panic!("owner get request")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "get"
    );
    assert_eq!(arguments, [Value::Unit]);
    machine.reply(program, Reply::Value(Ok(reply)))
}

#[test]
fn owner_member_survives_accepted_lines_clone_withdraw_and_redefinition() {
    let mut session = Session::new(owner_declarations());
    let mut machine = Machine::new(Some(100_000));
    for source in ["use csv", "type W<T> { value: T }", OWNER_MEMBERS] {
        run_recovery_line(&mut session, &mut machine, source);
    }
    let report = ok(&mut session, "x: W<Integer> = W 1");
    assert!(report.accepted.is_some());
    assert!(matches!(
        machine.run(&compile::compile(report.unit.as_ref().unwrap())),
        Outcome::Done(Ok(Value::Unit))
    ));
    let mut cloned = session.clone();
    let report = ok(&mut session, "x.call");
    let call = report.accepted.unwrap();
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert!(matches!(
        owner_answer(&mut machine, &program, Value::text("wrong")),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Host(_),
            ..
        }))
    ));
    machine.withdraw(&session.withdraw(call).unwrap());
    run_recovery_line(&mut session, &mut machine, "x: W<String> = W \"new\"");
    let report = ok(&mut session, "x.call");
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert!(matches!(
        owner_answer(&mut machine, &program, Value::text("right")),
        Outcome::Done(Ok(Value::Int(1)))
    ));
    let report = done(cloned.query("v: W<Integer> = W 1\nv.call"));
    assert_eq!(report.diagnostics, []);
    let program = compile::compile(report.unit.as_ref().unwrap());
    let mut clone_machine = Machine::new(Some(100_000));
    assert!(matches!(
        owner_answer(&mut clone_machine, &program, Value::text("wrong")),
        Outcome::Done(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Host(_),
            ..
        }))
    ));
}

#[test]
fn owner_member_import_uses_the_same_file_scheme_and_owner() {
    let mut session = Session::new(owner_declarations());
    let file="use csv\npub type W<T> { pub value: T }\nimpl W<a> { pub probe: () -> Integer = _ => { x: a = csv.get ()\n1 }, pub call: Self -> Integer = self => Self.probe () }";
    assert!(
        matches!(session.line("use foreign\nx: foreign.W<Integer> = foreign.W 1\nx.call"),Outcome::Request(Request::File{stem})if stem=="foreign")
    );
    let report = done(session.reply(Reply::File(Some(file.into()))));
    assert_eq!(report.diagnostics, []);
    let program = compile::compile(report.unit.as_ref().unwrap());
    for wrong in [true, false] {
        let mut machine = Machine::new(Some(100_000));
        let result = owner_answer(
            &mut machine,
            &program,
            if wrong {
                Value::text("wrong")
            } else {
                Value::Int(7)
            },
        );
        if wrong {
            assert!(
                matches!(
                    result,
                    Outcome::Done(Err(fipe::vm::Fault {
                        reason: fipe::value::Reason::Host(_),
                        ..
                    }))
                ),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Outcome::Done(Ok(Value::Int(1)))),
                "{result:?}"
            );
        }
    }
}

#[test]
fn receiver_adapter_old_alias_survives_repeated_withdrawal_and_row_reuse() {
    let mut declarations = Declarations::new();
    declarations
        .module(
            "host",
            "pub load: () -> ((Integer -> Integer -> Integer) * ())",
        )
        .unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(Some(100_000));
    run_recovery_line(&mut session, &mut machine, "use host");
    run_recovery_line(&mut session, &mut machine, "type M { n: Integer }");
    let report = ok(
        &mut session,
        "impl M { op = a b self => self.n + a * 10 + b }\n(M 2).op",
    );
    let mut implementation = report.accepted.unwrap();
    let Outcome::Done(Ok(old @ Value::Function(_))) =
        machine.run(&compile::compile(report.unit.as_ref().unwrap()))
    else {
        panic!("old partial")
    };
    for extra in [100, 200] {
        machine.withdraw(&session.withdraw(implementation).unwrap());
        let report = ok(&mut session, &format!("impl M {{ op = a b self => self.n + a * 10 + b + {extra} }}\n(f, ignored) = host.load ()\n((M 2).op 3 4, f 3 4)"));
        implementation = report.accepted.unwrap();
        let program = compile::compile(report.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("load")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "load"
        );
        assert_eq!(arguments, [Value::Unit]);
        let Outcome::Done(Ok(Value::Tuple(values))) = machine.reply(
            &program,
            fipe::embed::Reply::Value(Ok(Value::Tuple(std::rc::Rc::new(
                vec![old.clone(), Value::Unit],
            )))),
        ) else {
            panic!("new and old results")
        };
        assert_eq!(values.as_slice(), [Value::Int(36 + extra), Value::Int(36)]);
    }
}

#[test]
fn bound_member_witnesses_follow_later_session_calls_and_withdrawal() {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a").unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(Some(100_000));
    let setup=ok(&mut session,"use host\ntype W<T> { value: T }\nimpl W<a> { op: Self -> a = self => host.get () }\nget = w => w.op");
    let program = compile::compile(setup.unit.as_ref().unwrap());
    assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
    let mut fork = session.clone();
    let cloned = ok(&mut fork, "get (W 1)");
    let fork_program = compile::compile(cloned.unit.as_ref().unwrap());
    assert!(!std::rc::Rc::ptr_eq(&program.tables, &fork_program.tables));
    let mut fork_machine = Machine::new(Some(100_000));
    assert!(matches!(
        fork_machine.run(&fork_program),
        Outcome::Request(Request::Call { .. })
    ));
    assert!(matches!(
        fork_machine.reply(&fork_program, Reply::Value(Ok(Value::Int(9)))),
        Outcome::Done(Ok(Value::Int(9)))
    ));
    for (source, value) in [
        ("get (W 1)", Value::Int(7)),
        ("get (W \"x\")", Value::text("ok")),
        ("get (W 2)", Value::Int(8)),
    ] {
        let report = ok(&mut session, source);
        let program = compile::compile(report.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("expected get")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "get"
        );
        assert_eq!(arguments, [Value::Unit]);
        assert!(
            matches!(machine.reply(&program,Reply::Value(Ok(value.clone()))),Outcome::Done(Ok(actual)) if actual==value)
        );
        let withdrawn = session.withdraw(report.accepted.unwrap()).unwrap();
        machine.withdraw(&withdrawn);
    }
}

#[test]
fn receiver_adapter_stored_callable_resolves_weak_session_result_on_later_call()
{
    for reply in [Value::Int(7), Value::text("wrong")] {
        let mut declarations = Declarations::new();
        declarations.module("host", "pub type M { pub n: Integer }\npub save: a -> ()\nimpl M { op: Integer -> Integer -> (self: Self) -> a }").unwrap();
        let mut session = Session::new(declarations);
        let mut machine = Machine::new(Some(100_000));
        run_recovery_line(
            &mut session,
            &mut machine,
            "use host\npartial = m => m.op",
        );
        // No result annotation here: the stored method really precedes its
        // later Integer constraint and keeps a Session witness, not Integer.
        let stored =
            ok(&mut session, "[f] = [partial (host.M 7)]\nhost.save f");
        assert!(
            !stored.typed.as_ref().unwrap().session_variables.is_empty(),
            "pattern binding must actually weaken the stored result"
        );
        let program = compile::compile(stored.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("save")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "save"
        );
        let [Value::Function(function)] = arguments.as_slice() else {
            panic!("stored callable")
        };
        assert_eq!(function.filled.len(), 1);
        let original_receiver = function.filled[0].clone();
        let mut pending =
            vec![function.meanings.last().expect("full callable witness")];
        let mut weak = Vec::new();
        while let Some(ty) = pending.pop() {
            match ty {
                fipe::ir::Type::Session(slot) => weak.push(*slot),
                fipe::ir::Type::Function { parameter, result } => {
                    pending.push(parameter);
                    pending.push(result);
                }
                _ => (),
            }
        }
        assert_eq!(
            weak.len(),
            1,
            "must observe actual unresolved stored result"
        );
        let slot = weak[0];
        assert!(program.tables.borrow().session_meanings[slot].is_none());
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(Value::Unit))),
            Outcome::Done(Ok(Value::Unit))
        ));

        let later = ok(&mut session, "x: Integer = f 3 4\nx");
        let program = compile::compile(later.unit.as_ref().unwrap());
        let tables = program.tables.borrow();
        let Some(fipe::ir::Type::Meaning { index, arguments }) =
            &tables.session_meanings[slot]
        else {
            panic!("settled result")
        };
        assert_eq!(tables.meanings[*index].label, "Integer");
        assert!(arguments.is_empty());
        drop(tables);
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("op")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "op"
        );
        assert_eq!(
            arguments,
            [Value::Int(3), Value::Int(4), original_receiver]
        );
        let outcome = machine.reply(&program, Reply::Value(Ok(reply.clone())));
        if reply == Value::Int(7) {
            assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(7)))));
        } else {
            assert!(
                matches!(outcome, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(ref message), .. })) if message.contains("Integer")),
                "{outcome:?}"
            );
        }
    }
}

fn expanded_callable_setup(depth: usize) -> (Session, Machine) {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type M<T> { pub n: Integer }\npub grow: M<a> -> M<Vec<a>>\npub save: a -> ()\nimpl M<a> { op: Integer -> Integer -> (self: Self) -> Integer }").unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(None);
    run_recovery_line(&mut session, &mut machine, "use host\nget = m => m.op\napply_one = fn x => fn x\ntype Small: Integer where self < 10\nx0: host.M<()> = host.M 7");
    for index in 0..depth {
        let report = ok(
            &mut session,
            &format!("x{} = host.grow x{index}", index + 1),
        );
        let program = compile::compile(report.unit.as_ref().unwrap());
        assert!(!program.tables.borrow().has_resources);
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("grow")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "grow"
        );
        let [value @ Value::Record(_)] = arguments.as_slice() else {
            panic!("lawful M payload")
        };
        // Phantom arguments change only through the checked grow signature;
        // the Host returns the actual ordinary record it just received.
        let value = value.clone();
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(value))),
            Outcome::Done(Ok(Value::Unit))
        ));
    }
    (session, machine)
}

fn expanded_callable_capture(
    session: &mut Session,
    machine: &mut Machine,
    text: &str,
    depth: usize,
    filled: usize,
    guarded: bool,
) -> (fipe::mir::Program, u64) {
    let report = ok(session, text);
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert!(!program.tables.borrow().has_resources);
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("save actual callable")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "save"
    );
    let [Value::Function(function)] = arguments.as_slice() else {
        panic!("actual callable")
    };
    assert_eq!(function.filled.len(), filled);
    assert_eq!(!function.guards.is_empty(), guarded);
    let joined = machine.program().expect("joined runtime program");
    let tables = joined.tables.borrow();
    assert!(!tables.has_resources);
    let context = joined.receiver_contexts[function.body].as_ref().unwrap();
    let slot = context.protocol_result.as_ref().unwrap().meaning;
    if filled == 2 {
        let definition = tables
            .definitions
            .iter()
            .position(|definition| definition.label == "apply_one")
            .unwrap();
        let body = &joined.bodies[joined.definitions[definition].unwrap()];
        assert_eq!(body.arity, 2);
        assert!(
            matches!(body.locals[0].ty, fipe::ir::Type::Parameter(_)),
            "generic result stays variable in helper body"
        );
        assert!(body.blocks.iter().any(|block| matches!(&block.terminator, fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Value(_), args, .. } if args.len() == 1)));
        assert!(!body.blocks.iter().flat_map(|block| &block.statements).any(
            |statement| matches!(
                statement,
                fipe::mir::Statement::Assign(
                    _,
                    fipe::mir::Rvalue::Closure { .. }
                )
            )
        ));
    }

    let mut nodes = vec![(&function.meanings[slot], 0)];
    let mut deepest = 0;
    while let Some((ty, count)) = nodes.pop() {
        match ty {
            fipe::ir::Type::Meaning { index, arguments } => {
                let count =
                    count + usize::from(tables.meanings[*index].label == "Vec");
                deepest = deepest.max(count);
                nodes.extend(arguments.iter().map(|ty| (ty, count)));
            }
            fipe::ir::Type::Function { parameter, result } => {
                nodes.push((parameter, count));
                nodes.push((result, count));
            }
            _ => (),
        }
    }
    assert!(
        deepest >= depth,
        "actual copied C must contain expanded checked type"
    );
    drop(tables);
    let charged = machine.charged();
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Unit))),
        Outcome::Done(Ok(Value::Unit))
    ));
    (program, charged)
}

fn expanded_callable_open_alias(session: &mut Session, machine: &mut Machine) {
    let report = ok(session, "open = f 3\nhost.save (f, open)");
    let program = compile::compile(report.unit.as_ref().unwrap());
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("open alias")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "save"
    );
    let [Value::Tuple(pair)] = arguments.as_slice() else {
        panic!("alias pair")
    };
    let [Value::Function(original), Value::Function(open)] = pair.as_slice()
    else {
        panic!("functions")
    };
    assert_eq!(
        original.filled.len(),
        1,
        "original receiver-bound alias remains unchanged"
    );
    assert_eq!(
        open.body, original.body,
        "positive prefix enters the actual raw adapter"
    );
    assert!(!std::rc::Rc::ptr_eq(open, original));
    assert_eq!(open.filled.len(), 2);
    assert_eq!(open.filled[0], original.filled[0]);
    assert_eq!(open.filled[1], Value::Int(3));
    assert_eq!(open.captures, original.captures);
    assert_eq!(
        open.meanings, original.meanings,
        "partial keeps the full callable context before final projection"
    );
    let joined = machine.program().unwrap();
    assert_eq!(joined.bodies[open.body].arity, 3);
    assert!(joined.bodies[open.body].binding.is_none());
    let context = joined.receiver_contexts[open.body]
        .as_ref()
        .expect("actual receiver adapter context");
    let callable = context
        .protocol_result
        .as_ref()
        .expect("full caller callable")
        .meaning;
    assert_eq!(open.meanings[callable], original.meanings[callable]);
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Unit))),
        Outcome::Done(Ok(Value::Unit))
    ));
    let report = ok(session, "open 4");
    let program = compile::compile(report.unit.as_ref().unwrap());
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("invoke open alias")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "op"
    );
    assert!(matches!(
        arguments.as_slice(),
        [Value::Int(3), Value::Int(4), Value::Record(_)]
    ));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
        Outcome::Done(Ok(Value::Int(7)))
    ));
}

fn expanded_callable_budget_then_retry(
    machine: &mut Machine,
    program: &fipe::mir::Program,
    budget: u64,
    label: &str,
) {
    use fipe::value::Reason;
    machine.set_budget(Some(budget));
    let outcome = machine.run(program);
    assert!(
        matches!(
            outcome,
            Outcome::Done(Err(fipe::vm::Fault {
                reason: Reason::Budget,
                ..
            }))
        ),
        "must fail before {label}: {outcome:?}"
    );
    machine.set_budget(None);
    let Outcome::Request(Request::Call { declaration, .. }) =
        machine.run(program)
    else {
        panic!("healthy retry")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        label
    );
    let reply = if label == "op" {
        Value::Int(7)
    } else {
        Value::Unit
    };
    assert!(matches!(
        machine.reply(program, Reply::Value(Ok(reply))),
        Outcome::Done(Ok(_))
    ));
}

fn expanded_callable_depth_boundary(
    session: &Session,
    depth: usize,
) -> Session {
    let mut boundary = session.clone();
    for index in depth..fipe::parse::MAX_NESTING {
        let text = format!("x{} = host.grow x{index}", index + 1);
        let report = done(boundary.line(&text));
        if index + 1 == fipe::parse::MAX_NESTING {
            assert!(report.unit.is_none());
            assert!(matches!(
                report.diagnostics.as_slice(),
                [Diagnostic::Type(fipe::infer::Error {
                    reason: fipe::infer::Reason::TypeDepth,
                    ..
                })]
            ));
        } else {
            assert_eq!(report.diagnostics, [], "boundary setup: {text}");
            assert!(report.unit.is_some());
        }
    }
    boundary
}

fn expanded_callable_factory(session: &mut Session, machine: &mut Machine) {
    let report = ok(session, "factory: () -> Small -> Integer -> Integer = _ => f\nhost.save factory");
    let program = compile::compile(report.unit.as_ref().unwrap());
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("outer factory")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "save"
    );
    let [Value::Function(factory)] = arguments.as_slice() else {
        panic!("actual outer factory")
    };
    let joined = machine.program().expect("joined factory program");
    let arity = joined.bodies[factory.body].arity;
    assert_eq!(arity, 1);
    assert!(factory.guards.iter().any(|guard| guard.spot >= arity));
    assert!(matches!(
        machine.reply(&program, Reply::Value(Ok(Value::Unit))),
        Outcome::Done(Ok(Value::Unit))
    ));
}

#[test]
fn receiver_adapter_expanded_callable_copy_is_budgeted_at_each_transition() {
    const DEPTH: usize = fipe::parse::MAX_NESTING - 4;

    let (mut shallow, mut sm) = expanded_callable_setup(0);
    // Three original protocol arrows plus nominal M leave this many
    // nested Vec arguments inside the checked language depth boundary.
    let (mut deep, mut dm) = expanded_callable_setup(DEPTH);
    let _boundary = expanded_callable_depth_boundary(&deep, DEPTH);
    let creation = format!("f = get x{DEPTH}\nhost.save f");
    // Setup runs are unbounded and finish before each isolated measured run.
    for (small, large, filled, guarded) in [
        ("f = get x0\nhost.save f", creation.as_str(), 1, false),
        (
            "g = apply_one f 3\nhost.save g",
            "g = apply_one f 3\nhost.save g",
            2,
            false,
        ),
        (
            "guarded: Small -> Integer -> Integer = f\nhost.save guarded",
            "guarded: Small -> Integer -> Integer = f\nhost.save guarded",
            1,
            true,
        ),
    ] {
        let (_, low) = expanded_callable_capture(
            &mut shallow,
            &mut sm,
            small,
            0,
            filled,
            guarded,
        );
        let (program, high) = expanded_callable_capture(
            &mut deep, &mut dm, large, DEPTH, filled, guarded,
        );
        assert!(
            high > low + DEPTH as u64,
            "deep C transition must be charged: {low}/{high}"
        );
        expanded_callable_budget_then_retry(&mut dm, &program, low, "save");
        if filled == 1 && !guarded {
            expanded_callable_open_alias(&mut shallow, &mut sm);
            expanded_callable_open_alias(&mut deep, &mut dm);
        }
    }
    // Observe the actual outer closure before calling it: its guard is
    // beyond its one raw formal, so the later transfer cannot be confused
    // with merely entering the returned function at the call site.
    for (session, machine) in [(&mut shallow, &mut sm), (&mut deep, &mut dm)] {
        expanded_callable_factory(session, machine);
    }
    let (_, low) = expanded_callable_capture(
        &mut shallow,
        &mut sm,
        "transferred = factory ()\nhost.save transferred",
        0,
        1,
        true,
    );
    let (program, high) = expanded_callable_capture(
        &mut deep,
        &mut dm,
        "transferred = factory ()\nhost.save transferred",
        DEPTH,
        1,
        true,
    );
    assert!(
        high > low + DEPTH as u64,
        "beyond-guard transfer must charge C: {low}/{high}"
    );
    expanded_callable_budget_then_retry(&mut dm, &program, low, "save");

    let mut costs = Vec::new();
    let mut deep_program = None;
    for (session, machine) in [(&mut shallow, &mut sm), (&mut deep, &mut dm)] {
        let report = ok(session, "transferred 3 4");
        let program = compile::compile(report.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("full invocation")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "op"
        );
        assert!(matches!(
            arguments.as_slice(),
            [Value::Int(3), Value::Int(4), Value::Record(_)]
        ));
        costs.push(machine.charged());
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
            Outcome::Done(Ok(Value::Int(7)))
        ));
        deep_program = Some(program);
    }
    assert!(costs[1] > costs[0] + DEPTH as u64);
    expanded_callable_budget_then_retry(
        &mut dm,
        &deep_program.unwrap(),
        costs[0],
        "op",
    );
}

#[test]
fn host_default_late_writes_survive_failed_line_then_healthy_retry_and_clone() {
    let mut declarations = Declarations::new();
    declarations.module("probe", "pub type Box { pub n: Integer }\npub type A { pub n: Integer }\nimpl A { f: () -> (Integer * Integer) = _ => (entry 7, entry 8), g = x => Box x }\npub entry: (seed: Integer) -> (n: Integer = (A.g seed).n) -> Integer").expect("late default header");
    let mut session = Session::new(declarations);
    let imported = ok(&mut session, "use probe");
    let import_program =
        compile::compile(imported.unit.as_ref().expect("import Unit"));
    let mut control = session.clone();
    let before: Vec<_> = session.lines().map(String::from).collect();
    let rejected = done(session.line("bad: Bool = probe.A.f ()"));
    assert!(!rejected.diagnostics.is_empty());
    assert!(rejected.accepted.is_none());
    assert!(rejected.unit.is_none());
    assert_eq!(
        session.lines().map(String::from).collect::<Vec<_>>(),
        before
    );
    for candidate in [&mut session, &mut control] {
        let mut machine = Machine::new(Some(100_000));
        assert!(matches!(
            machine.run(&import_program),
            Outcome::Done(Ok(Value::Unit))
        ));
        let healthy = ok(candidate, "probe.A.f ()");
        assert!(healthy.accepted.is_some());
        let program =
            compile::compile(healthy.unit.as_ref().expect("healthy Unit"));
        let mut outcome = machine.run(&program);
        for value in [7, 8] {
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = outcome
            else {
                panic!("entry after rollback: {outcome:?}")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "entry"
            );
            assert_eq!(arguments, vec![Value::Int(value), Value::Int(value)]);
            outcome =
                machine.reply(&program, Reply::Value(Ok(Value::Int(value))));
        }
        assert!(
            matches!(outcome, Outcome::Done(Ok(Value::Tuple(values))) if values.as_slice() == [Value::Int(7), Value::Int(8)])
        );
    }
}

#[test]
fn default_replay_witnesses_survive_session_clone_and_withdrawal() {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub get: () -> a\npub type W<T> { pub value: T }\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub result: T = w.op }\npub after: Integer -> Integer\n").unwrap();
    let mut session = Session::new(declarations);
    let setup = ok(&mut session, "use host\nmake = w => host.R { w: w }");
    let setup_program = compile::compile(setup.unit.as_ref().unwrap());
    for mut branch in [session.clone(), session] {
        let mut machine = Machine::new(Some(100_000));
        assert!(matches!(machine.run(&setup_program), Outcome::Done(Ok(_))));
        for valid in [true, false, true] {
            let report =
                ok(&mut branch, "r = make (host.W 1)\nhost.after r.result");
            let program = compile::compile(report.unit.as_ref().unwrap());
            let Outcome::Request(Request::Call {
                declaration,
                arguments,
                ..
            }) = machine.run(&program)
            else {
                panic!("get request")
            };
            assert_eq!(
                program.tables.borrow().declarations[declaration].label,
                "get"
            );
            assert_eq!(arguments, [Value::Unit]);
            let result = machine.reply(
                &program,
                Reply::Value(Ok(if valid {
                    Value::Int(7)
                } else {
                    Value::text("wrong")
                })),
            );
            if valid {
                let Outcome::Request(Request::Call {
                    declaration,
                    arguments,
                    ..
                }) = result
                else {
                    panic!("after request: {result:?}")
                };
                assert_eq!(
                    program.tables.borrow().declarations[declaration].label,
                    "after"
                );
                assert_eq!(arguments, [Value::Int(7)]);
                assert_eq!(
                    machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
                    Outcome::Done(Ok(Value::Int(7)))
                );
            } else {
                assert!(
                    matches!(result, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(ref message), .. })) if message.contains("Integer")),
                    "wrong reply before after: {result:?}"
                );
            }
            let removed = branch.withdraw(report.accepted.unwrap()).unwrap();
            machine.withdraw(&removed);
        }
    }
}

fn assert_unfixed_default_callable(
    function: &fipe::value::Closure,
    calls: &[(fipe::ir::Type, Vec<fipe::ir::Type>)],
    result_template: &fipe::ir::Type,
    stored_program: &fipe::mir::Program,
) -> (usize, std::collections::BTreeSet<usize>) {
    use fipe::ir::Type as IrType;
    let mut pending: Vec<_> = function.meanings.iter().collect();
    for (destination, meanings) in calls {
        pending.push(destination);
        pending.extend(meanings);
    }
    let mut weak = std::collections::BTreeSet::new();
    while let Some(ty) = pending.pop() {
        match ty {
            IrType::Session(slot) => {
                weak.insert(*slot);
            }
            IrType::Meaning { arguments, .. }
            | IrType::Tuple(arguments)
            | IrType::Packed(arguments)
            | IrType::All(arguments)
            | IrType::Sum(arguments) => pending.extend(arguments),
            IrType::Function { parameter, result } => {
                pending.push(parameter);
                pending.push(result);
            }
            IrType::Array { element, length } => {
                pending.push(element);
                pending.push(length);
            }
            IrType::Not(inner) => pending.push(inner),
            _ => (),
        }
    }
    assert_eq!(weak.len(), 1, "must observe one unresolved witness in the actual callable carrier, not merely an unused table slot");
    let slot = *weak.first().unwrap();
    assert_eq!(slot, 0);
    assert_eq!(result_template, &IrType::Session(slot));
    assert!(stored_program.tables.borrow().session_meanings[slot].is_none());
    (slot, weak)
}

fn exercise_default_session_fix(
    branch: &mut Session,
    machine: &mut Machine,
    original_function: &fipe::value::Closure,
    captured_meanings: &[fipe::ir::Type],
    calls: &[(fipe::ir::Type, Vec<fipe::ir::Type>)],
    slot: usize,
    healthy: bool,
) {
    use fipe::ir::Type as IrType;
    let later = ok(branch, "x: Integer = f ()\nhost.after x");
    let program = compile::compile(later.unit.as_ref().unwrap());
    let tables = program.tables.borrow();
    let Some(IrType::Meaning { index, arguments }) =
        &tables.session_meanings[slot]
    else {
        panic!("later legitimate result constraint fixes the captured Session slot")
    };
    assert_eq!(tables.meanings[*index].label, "Integer");
    assert!(arguments.is_empty());
    drop(tables);
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("get from the retained default callable")
    };
    assert_eq!(
        program.tables.borrow().declarations[declaration].label,
        "get"
    );
    assert_eq!(arguments, [Value::Unit]);
    let joined = machine
        .program()
        .expect("the saved callable's joined body survives fixing");
    let body = &joined.bodies[original_function.body];
    let current_calls: Vec<_> = body
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            fipe::mir::Terminator::Call {
                dest: 0, meanings, ..
            } => Some((body.locals[0].ty.clone(), meanings.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(current_calls.as_slice(), calls, "later fixing resolves the shared Session slot rather than rewriting the saved executable carrier");
    let result = machine.reply(
        &program,
        Reply::Value(Ok(if healthy {
            Value::Int(7)
        } else {
            Value::text("wrong")
        })),
    );
    if healthy {
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = result
        else {
            panic!("after healthy reply: {result:?}")
        };
        assert_eq!(
            program.tables.borrow().declarations[declaration].label,
            "after"
        );
        assert_eq!(arguments, [Value::Int(7)]);
        assert_eq!(
            machine.reply(&program, Reply::Value(Ok(Value::Int(7)))),
            Outcome::Done(Ok(Value::Int(7)))
        );
    } else {
        assert!(
            matches!(result, Outcome::Done(Err(fipe::vm::Fault { reason: fipe::value::Reason::Host(ref message), .. })) if message.contains("Integer")),
            "wrong reply refused before after: {result:?}"
        );
    }
    let removed = branch.withdraw(later.accepted.unwrap()).unwrap();
    machine.withdraw(&removed);
    let query = done(branch.query("f"));
    assert!(query.diagnostics.is_empty());
    assert!(
        query
            .unit
            .as_ref()
            .expect("the retained callable remains usable")
            .tables()
            .session_meanings[slot]
            .is_none(),
        "withdrawing the fixing line restores the unresolved witness"
    );
    assert_eq!(original_function.meanings.as_slice(), captured_meanings, "stored callable retains its original weak witness rather than being eagerly rewritten");
}

#[test]
fn default_callable_keeps_unfixed_session_witness_until_later_fix() {
    let mut declarations = Declarations::new();
    declarations.module("host", "pub type W<T> { pub value: T }\npub open: W<Integer> -> W<a>\npub get: () -> a\npub save: a -> ()\nimpl W<a> { op: Self -> a = self => get () }\npub type R<T> { pub w: W<T>, pub later: () -> T = (_ => w.op) }\npub after: Integer -> Integer\n").unwrap();
    let mut session = Session::new(declarations);
    let setup = ok(&mut session, "use host\nmake = w => host.R { w: w }");
    let setup_program = compile::compile(setup.unit.as_ref().unwrap());
    for mut branch in [session.clone(), session] {
        let mut machine = Machine::new(Some(100_000));
        assert!(matches!(machine.run(&setup_program), Outcome::Done(Ok(_))));
        let opened = ok(&mut branch, "[w] = [host.open (host.W 1)]");
        assert!(
            !opened.typed.as_ref().unwrap().session_variables.is_empty(),
            "the opened receiver really has an unresolved Session argument"
        );
        let open_program = compile::compile(opened.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&open_program)
        else {
            panic!("open")
        };
        assert_eq!(
            open_program.tables.borrow().declarations[declaration].label,
            "open"
        );
        let [original_receiver] = arguments.as_slice() else {
            panic!("one genuine checked receiver")
        };
        let original_receiver = original_receiver.clone();
        assert!(matches!(&original_receiver, Value::Record(_)));
        assert!(matches!(
            machine.reply(&open_program, Reply::Value(Ok(original_receiver))),
            Outcome::Done(Ok(_))
        ));

        let stored = ok(&mut branch, "[f] = [(make w).later]\nhost.save f");
        let stored_program = compile::compile(stored.unit.as_ref().unwrap());
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&stored_program)
        else {
            panic!("save default-produced callable")
        };
        assert_eq!(
            stored_program.tables.borrow().declarations[declaration].label,
            "save"
        );
        let [Value::Function(function)] = arguments.as_slice() else {
            panic!("actual default-produced function")
        };
        let original_function = std::rc::Rc::clone(function);
        let captured_meanings = original_function.meanings.clone();
        let joined = machine
            .program()
            .expect("authoritative saved callable body");
        let body = &joined.bodies[function.body];
        let result_template = body.locals[0].ty.clone();
        let calls: Vec<_> = body
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                fipe::mir::Terminator::Call {
                    dest: 0, meanings, ..
                } => Some((body.locals[0].ty.clone(), meanings.clone())),
                _ => None,
            })
            .collect();
        assert!(
            !calls.is_empty(),
            "the saved callable actually calls into its result destination"
        );
        let (slot, _weak) = assert_unfixed_default_callable(
            function,
            &calls,
            &result_template,
            &stored_program,
        );
        assert_eq!(
            machine.reply(&stored_program, Reply::Value(Ok(Value::Unit))),
            Outcome::Done(Ok(Value::Unit))
        );

        for healthy in [true, false, true] {
            exercise_default_session_fix(
                &mut branch,
                &mut machine,
                &original_function,
                &captured_meanings,
                &calls,
                slot,
                healthy,
            );
        }
    }
}

#[test]
fn prior_line_defaults_keep_binding_sites_across_shadow_clone_and_withdrawal() {
    // Session withdrawal retains only the latest undo, so independently
    // exercise evaluation under shadowing and withdrawal of that shadow.
    for evaluate_shadowed in [true, false] {
        let (mut session, mut machine) = recovery_session();
        run_recovery_line(
            &mut session,
            &mut machine,
            "type R { a: Integer = keep, b: Integer = (a + keep) }",
        );
        let shadow = ok(&mut session, "keep = 99");
        let token = shadow.accepted.unwrap();
        assert_eq!(
            machine.run(&compile::compile(shadow.unit.as_ref().unwrap())),
            Outcome::Done(Ok(Value::Unit))
        );
        if evaluate_shadowed {
            assert_eq!(
                run_recovery_line(&mut session, &mut machine, "(R).b"),
                Value::Int(14),
                "same-spelled shadow cannot change an older captured site"
            );
            continue;
        }
        let mut cloned = session.clone();
        for branch in [&mut session, &mut cloned] {
            let before = branch.source().to_owned();
            let report = done(branch.query("(R).b"));
            assert_eq!(report.diagnostics, []);
            assert!(
                report.unit.is_some(),
                "old declaration aliases remain available"
            );
            assert_eq!(branch.source(), before, "query cannot publish aliases");
        }
        assert!(cloned.withdraw(token).is_some());
        assert!(done(cloned.query("(R).b")).unit.is_some());
        let withdrawn = session.withdraw(token).unwrap();
        machine.withdraw(&withdrawn);
        assert_eq!(
            run_recovery_line(&mut session, &mut machine, "keep"),
            Value::Int(7)
        );
        assert_eq!(
            run_recovery_line(&mut session, &mut machine, "(R).b"),
            Value::Int(14)
        );
    }
}

#[test]
fn receiver_locator_keeps_live_bound_reads_and_discards_removed_rows_before_reuse(
) {
    let mut session = Session::new(Declarations::new());
    let mut machine = Machine::new(Some(100_000));
    run_recovery_line(&mut session, &mut machine, "type M { n: Integer }");
    run_recovery_line(
        &mut session,
        &mut machine,
        "impl M { op = a b self => self.n + a * 10 + b }",
    );
    run_recovery_line(
        &mut session,
        &mut machine,
        "make = x => (_ => x.op 3 4)\nlater = make (M 2)",
    );
    let Value::Function(saved) =
        run_recovery_line(&mut session, &mut machine, "later")
    else {
        panic!("actual retained generic callable")
    };
    let joined = machine.program().expect("joined saved body");
    assert!(joined.bodies[saved.body].blocks.iter().any(|block| matches!(
        &block.terminator,
        fipe::mir::Terminator::Call { callee: fipe::mir::Callee::Dispatch { witness: fipe::mir::Witness::Value(receiver), .. }, args, .. }
            if args.as_slice() == [*receiver]
    )), "saved callable must perform a new receiver-only bound read on invocation");
    let tail = ok(&mut session,
        "type Gone { n: Integer }\nimpl Gone { op = a b self => self.n + a * 10 + b + 1000 }\nmake (Gone 1)");
    let mut token = tail.accepted.unwrap();
    let program = compile::compile(tail.unit.as_ref().unwrap());
    assert!(matches!(
        machine.run(&program),
        Outcome::Done(Ok(Value::Function(_)))
    ));
    for extra in [100, 200] {
        let (live, removed) = {
            let joined = machine.program().unwrap();
            let tables = joined.tables.borrow();
            let meaning = |label: &str| {
                tables
                    .meanings
                    .iter()
                    .position(|meaning| meaning.label == label)
                    .unwrap()
            };
            let m = meaning("M");
            let gone = meaning("Gone");
            let matching = |index| {
                joined.receiver_adapters.iter().filter(|adapter| matches!(
                &tables.dispatch[adapter.member].rows[adapter.row].receiver,
                fipe::ir::Type::Meaning { index: actual, .. } if *actual == index
            )).cloned().collect::<Vec<_>>()
            };
            (matching(m), matching(gone))
        };
        assert!(
            !live.is_empty() && !removed.is_empty(),
            "both live and withdrawn creation locators are real"
        );
        let withdrawn = session.withdraw(token).expect("latest tail");
        machine.withdraw(&withdrawn);
        let joined = machine.program().unwrap();
        assert!(
            live.iter()
                .all(|adapter| joined.receiver_adapters.contains(adapter)),
            "live prefix creation locators must survive for the retained body"
        );
        assert!(
            removed.iter().all(|adapter| !joined
                .receiver_adapters
                .iter()
                .any(|current| current.member == adapter.member
                    && current.row == adapter.row)),
            "removed locators must disappear before row indices can be reused"
        );
        let report = ok(&mut session, &format!(
            "type Gone {{ n: Integer }}\nimpl Gone {{ op = a b self => self.n + a * 10 + b + {extra} }}\n(later (), (make (Gone 1)) ())"));
        token = report.accepted.unwrap();
        let program = compile::compile(report.unit.as_ref().unwrap());
        assert!(removed.iter().any(|old| program.receiver_adapters.iter().any(|new| old.member == new.member && old.row == new.row)), "the replacement must actually reuse a removed locator's row position");
        assert_eq!(
            machine.run(&program),
            Outcome::Done(Ok(Value::Tuple(std::rc::Rc::new(vec![
                Value::Int(36),
                Value::Int(35 + extra)
            ]))))
        );
    }
}

const SESSION_GUARD_HOST: &str = "pub type Box<T>: Ord { pub payload: Vec<T>, pub rank: Integer }\nimpl Box<a>: Eq { eq: Self -> (self: Self) -> Bool = other self => self.rank == other.rank }\nimpl Box<a>: Ord { cmp: Self -> (self: Self) -> Ordering = other self => self.rank.cmp other.rank }\npub relay: a -> a\npub items: a -> a\npub after: () -> Integer\npub load: () -> a\n";
const SESSION_GUARD_SETUP: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => host.after ()\nmake_guard: Ord a => [a; 2] -> (Integer -> Sorted<a> -> Integer) = sample => host.relay keep<a>\n";
const SESSION_GUARD_WEAK: &str = "[boxes] = [[host.Box [] 1, host.Box [] 2]]\n";
const SESSION_GUARD_STORE: &str = "[f] = [make_guard boxes]\n";
const SESSION_GUARD_FIX_CALL: &str =
    "fixed: [host.Box<Integer>; 2] = boxes\nloaded: Integer -> Vec<host.Box<Integer>> -> Integer = host.load ()\nloaded 7 (host.items (fixed | to<Vec<host.Box<Integer>>>))\n";
struct SessionGuardSaved {
    function: std::rc::Rc<fipe::value::Closure>,
    guard: fipe::value::Guard,
    slot: usize,
    test: usize,
    creator: usize,
    definition: usize,
}

fn session_guard_line(
    session: &mut Session,
    source: &str,
    stage: &str,
) -> Report {
    let report = done(session.line(source));
    assert!(
        report.diagnostics.is_empty(),
        "{stage}: {:?}",
        report.diagnostics
    );
    assert!(report.accepted.is_some(), "{stage}: line must be accepted");
    report
}

fn session_guard_test_arguments(
    predicate: &fipe::ir::Predicate,
    test: usize,
) -> Option<&[fipe::ir::Type]> {
    use fipe::ir::Predicate;
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            Predicate::All(parts) | Predicate::Any(parts) => {
                pending.extend(parts);
            }
            Predicate::Not(inner) => pending.push(inner),
            Predicate::Function { parameter, result } => {
                pending.push(result);
                pending.push(parameter);
            }
            Predicate::Test { index, meanings } if *index == test => {
                return Some(meanings)
            }
            Predicate::Nonzero { .. } | Predicate::Test { .. } => {}
        }
    }
    None
}

fn session_guard_source_test(program: &fipe::mir::Program) -> usize {
    let tables = program.tables.borrow();
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "line 1"
                && SESSION_GUARD_SETUP.get(test.span.start..test.span.end)
                    == Some("is_sorted self"))
            .then_some(index)
        })
        .collect();
    assert_eq!(
        tests.len(),
        1,
        "stage setup identity: one actual line1 Sorted Test"
    );
    tests[0]
}

fn assert_session_guard_box(
    ty: &fipe::ir::Type,
    tables: &fipe::ir::Tables,
    fixed: Option<usize>,
) -> usize {
    use fipe::ir::Type as IrType;
    let IrType::Meaning { index, arguments } = ty else {
        panic!("actual captured Box meaning: {ty:?}")
    };
    assert_eq!(tables.meanings[*index].label, "Box");
    assert_eq!(tables.namespaces[tables.meanings[*index].namespace], "host");
    let [argument] = arguments.as_slice() else {
        panic!("one actual Box argument: {arguments:?}")
    };
    fixed.map_or_else(|| {
            let IrType::Session(slot) = argument else {
                panic!("actual unresolved Session inside captured Box, not an unused table slot: {ty:?}")
            };
            assert!(tables.session_meanings[*slot].is_none());
            *slot
        }, |slot| {
            let IrType::Meaning { index, arguments } = argument else {
                panic!("current test receives Box<Integer>: {ty:?}")
            };
            assert_eq!(tables.meanings[*index].label, "Integer");
            assert_eq!(
                tables.namespaces[tables.meanings[*index].namespace],
                "prelude"
            );
            assert!(arguments.is_empty());
            assert!(matches!(
                tables.session_meanings[slot],
                Some(IrType::Meaning { .. })
            ));
            slot
        })
}

fn observe_session_guard_store(
    machine: &Machine,
    arguments: &[Value],
) -> SessionGuardSaved {
    use fipe::{ir, vm::Frame};
    let [Value::Function(function)] = arguments else {
        panic!("stage attachment: relay must receive actual Function")
    };
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    let test = session_guard_source_test(program);
    let node = &tables.nodes[program.bodies[function.body].node];
    assert_eq!(tables.namespaces[node.namespace], "line 1");
    assert_eq!(
        SESSION_GUARD_SETUP.get(node.span.start..node.span.end),
        Some("n xs => host.after ()"),
        "stage attachment: raw keep body, not generated wrapper"
    );
    assert_eq!(program.bodies[function.body].arity, 2);
    assert!(function.filled.is_empty());
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            session_guard_test_arguments(&guard.predicate, test).is_some()
        })
        .unwrap_or_else(|| {
            panic!(
                "stage attachment: actual semantic Sorted Guard absent: {:?}",
                function.guards
            )
        });
    assert_eq!(guard.spot, 1);
    assert!(
        !guard.witnesses.is_empty(),
        "stage attachment: actual captured associations required"
    );
    let creator_definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "make_guard"
                && tables.namespaces[definition.namespace] == "line 1"
        })
        .unwrap();
    let creator =
        program.definitions[creator_definition].expect("actual creator body");
    let frame = machine.frames().iter().find_map(|frame| match frame {
        Frame::Body(frame) if frame.body == creator => Some(frame), _ => None,
    }).expect("stage attachment: actual make_guard creator frame still owns request");
    assert_eq!(program.bodies[creator].hidden.len(), frame.meanings.len());
    let mut slots = std::collections::BTreeSet::new();
    for template in
        session_guard_test_arguments(&guard.predicate, test).unwrap()
    {
        let ir::Type::Parameter(parameter) = template else {
            panic!(
                "creator template must use its actual Parameter: {template:?}"
            )
        };
        assert!(program.bodies[creator].hidden.contains(parameter));
        let (_, meaning) = guard
            .witnesses
            .iter()
            .find(|(id, _)| id == parameter)
            .unwrap();
        slots.insert(assert_session_guard_box(
            meaning.as_type(),
            &tables,
            None,
        ));
    }
    assert_eq!(
        slots.len(),
        1,
        "stage attachment: exactly one live captured weak slot"
    );
    SessionGuardSaved {
        function: std::rc::Rc::clone(function),
        guard: guard.clone(),
        slot: *slots.first().unwrap(),
        test,
        creator,
        definition: usize::MAX,
    }
}

fn assert_session_guard_saved_value(value: &Value, saved: &SessionGuardSaved) {
    let Value::Function(function) = value else {
        panic!("retained f is an actual Function")
    };
    assert_eq!(function.body, saved.function.body);
    assert!(function.filled.is_empty());
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            guard.entry == saved.guard.entry
                && guard.spot == saved.guard.spot
                && session_guard_test_arguments(&guard.predicate, saved.test)
                    .is_some()
        })
        .expect("same retained semantic guard");
    assert!(std::rc::Rc::ptr_eq(
        &guard.witnesses,
        &saved.guard.witnesses
    ));
    assert_eq!(guard.witnesses.len(), saved.guard.witnesses.len());
    for ((id, witness), (old_id, old)) in
        guard.witnesses.iter().zip(saved.guard.witnesses.iter())
    {
        assert_eq!(id, old_id);
        assert!(
            std::rc::Rc::ptr_eq(witness, old),
            "individual owned Meaning Rc is retained"
        );
    }
}

fn assert_session_guard_global(machine: &Machine, saved: &SessionGuardSaved) {
    let (_, value) = machine
        .globals()
        .find(|(definition, _)| *definition == saved.definition)
        .expect("saved language binding f remains in the accepted prefix");
    assert_session_guard_saved_value(value, saved);
}

fn create_session_guard(
    session: &mut Session,
    machine: &mut Machine,
) -> SessionGuardSaved {
    let setup = session_guard_line(
        session,
        SESSION_GUARD_SETUP,
        "stage1 setup admission",
    );
    let setup_program = compile::compile(setup.unit.as_ref().unwrap());
    assert!(
        matches!(machine.run(&setup_program), Outcome::Done(Ok(_))),
        "stage1 setup runtime"
    );
    let weak = session_guard_line(
        session,
        SESSION_GUARD_WEAK,
        "stage2 weak Box admission",
    );
    assert!(
        !weak.typed.as_ref().unwrap().session_variables.is_empty(),
        "stage2 actual weak variables"
    );
    let weak_program = compile::compile(weak.unit.as_ref().unwrap());
    assert!(
        matches!(machine.run(&weak_program), Outcome::Done(Ok(_))),
        "stage2 weak value construction"
    );
    let stored = session_guard_line(
        session,
        SESSION_GUARD_STORE,
        "stage3 guard store admission",
    );
    let program = compile::compile(stored.unit.as_ref().unwrap());
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        ..
    }) = machine.run(&program)
    else {
        panic!("stage3 runtime must request relay with the actual guarded keep")
    };
    assert_eq!(
        machine.program().unwrap().tables.borrow().declarations[declaration]
            .label,
        "relay"
    );
    let mut saved = observe_session_guard_store(machine, &arguments);
    let result =
        machine.reply(&program, Reply::Value(Ok(arguments[0].clone())));
    assert!(
        matches!(result, Outcome::Done(Ok(_))),
        "stage3 exact Function reply/store: {result:?}"
    );
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let site = stored.offset + SESSION_GUARD_STORE.find('f').unwrap();
    saved.definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "f"
                && definition.span.start == site
                && definition.span.end == site + 1
        })
        .expect("actual source f definition");
    drop(tables);
    assert_session_guard_global(machine, &saved);
    saved
}

fn assert_session_guard_fixed_slot(tables: &fipe::ir::Tables, slot: usize) {
    let Some(fipe::ir::Type::Meaning { index, arguments }) =
        &tables.session_meanings[slot]
    else {
        panic!("stage fix: actual captured Session slot resolves Integer")
    };
    assert_eq!(tables.meanings[*index].label, "Integer");
    assert_eq!(
        tables.namespaces[tables.meanings[*index].namespace],
        "prelude"
    );
    assert!(arguments.is_empty());
}

fn session_guard_clone_isolation(
    session: &Session,
    machine: &Machine,
    saved: &SessionGuardSaved,
) -> Session {
    let mut fork = session.clone();
    let forked = session_guard_line(
        &mut fork,
        "fixed: [host.Box<Integer>; 2] = boxes",
        "stage4 metadata clone fix",
    );
    let fork_program = compile::compile(forked.unit.as_ref().unwrap());
    let original = machine.program().unwrap();
    assert!(
        !std::rc::Rc::ptr_eq(&original.tables, &fork_program.tables),
        "Session clone has separate compiler tables; Machine is not cloned"
    );
    assert_session_guard_fixed_slot(&fork_program.tables.borrow(), saved.slot);
    let tables = original.tables.borrow();
    assert!(
        tables.session_meanings[saved.slot].is_none(),
        "clone fix must not mutate active original weak slot"
    );
    for (_, meaning) in saved.guard.witnesses.iter() {
        assert_eq!(
            assert_session_guard_box(meaning.as_type(), &tables, None),
            saved.slot
        );
    }
    assert_session_guard_global(machine, saved);
    fork
}

#[derive(Clone, Copy, Debug)]
enum SessionGuardVerdict {
    HostFault,
    SortedFault,
    Healthy,
}

fn session_guard_fix_reply(
    machine: &Machine,
    request: &Request,
    saved: &SessionGuardSaved,
    verdict: SessionGuardVerdict,
    requests: &mut Vec<String>,
) -> Value {
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("stage5 actual declared request")
    };
    let label = machine.program().unwrap().tables.borrow().declarations
        [*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "load" => {
            assert_eq!(requests.as_slice(), ["load"]);
            assert_eq!(arguments, &[Value::Unit]);
            assert_session_guard_fixed_slot(
                &machine.program().unwrap().tables.borrow(),
                saved.slot,
            );
            assert_session_guard_global(machine, saved);
            // This exact observed Function already belongs to this Machine's
            // retained program; no closure or Guard is manufactured.
            Value::Function(std::rc::Rc::clone(&saved.function))
        }
        "items" => {
            assert_eq!(requests.as_slice(), ["load", "items"]);
            let [Value::Seq(values)] = arguments.as_slice() else {
                panic!("actual checked Box vector")
            };
            assert_eq!(values.len(), 2);
            let mut reply = values.as_ref().clone();
            if matches!(verdict, SessionGuardVerdict::SortedFault) {
                reply.reverse();
            }
            Value::seq(reply)
        }
        "after" => {
            assert!(
                !matches!(verdict, SessionGuardVerdict::SortedFault),
                "unsorted refuses before after"
            );
            assert_eq!(requests.as_slice(), ["load", "items", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            if matches!(verdict, SessionGuardVerdict::HostFault) {
                Value::text("wrong")
            } else {
                Value::Int(7)
            }
        }
        _ => panic!("unexpected effect {label}"),
    }
}
fn observe_session_guard_fix(
    machine: &Machine,
    saved: &SessionGuardSaved,
    offset: usize,
    observed: &mut [bool; 3],
) {
    use fipe::{mir, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return;
    };
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    let test_body =
        program.tests[saved.test].expect("current joined semantic Sorted body");
    if frame.body == test_body {
        assert!(observed[2], "stage5 captured Guard Test must run AFTER actual retained loaded call, never a masking caller-side entry");
        let hidden = &program.bodies[test_body].hidden;
        assert!(
            !hidden.is_empty(),
            "stage5 actual generic Sorted frame witnesses"
        );
        assert_eq!(hidden.len(), frame.meanings.len());
        for meaning in &frame.meanings {
            assert_session_guard_box(meaning, &tables, Some(saved.slot));
        }
        observed[0] = true;
    }
    if tables
        .definitions
        .iter()
        .enumerate()
        .any(|(index, definition)| {
            definition.label == "cmp"
                && tables.namespaces[definition.namespace] == "host"
                && program.definitions[index] == Some(frame.body)
        })
    {
        observed[1] = true;
    }
    // The shared IR call node can label argument-evaluation and return
    // blocks too. Only an actual main Value terminator is the call event.
    if frame.body != program.main {
        return;
    }
    let block = &program.bodies[frame.body].blocks[frame.block];
    if frame.statement != block.statements.len() {
        return;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(operand),
        args,
        ..
    } = &block.terminator
    else {
        return;
    };
    let Some(node) = block.nodes.last() else {
        return;
    };
    let site = &tables.nodes[*node];
    let start = offset + SESSION_GUARD_FIX_CALL.find("loaded 7").unwrap();
    let end = offset + SESSION_GUARD_FIX_CALL.trim_end().len();
    if site.span.start != start || site.span.end != end {
        return;
    }
    assert_eq!(tables.namespaces[site.namespace], "line 4");
    assert_eq!(args.len(), 2);
    let value = match operand {
        mir::Operand::Global(index) => {
            machine
                .globals()
                .find(|(definition, _)| definition == index)
                .unwrap()
                .1
        }
        mir::Operand::Local(index) => &frame.locals[*index],
        mir::Operand::Capture(index) => &frame.captures[*index],
        mir::Operand::Const(index) => &program.constants[*index],
        mir::Operand::Hidden(_) => panic!("source f is not a witness operand"),
    };
    assert_session_guard_saved_value(value, saved);
    observed[2] = true;
}

fn assert_session_guard_fix_result(
    machine: &Machine,
    saved: &SessionGuardSaved,
    verdict: SessionGuardVerdict,
    result: Result<Value, fipe::vm::Fault>,
    requests: &[String],
    observed: [bool; 3],
) {
    use fipe::value::Reason as ValueReason;
    assert!(observed.into_iter().all(|seen| seen), "stage5 post-call captured Sorted Box<Integer>/written cmp/retained loaded call: {observed:?}");
    match verdict {
        SessionGuardVerdict::Healthy => {
            assert_eq!(result, Ok(Value::Int(7)));
            assert_eq!(requests, ["load", "items", "after"]);
        }
        SessionGuardVerdict::HostFault => {
            let fault = result
                .expect_err("stage5 deliberately wrong after reply faults");
            assert!(
                matches!(&fault.reason, ValueReason::Host(message) if message.contains("Integer")),
                "{fault:?}"
            );
            assert_eq!(requests, ["load", "items", "after"]);
        }
        SessionGuardVerdict::SortedFault => {
            let fault = result.expect_err(
                "stage5 reversed sequence faults at actual Sorted test",
            );
            let ValueReason::Failed { entry, .. } = &fault.reason else {
                panic!("{fault:?}")
            };
            let program = machine.program().unwrap();
            assert!(session_guard_test_arguments(&program.tables.borrow().entries[*entry].predicate, saved.test).is_some(),
                "failure must name same semantic source Test, not just a display label");
            assert_eq!(requests, ["load", "items"]);
        }
    }
    assert_session_guard_global(machine, saved);
}

fn exercise_session_guard_fix(
    session: &mut Session,
    machine: &mut Machine,
    saved: &SessionGuardSaved,
    verdict: SessionGuardVerdict,
) -> Report {
    use fipe::vm::Stop;
    let report = session_guard_line(
        session,
        SESSION_GUARD_FIX_CALL,
        "stage5 fixing/invocation admission",
    );
    let program = compile::compile(report.unit.as_ref().unwrap());
    assert_session_guard_fixed_slot(&program.tables.borrow(), saved.slot);
    machine.begin(&program);
    let joined = machine.program().unwrap();
    assert_eq!(
        session_guard_source_test(joined),
        saved.test,
        "semantic source identity survives later program joins"
    );
    assert_session_guard_fixed_slot(&joined.tables.borrow(), saved.slot);
    assert_no_session_sorted_argument_entry(joined, saved.test);
    let creator = &joined.bodies[saved.creator];
    let tables = joined.tables.borrow();
    let node = &tables.nodes[creator.node];
    assert_eq!(tables.namespaces[node.namespace], "line 1");
    assert_eq!(
        SESSION_GUARD_SETUP.get(node.span.start..node.span.end),
        Some("sample => host.relay keep<a>")
    );
    for (_, meaning) in saved.guard.witnesses.iter() {
        let fipe::ir::Type::Meaning { arguments, .. } = meaning.as_type()
        else {
            panic!("captured Box root")
        };
        assert!(matches!(arguments.as_slice(), [fipe::ir::Type::Session(slot)] if *slot == saved.slot),
            "stored witness is immutable Session template, not eagerly rewritten");
    }
    drop(tables);
    let mut requests = Vec::new();
    let mut observed = [false; 3];
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_session_guard_fix(machine, saved, report.offset, &mut observed);
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = session_guard_fix_reply(
                    machine,
                    &request,
                    saved,
                    verdict,
                    &mut requests,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert_session_guard_fix_result(
                    machine, saved, verdict, result, &requests, observed,
                );
                return report;
            }
        }
    }
    panic!("stage5 bounded guarded invocation must complete or fault")
}

fn withdraw_session_guard_fix(
    session: &mut Session,
    machine: &mut Machine,
    saved: &SessionGuardSaved,
    report: &Report,
) {
    let removed = session
        .withdraw(report.accepted.expect("latest accepted fixing line"))
        .expect("stage6 only the latest faulted fixing line is withdrawn");
    machine.withdraw(&removed);
    let unknown = done(session.query("fixed"));
    assert!(
        !unknown_name(&unknown).is_empty(),
        "stage6 withdrawn fixed is an ordinary unknown name"
    );
    let unknown_loaded = done(session.query("loaded"));
    assert!(
        !unknown_name(&unknown_loaded).is_empty(),
        "stage6 withdrawn loaded is an ordinary unknown name"
    );
    let retained = done(session.query("f"));
    assert!(
        retained.diagnostics.is_empty(),
        "stage6 creator f remains in accepted prefix"
    );
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    assert!(
        tables.session_meanings[saved.slot].is_none(),
        "stage6 rollback restores unresolved weak slot"
    );
    for (_, meaning) in saved.guard.witnesses.iter() {
        assert_eq!(
            assert_session_guard_box(meaning.as_type(), &tables, None),
            saved.slot
        );
    }
    assert_session_guard_global(machine, saved);
}

#[test]
fn predicate_guard_session_witnesses_follow_clone_fix_withdraw_and_retry() {
    let mut declarations = Declarations::new();
    declarations
        .module("host", SESSION_GUARD_HOST)
        .expect("stage0 lawful rank-only Host Box Eq/Ord admission");
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(Some(1 << 20));
    let saved = create_session_guard(&mut session, &mut machine);
    let _fork = session_guard_clone_isolation(&session, &machine, &saved);
    for verdict in [
        SessionGuardVerdict::HostFault,
        SessionGuardVerdict::SortedFault,
    ] {
        let report = exercise_session_guard_fix(
            &mut session,
            &mut machine,
            &saved,
            verdict,
        );
        withdraw_session_guard_fix(&mut session, &mut machine, &saved, &report);
    }
    let _healthy = exercise_session_guard_fix(
        &mut session,
        &mut machine,
        &saved,
        SessionGuardVerdict::Healthy,
    );
    assert_session_guard_global(&machine, &saved);
}

fn assert_no_session_sorted_argument_entry(
    program: &fipe::mir::Program,
    test: usize,
) {
    let tables = program.tables.borrow();
    for block in &program.bodies[program.main].blocks {
        for statement in &block.statements {
            if let fipe::mir::Statement::Enter { entry, .. } = statement {
                assert!(session_guard_test_arguments(&tables.entries[*entry].predicate, test).is_none(),
                    "ordinary loaded signature must not insert a caller-side Sorted gate: {:?}", tables.entries[*entry]);
            }
        }
    }
}

const SESSION_EMPTY_HOST: &str = "pub type Box<T>: Ord { pub payload: Vec<T>, pub rank: Integer }\nimpl Box<a>: Eq { eq: Self -> (self: Self) -> Bool = other self => self.rank == other.rank }\nimpl Box<a>: Ord { cmp: Self -> (self: Self) -> Ordering = other self => self.rank.cmp other.rank }\npub relay: a -> a\npub items: [a; 2] -> Vec<a>\npub after: () -> Integer\npub load: () -> a\n";
const SESSION_EMPTY_SETUP: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => host.after ()\napply_one = fn x y => {\n partial = fn x\n next = host.relay partial\n next y\n}\n";
const SESSION_EMPTY_PRODUCE: &str = "apply_one keep 7 (host.items boxes)\n";
const SESSION_EMPTY_FIX_CALL: &str = "fixed: [host.Box<Integer>; 2] = boxes\nloaded: Vec<host.Box<Integer>> -> Integer = host.load ()\nloaded (host.items fixed)\n";
struct SessionEmptyGuardSaved {
    function: std::rc::Rc<fipe::value::Closure>,
    guard: fipe::value::Guard,
    slot: usize,
    test: usize,
}

fn assert_session_empty_template(
    program: &fipe::mir::Program,
    saved: &SessionEmptyGuardSaved,
    unresolved: bool,
) {
    use fipe::ir::Type as IrType;
    assert!(
        saved.guard.witnesses.is_empty(),
        "empty association is real, not absence of semantic Guard"
    );
    let arguments =
        session_guard_test_arguments(&saved.guard.predicate, saved.test)
            .unwrap();
    let [IrType::Meaning { index, arguments }] = arguments else {
        panic!(
            "direct Test Box<Session>, not a Parameter template: {arguments:?}"
        )
    };
    let tables = program.tables.borrow();
    assert_eq!(tables.meanings[*index].label, "Box");
    assert_eq!(tables.namespaces[tables.meanings[*index].namespace], "host");
    assert!(
        matches!(arguments.as_slice(), [IrType::Session(slot)] if *slot == saved.slot),
        "immutable predicate must retain its live Session slot: {arguments:?}"
    );
    if unresolved {
        assert!(tables.session_meanings[saved.slot].is_none());
    } else {
        assert_session_guard_fixed_slot(&tables, saved.slot);
    }
}

fn assert_session_empty_partial(value: &Value, saved: &SessionEmptyGuardSaved) {
    let Value::Function(function) = value else {
        panic!("actual observed partial Function")
    };
    assert_eq!(function.body, saved.function.body);
    assert_eq!(function.filled, [Value::Int(7)]);
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            guard.entry == saved.guard.entry
                && guard.spot == 1
                && session_guard_test_arguments(&guard.predicate, saved.test)
                    .is_some()
        })
        .expect("same actual semantic Guard");
    assert!(guard.witnesses.is_empty());
    assert!(
        std::rc::Rc::ptr_eq(&guard.witnesses, &saved.guard.witnesses),
        "same empty association Rc through partial and Host receipt"
    );
    assert_eq!(
        session_guard_test_arguments(&guard.predicate, saved.test),
        session_guard_test_arguments(&saved.guard.predicate, saved.test)
    );
}

fn session_empty_helper(program: &fipe::mir::Program) -> (usize, usize) {
    use fipe::{ir, mir};
    let tables = program.tables.borrow();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "apply_one"
                && tables.namespaces[definition.namespace] == "line 1"
        })
        .unwrap();
    let body = program.definitions[definition].expect("actual helper body");
    assert_eq!(program.bodies[body].arity, 3);
    assert!(
        !program.bodies[body]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| {
                matches!(
                    statement,
                    mir::Statement::Assign(_, mir::Rvalue::Closure { .. })
                )
            }),
        "closed dynamic helper, not a generated wrapper"
    );
    let mut found = Vec::new();
    for block in &program.bodies[body].blocks {
        let mir::Terminator::Call {
            callee: mir::Callee::Value(_),
            args,
            dest,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let Some(node) = block.nodes.last() else {
            continue;
        };
        let site = &tables.nodes[*node];
        if tables.namespaces[site.namespace] != "line 1"
            || SESSION_EMPTY_SETUP.get(site.span.start..site.span.end)
                != Some("fn x")
        {
            continue;
        }
        assert_eq!(args.len(), 1);
        assert!(matches!(
            program.bodies[body].locals[*dest].ty,
            ir::Type::Function { .. }
        ));
        let ir::Kind::Call { open, .. } = &site.kind else {
            panic!("actual fn x IR call")
        };
        assert!(open.is_empty());
        found.push(*node);
    }
    assert_eq!(found.len(), 1);
    (body, found[0])
}

fn observe_session_empty_raw(
    machine: &Machine,
    helper: usize,
    call_node: usize,
    test: usize,
) -> Option<SessionEmptyGuardSaved> {
    use fipe::{mir, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return None;
    };
    if frame.body != helper {
        return None;
    }
    let program = machine.program().unwrap();
    let block = &program.bodies[helper].blocks[frame.block];
    if frame.statement != block.statements.len() {
        return None;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(operand),
        args,
        ..
    } = &block.terminator
    else {
        return None;
    };
    if block.nodes.last() != Some(&call_node) {
        return None;
    }
    let mir::Operand::Local(local) = operand else {
        panic!("actual source-linked fn x requires its Local operand")
    };
    assert_eq!(args.len(), 1);
    let Value::Function(function) = &frame.locals[*local] else {
        panic!("actual raw Function operand")
    };
    assert_eq!(program.bodies[function.body].arity, 2);
    assert!(function.filled.is_empty());
    let tables = program.tables.borrow();
    let node = &tables.nodes[program.bodies[function.body].node];
    assert_eq!(tables.namespaces[node.namespace], "line 1");
    assert_eq!(
        SESSION_EMPTY_SETUP.get(node.span.start..node.span.end),
        Some("n xs => host.after ()")
    );
    let guard = function
        .guards
        .iter()
        .find(|guard| {
            session_guard_test_arguments(&guard.predicate, test).is_some()
        })
        .expect("creation attachment: real semantic Guard before partial");
    assert_eq!(guard.spot, 1);
    assert!(guard.witnesses.is_empty(), "creation attachment: live Session does not invent captured Parameter IDs");
    let arguments =
        session_guard_test_arguments(&guard.predicate, test).unwrap();
    let [meaning] = arguments else {
        panic!("one direct Test argument")
    };
    let slot = assert_session_guard_box(meaning, &tables, None);
    Some(SessionEmptyGuardSaved {
        function: std::rc::Rc::clone(function),
        guard: guard.clone(),
        slot,
        test,
    })
}

fn assert_session_empty_items(
    machine: &Machine,
    arguments: &[Value],
) -> Vec<Value> {
    let [Value::Seq(values)] = arguments else {
        panic!("Host items receives actual checked array2")
    };
    assert_eq!(values.len(), 2);
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    for (value, expected) in values.iter().zip([1, 2]) {
        let Value::Record(record) = value else {
            panic!("actual checked Box record")
        };
        assert_eq!(tables.meanings[record.meaning].label, "Box");
        assert_eq!(
            tables.namespaces[tables.meanings[record.meaning].namespace],
            "host"
        );
        let [Value::Seq(payload), Value::Int(rank)] = record.fields.as_slice()
        else {
            panic!("lawful Box payload/rank")
        };
        assert!(payload.is_empty());
        assert_eq!(*rank, expected);
    }
    values.as_ref().clone()
}

fn session_empty_creation_reply(
    machine: &Machine,
    request: &Request,
    saved: &mut Option<SessionEmptyGuardSaved>,
    requests: &mut Vec<String>,
    test_seen: bool,
    cmp_seen: bool,
) -> Value {
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("actual creation request")
    };
    let label = machine.program().unwrap().tables.borrow().declarations
        [*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "items" => {
            assert_eq!(requests.as_slice(), ["items"]);
            Value::seq(assert_session_empty_items(machine, arguments))
        }
        "relay" => {
            assert_eq!(requests.as_slice(), ["items", "relay"]);
            let saved = saved
                .as_mut()
                .expect("raw empty-context Guard observed before partial");
            let [value @ Value::Function(function)] = arguments.as_slice()
            else {
                panic!("actual partial at relay")
            };
            assert_session_empty_partial(value, saved);
            saved.function = std::rc::Rc::clone(function);
            value.clone()
        }
        "after" => {
            assert_eq!(requests.as_slice(), ["items", "relay", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            assert!(test_seen && cmp_seen, "unfixed Session Test and written rank cmp execute before after");
            Value::Int(7)
        }
        _ => panic!("unexpected creation effect {label}"),
    }
}

fn session_empty_creation_observation(
    machine: &Machine,
    helper: usize,
    saved: &SessionEmptyGuardSaved,
    final_called: &mut bool,
    observed: &mut [bool; 2],
) {
    use fipe::{mir, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return;
    };
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    if frame.body == program.tests[saved.test].unwrap() {
        assert!(
            *final_called,
            "unfixed Test executes only after actual guarded next y call"
        );
        let hidden = &program.bodies[frame.body].hidden;
        assert!(!hidden.is_empty());
        assert_eq!(hidden.len(), frame.meanings.len());
        for meaning in &frame.meanings {
            assert_eq!(
                assert_session_guard_box(meaning, &tables, None),
                saved.slot
            );
        }
        observed[0] = true;
    }
    if tables
        .definitions
        .iter()
        .enumerate()
        .any(|(index, definition)| {
            definition.label == "cmp"
                && tables.namespaces[definition.namespace] == "host"
                && program.definitions[index] == Some(frame.body)
        })
    {
        observed[1] = true;
    }
    if frame.body != helper {
        return;
    }
    let block = &program.bodies[helper].blocks[frame.block];
    if frame.statement != block.statements.len() {
        return;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(mir::Operand::Local(local)),
        args,
        ..
    } = &block.terminator
    else {
        return;
    };
    let site = &tables.nodes[*block.nodes.last().unwrap()];
    if SESSION_EMPTY_SETUP.get(site.span.start..site.span.end) != Some("next y")
    {
        return;
    }
    assert_eq!(args.len(), 1);
    assert_session_empty_partial(&frame.locals[*local], saved);
    *final_called = true;
}

fn create_session_empty_guard(
    session: &mut Session,
    machine: &mut Machine,
) -> SessionEmptyGuardSaved {
    use fipe::vm::Stop;
    let setup =
        session_guard_line(session, SESSION_EMPTY_SETUP, "empty stage1 setup");
    assert!(matches!(
        machine.run(&compile::compile(setup.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
    let weak = session_guard_line(
        session,
        SESSION_GUARD_WEAK,
        "empty stage2 weak Box array",
    );
    assert!(!weak.typed.as_ref().unwrap().session_variables.is_empty());
    assert!(matches!(
        machine.run(&compile::compile(weak.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
    let produced = session_guard_line(
        session,
        SESSION_EMPTY_PRODUCE,
        "empty stage3 producer admission",
    );
    let program = compile::compile(produced.unit.as_ref().unwrap());
    machine.begin(&program);
    let joined = machine.program().unwrap();
    let test = session_guard_source_test(joined);
    let (helper, call_node) = session_empty_helper(joined);
    let mut saved = None;
    let mut final_called = false;
    let mut observed = [false; 2];
    let mut requests = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if let Some(saved) = &saved {
            session_empty_creation_observation(
                machine,
                helper,
                saved,
                &mut final_called,
                &mut observed,
            );
        }
        if matches!(&stop, Stop::Before(_)) {
            if let Some(value) =
                observe_session_empty_raw(machine, helper, call_node, test)
            {
                assert!(
                    saved.replace(value).is_none(),
                    "one observed runtime partial transition"
                );
            }
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = session_empty_creation_reply(
                    machine,
                    &request,
                    &mut saved,
                    &mut requests,
                    observed[0],
                    observed[1],
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert_eq!(
                    result,
                    Ok(Value::Int(7)),
                    "empty stage3 unfixed rank-only Guard run"
                );
                assert!(final_called && observed.into_iter().all(|seen| seen));
                assert_eq!(requests, ["items", "relay", "after"]);
                let saved = saved.expect(
                    "actual saved partial, not a manufactured Function",
                );
                assert_eq!(saved.function.filled, [Value::Int(7)]);
                assert_session_empty_template(
                    machine.program().unwrap(),
                    &saved,
                    true,
                );
                return saved;
            }
        }
    }
    panic!("empty stage3 bounded creation/partial/unfixed invocation")
}
fn session_empty_fix_reply(
    machine: &Machine,
    request: &Request,
    saved: &SessionEmptyGuardSaved,
    verdict: SessionGuardVerdict,
    requests: &mut Vec<String>,
) -> Value {
    let Request::Call {
        declaration,
        arguments,
        ..
    } = request
    else {
        panic!("empty fix actual request")
    };
    let label = machine.program().unwrap().tables.borrow().declarations
        [*declaration]
        .label
        .clone();
    requests.push(label.clone());
    match label.as_str() {
        "load" => {
            assert_eq!(requests.as_slice(), ["load"]);
            assert_eq!(arguments, &[Value::Unit]);
            assert_session_empty_template(
                machine.program().unwrap(),
                saved,
                false,
            );
            Value::Function(std::rc::Rc::clone(&saved.function))
        }
        "items" => {
            assert_eq!(requests.as_slice(), ["load", "items"]);
            let mut reply = assert_session_empty_items(machine, arguments);
            if matches!(verdict, SessionGuardVerdict::SortedFault) {
                reply.reverse();
            }
            Value::seq(reply)
        }
        "after" => {
            assert!(!matches!(verdict, SessionGuardVerdict::SortedFault));
            assert_eq!(requests.as_slice(), ["load", "items", "after"]);
            assert_eq!(arguments, &[Value::Unit]);
            if matches!(verdict, SessionGuardVerdict::HostFault) {
                Value::text("wrong")
            } else {
                Value::Int(7)
            }
        }
        _ => panic!("unexpected empty fix effect {label}"),
    }
}

fn observe_session_empty_fix(
    machine: &Machine,
    saved: &SessionEmptyGuardSaved,
    offset: usize,
    observed: &mut [bool; 3],
) {
    use fipe::{mir, vm::Frame};
    let Some(Frame::Body(frame)) = machine.frames().last() else {
        return;
    };
    let program = machine.program().unwrap();
    let tables = program.tables.borrow();
    if frame.body == program.tests[saved.test].unwrap() {
        assert!(observed[2], "empty fix: Test AFTER actual retained partial call, not source gate");
        assert!(!program.bodies[frame.body].hidden.is_empty());
        assert_eq!(
            program.bodies[frame.body].hidden.len(),
            frame.meanings.len()
        );
        for meaning in &frame.meanings {
            assert_session_guard_box(meaning, &tables, Some(saved.slot));
        }
        observed[0] = true;
    }
    if tables
        .definitions
        .iter()
        .enumerate()
        .any(|(index, definition)| {
            definition.label == "cmp"
                && tables.namespaces[definition.namespace] == "host"
                && program.definitions[index] == Some(frame.body)
        })
    {
        observed[1] = true;
    }
    if frame.body != program.main {
        return;
    }
    let block = &program.bodies[frame.body].blocks[frame.block];
    if frame.statement != block.statements.len() {
        return;
    }
    let mir::Terminator::Call {
        callee: mir::Callee::Value(operand),
        args,
        ..
    } = &block.terminator
    else {
        return;
    };
    let site = &tables.nodes[*block.nodes.last().unwrap()];
    if site.span.start
        != offset + SESSION_EMPTY_FIX_CALL.find("loaded (").unwrap()
        || site.span.end != offset + SESSION_EMPTY_FIX_CALL.trim_end().len()
    {
        return;
    }
    assert_eq!(tables.namespaces[site.namespace], "line 4");
    assert_eq!(
        args.len(),
        1,
        "actual partial needs only its remaining vector argument"
    );
    let value = match operand {
        mir::Operand::Global(index) => {
            machine
                .globals()
                .find(|(definition, _)| definition == index)
                .unwrap()
                .1
        }
        mir::Operand::Local(index) => &frame.locals[*index],
        mir::Operand::Capture(index) => &frame.captures[*index],
        mir::Operand::Const(index) => &program.constants[*index],
        mir::Operand::Hidden(_) => {
            panic!("loaded partial is not a witness operand")
        }
    };
    assert_session_empty_partial(value, saved);
    assert_eq!(program.bodies[saved.function.body].arity, 2);
    observed[2] = true;
}

fn assert_session_empty_fix_result(
    machine: &Machine,
    saved: &SessionEmptyGuardSaved,
    verdict: SessionGuardVerdict,
    result: Result<Value, fipe::vm::Fault>,
    requests: &[String],
    observed: [bool; 3],
) {
    use fipe::value::Reason as ValueReason;
    assert!(
        observed.into_iter().all(|seen| seen),
        "empty fix actual call then Integer Test/written cmp: {observed:?}"
    );
    match verdict {
        SessionGuardVerdict::Healthy => {
            assert_eq!(result, Ok(Value::Int(7)));
            assert_eq!(requests, ["load", "items", "after"]);
        }
        SessionGuardVerdict::HostFault => {
            let fault = result.expect_err("wrong after reply must refuse");
            assert!(
                matches!(&fault.reason, ValueReason::Host(message) if message.contains("Integer")),
                "{fault:?}"
            );
            assert_eq!(requests, ["load", "items", "after"]);
        }
        SessionGuardVerdict::SortedFault => {
            let fault = result.expect_err(
                "actual retained Guard refuses unsorted after call",
            );
            let ValueReason::Failed { entry, .. } = &fault.reason else {
                panic!("{fault:?}")
            };
            assert!(session_guard_test_arguments(
                &machine.program().unwrap().tables.borrow().entries[*entry]
                    .predicate,
                saved.test
            )
            .is_some());
            assert_eq!(requests, ["load", "items"]);
        }
    }
    assert_session_empty_template(machine.program().unwrap(), saved, false);
}

fn exercise_session_empty_fix(
    session: &mut Session,
    machine: &mut Machine,
    saved: &SessionEmptyGuardSaved,
    verdict: SessionGuardVerdict,
) -> Report {
    use fipe::vm::Stop;
    let report = session_guard_line(
        session,
        SESSION_EMPTY_FIX_CALL,
        "empty stage4 fixing/ordinary loaded admission",
    );
    let program = compile::compile(report.unit.as_ref().unwrap());
    machine.begin(&program);
    let joined = machine.program().unwrap();
    assert_eq!(session_guard_source_test(joined), saved.test);
    assert_no_session_sorted_argument_entry(joined, saved.test);
    assert_session_empty_template(joined, saved, false);
    let mut observed = [false; 3];
    let mut requests = Vec::new();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        observe_session_empty_fix(machine, saved, report.offset, &mut observed);
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(request) => {
                let reply = session_empty_fix_reply(
                    machine,
                    &request,
                    saved,
                    verdict,
                    &mut requests,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Done(result) => {
                assert_session_empty_fix_result(
                    machine, saved, verdict, result, &requests, observed,
                );
                return report;
            }
        }
    }
    panic!("empty stage4 bounded loaded invocation")
}

fn withdraw_session_empty_fix(
    session: &mut Session,
    machine: &mut Machine,
    saved: &SessionEmptyGuardSaved,
    report: &Report,
) {
    let removed = session
        .withdraw(report.accepted.unwrap())
        .expect("latest faulted fixing line only");
    machine.withdraw(&removed);
    for name in ["fixed", "loaded"] {
        assert!(
            !unknown_name(&done(session.query(name))).is_empty(),
            "withdrawn {name} must be unknown"
        );
    }
    assert!(
        done(session.query("boxes")).diagnostics.is_empty(),
        "weak source remains accepted"
    );
    assert_eq!(
        session.lines().count(),
        3,
        "setup/weak/actual producer histories retained"
    );
    assert_session_empty_template(machine.program().unwrap(), saved, true);
    assert_session_empty_partial(
        &Value::Function(std::rc::Rc::clone(&saved.function)),
        saved,
    );
}

#[test]
fn predicate_guard_empty_associations_follow_live_session_fix_and_withdrawal() {
    let mut declarations = Declarations::new();
    declarations
        .module("host", SESSION_EMPTY_HOST)
        .expect("empty stage0 rank-only Box/array2 to Vec declarations");
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(Some(1 << 20));
    let saved = create_session_empty_guard(&mut session, &mut machine);
    for verdict in [
        SessionGuardVerdict::HostFault,
        SessionGuardVerdict::SortedFault,
    ] {
        let report = exercise_session_empty_fix(
            &mut session,
            &mut machine,
            &saved,
            verdict,
        );
        withdraw_session_empty_fix(&mut session, &mut machine, &saved, &report);
    }
    let _healthy = exercise_session_empty_fix(
        &mut session,
        &mut machine,
        &saved,
        SessionGuardVerdict::Healthy,
    );
    assert_session_empty_template(machine.program().unwrap(), &saved, false);
}

const OWNED_SESSION_MEANING_SETUP: &str = "use host\nstep = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }\nis_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }\ntype Sorted<T: Ord>: Vec<T> where is_sorted self\nkeep: Ord a => Integer -> Sorted<a> -> Integer = n xs => if is_sorted xs { host.after () } else { 0 }\nmake_guard: Ord a => [a; 2] -> (Integer -> Sorted<a> -> Integer) = sample => host.relay keep<a>\n";

struct OwnedSessionCopySaved {
    original: std::rc::Rc<fipe::value::Closure>,
    canonical: std::rc::Rc<fipe::value::Closure>,
    slot: usize,
    test: usize,
    definition: usize,
    meaning_position: usize,
}

fn owned_session_root(
    joined: &fipe::mir::Program,
    tables: &fipe::ir::Tables,
    function: &fipe::value::Closure,
) -> (usize, usize) {
    use fipe::ir;
    let body = &joined.bodies[function.body];
    let node = &tables.nodes[body.node];
    assert_eq!(tables.namespaces[node.namespace], "line 1");
    assert_eq!(
        OWNED_SESSION_MEANING_SETUP.get(node.span.start..node.span.end),
        Some("n xs => if is_sorted xs { host.after () } else { 0 }"),
        "actual body that consumes T, not generated wrapper"
    );
    assert_eq!(body.arity, 2);
    assert!(body.binding.is_none());
    assert!(joined.receiver_contexts[function.body].is_none());
    assert!(function.filled.is_empty());
    println!(
        "OWNED_SESSION_KEEP body={} hidden={:?} meanings={:?} guards={:?}",
        function.body, body.hidden, function.meanings, function.guards
    );
    assert!(
        !body.hidden.is_empty(),
        "T-dependent body must actually demand its binder"
    );
    assert_eq!(body.hidden.len(), function.meanings.len());
    let roots: Vec<_> = function.meanings.iter().enumerate().filter(|(_, ty)| matches!(ty,
        ir::Type::Meaning { index, .. } if tables.namespaces[tables.meanings[*index].namespace] == "host"
            && tables.meanings[*index].label == "Box")).collect();
    assert_eq!(
        roots.len(),
        1,
        "one actual source T root in Closure.meanings"
    );
    let (position, owned) = roots[0];
    let slot = assert_session_guard_box(owned, tables, None);
    (position, slot)
}

fn owned_session_test(
    joined: &fipe::mir::Program,
    tables: &fipe::ir::Tables,
) -> usize {
    let tests: Vec<_> = tables
        .tests
        .iter()
        .enumerate()
        .filter_map(|(index, test)| {
            (tables.namespaces[test.namespace] == "line 1"
                && OWNED_SESSION_MEANING_SETUP
                    .get(test.span.start..test.span.end)
                    == Some("is_sorted self"))
            .then_some(index)
        })
        .collect();
    assert_eq!(tests.len(), 1, "source-owned semantic Sorted Test");
    let test = tests[0];
    assert!(joined.tests[test].is_some(), "actual compiled Test body");
    test
}

fn assert_owned_session_creator(
    machine: &Machine,
    tables: &fipe::ir::Tables,
    function: &fipe::value::Closure,
    guard: &fipe::value::Guard,
    test: usize,
    position: usize,
    slot: usize,
) {
    use fipe::{ir, vm::Frame};
    let joined = machine.program().unwrap();
    let owned = &function.meanings[position];
    let keep_parameter = joined.bodies[function.body].hidden[position];
    let creator_definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "make_guard"
                && tables.namespaces[definition.namespace] == "line 1"
        })
        .unwrap();
    let creator =
        joined.definitions[creator_definition].expect("actual make_guard body");
    let frame = machine
        .frames()
        .iter()
        .find_map(|frame| match frame {
            Frame::Body(frame) if frame.body == creator => Some(frame),
            _ => None,
        })
        .expect("actual creating frame owns the relay request");
    assert_eq!(joined.bodies[creator].hidden.len(), frame.meanings.len());
    let templates =
        session_guard_test_arguments(&guard.predicate, test).unwrap();
    assert_eq!(templates.len(), 1);
    let ir::Type::Parameter(creator_parameter) = &templates[0] else {
        panic!("creator-owned Test template parameter")
    };
    let creator_position = joined.bodies[creator]
        .hidden
        .iter()
        .position(|id| id == creator_parameter)
        .expect("Test binder belongs to creator");
    let (_, captured) = guard
        .witnesses
        .iter()
        .find(|(id, _)| id == creator_parameter)
        .expect("real captured association");
    assert_eq!(captured.as_type(), owned);
    assert_eq!(&frame.meanings[creator_position], owned);
    assert_eq!(
        assert_session_guard_box(captured.as_type(), tables, None),
        slot
    );
    println!("OWNED_SESSION_ASSOCIATION keep_parameter={keep_parameter} creator_parameter={creator_parameter} slot={slot} owned={owned:?} captured={captured:?}");
}

fn assert_owned_session_alias(
    retained: &std::rc::Rc<fipe::value::Closure>,
    original: &std::rc::Rc<fipe::value::Closure>,
    original_meanings: &[fipe::ir::Type],
    witnesses: &std::rc::Rc<[(usize, std::rc::Rc<fipe::value::Meaning>)]>,
) {
    assert_eq!(
        original.guards.len(),
        1,
        "original Host argument remains unchanged"
    );
    assert_eq!(original.meanings, original_meanings);
    assert!(original.filled.is_empty());
    assert!(
        !std::rc::Rc::ptr_eq(retained, original),
        "actual store Function entry owns its private copy"
    );
    assert_eq!(retained.body, original.body);
    assert_eq!(retained.captures, original.captures);
    assert_eq!(retained.filled, original.filled);
    assert_eq!(retained.meanings, original_meanings);
    assert_eq!(
        retained.guards.len(),
        2,
        "one original guard plus actual store contract"
    );
    assert_eq!(retained.guards[0].entry, original.guards[0].entry);
    assert_eq!(retained.guards[0].spot, original.guards[0].spot);
    assert_eq!(retained.guards[0].predicate, original.guards[0].predicate);
    assert!(std::rc::Rc::ptr_eq(
        &retained.guards[0].witnesses,
        witnesses
    ));
}

fn assert_owned_session_store(
    tables: &fipe::ir::Tables,
    retained: &fipe::value::Closure,
    definition: usize,
    offset: usize,
    expected_raw_meaning: &fipe::ir::Type,
    slot: usize,
    test: usize,
) {
    use fipe::ir;
    let call = "make_guard boxes";
    let start = offset + SESSION_GUARD_STORE.find(call).unwrap();
    let store_entries: Vec<_> = tables
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            (entry.namespace == tables.definitions[definition].namespace
                && entry.span.start == start
                && entry.span.end == start + call.len()
                && entry.path.is_empty()
                && matches!(&entry.predicate, ir::Predicate::Function { .. }))
            .then_some(index)
        })
        .collect();
    assert_eq!(
        store_entries.len(),
        1,
        "actual source-linked store Function entry"
    );
    let store_guard = &retained.guards[1];
    assert_eq!(store_guard.entry, store_entries[0]);
    assert_eq!(store_guard.spot, 1);
    assert!(
        store_guard.witnesses.is_empty(),
        "direct Session template needs no captured Parameter association"
    );
    let store_arguments =
        session_guard_test_arguments(&store_guard.predicate, test)
            .expect("same source semantic Sorted Test");
    assert_eq!(store_arguments.len(), 1);
    assert_eq!(&store_arguments[0], expected_raw_meaning);
    assert_eq!(
        assert_session_guard_box(&store_arguments[0], tables, None),
        slot
    );
}

fn owned_session_store_definition(
    tables: &fipe::ir::Tables,
    offset: usize,
) -> usize {
    tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "f"
                && definition.span.start
                    == offset + SESSION_GUARD_STORE.find('f').unwrap()
        })
        .expect("actual stored global f")
}

fn owned_session_creation_guard(
    function: &fipe::value::Closure,
    test: usize,
) -> &fipe::value::Guard {
    let guards: Vec<_> = function
        .guards
        .iter()
        .filter(|guard| {
            session_guard_test_arguments(&guard.predicate, test).is_some()
        })
        .collect();
    assert_eq!(guards.len(), 1);
    let guard = guards[0];
    assert_eq!(guard.spot, 1);
    assert!(!guard.witnesses.is_empty());
    guard
}

fn assert_owned_session_relay(tables: &fipe::ir::Tables, declaration: usize) {
    let declared = &tables.declarations[declaration];
    assert_eq!(tables.namespaces[declared.namespace], "host");
    assert_eq!(declared.label, "relay");
}

fn create_owned_session_meaning(
    session: &mut Session,
    machine: &mut Machine,
) -> OwnedSessionCopySaved {
    let setup = session_guard_line(
        session,
        OWNED_SESSION_MEANING_SETUP,
        "owned meaning setup admission",
    );
    assert!(matches!(
        machine.run(&compile::compile(setup.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
    let weak = session_guard_line(
        session,
        SESSION_GUARD_WEAK,
        "owned meaning weak Box admission",
    );
    assert!(!weak.typed.as_ref().unwrap().session_variables.is_empty());
    assert!(matches!(
        machine.run(&compile::compile(weak.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
    let stored = session_guard_line(
        session,
        SESSION_GUARD_STORE,
        "owned meaning guard creation admission",
    );
    let program = compile::compile(stored.unit.as_ref().unwrap());
    let outcome = machine.run(&program);
    println!("OWNED_SESSION_CREATION outcome={outcome:?}");
    let Outcome::Request(Request::Call {
        declaration,
        arguments,
        environment,
    }) = outcome
    else {
        panic!("actual creator must publish its Function to relay")
    };
    assert!(environment.is_empty());
    let [Value::Function(function)] = arguments.as_slice() else {
        panic!("actual saved Function argument")
    };
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    assert_owned_session_relay(&tables, declaration);
    let (position, slot) = owned_session_root(joined, &tables, function);
    let test = owned_session_test(joined, &tables);
    let guard = owned_session_creation_guard(function, test);
    assert_owned_session_creator(
        machine, &tables, function, guard, test, position, slot,
    );
    // Declaration contexts are correlated through actual meanings, without
    // assuming their numeric IDs must be equal or different.
    let original = function.clone();
    let witnesses = guard.witnesses.clone();
    let original_meanings = function.meanings.clone();
    drop(tables);
    let done = machine.reply(&program, Reply::Value(Ok(arguments[0].clone())));
    assert!(
        matches!(done, Outcome::Done(Ok(_))),
        "exact Function reply retains producer: {done:?}"
    );
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let definition = owned_session_store_definition(&tables, stored.offset);
    let (_, Value::Function(retained)) = machine
        .globals()
        .find(|(index, _)| *index == definition)
        .expect("retained source global")
    else {
        panic!("stored value remains Function")
    };
    assert_owned_session_alias(
        retained,
        &original,
        &original_meanings,
        &witnesses,
    );
    assert_owned_session_store(
        &tables,
        retained,
        definition,
        stored.offset,
        &original_meanings[position],
        slot,
        test,
    );
    let guard = retained
        .guards
        .iter()
        .find(|guard| {
            session_guard_test_arguments(&guard.predicate, test).is_some()
        })
        .unwrap();
    assert!(std::rc::Rc::ptr_eq(&guard.witnesses, &witnesses));
    assert!(tables.session_meanings[slot].is_none());
    OwnedSessionCopySaved {
        original,
        canonical: retained.clone(),
        slot,
        test,
        definition,
        meaning_position: position,
    }
}

#[test]
fn session_guard_body_owns_the_actual_unfixed_box_meaning() {
    let mut declarations = Declarations::new();
    declarations.module("host", SESSION_GUARD_HOST).unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(None);
    let _saved = create_owned_session_meaning(&mut session, &mut machine);
}

const OWNED_SESSION_PARTIAL_DRIVER: &str = "apply_one = fn x y => {\n partial = fn x\n next = host.relay partial\n next y\n}\nrun: Ord a => [a; 2] -> Integer = sample => {\n loaded: Integer -> Vec<a> -> Integer = host.load ()\n apply_one loaded 7 (host.items (sample | to<Vec<a>>))\n}\n";

fn owned_partial_raw_aliases(
    machine: &Machine,
    saved: &OwnedSessionCopySaved,
    fixed: bool,
) {
    use fipe::ir::Type;
    let joined = machine.program().unwrap();
    let tables = joined.tables.borrow();
    let raw = &saved.original.meanings[saved.meaning_position];
    let Type::Meaning { arguments, .. } = raw else {
        panic!("actual owned Box root")
    };
    assert!(
        matches!(arguments.as_slice(), [Type::Session(slot)] if *slot == saved.slot)
    );
    assert_eq!(saved.canonical.meanings[saved.meaning_position], *raw);
    assert_eq!(saved.original.guards.len(), 1);
    assert_eq!(saved.canonical.guards.len(), 2);
    assert!(
        saved.original.filled.is_empty() && saved.canonical.filled.is_empty()
    );
    assert_eq!(saved.original.body, saved.canonical.body);
    let (_, Value::Function(global)) = machine
        .globals()
        .find(|(index, _)| *index == saved.definition)
        .unwrap()
    else {
        panic!("canonical global f")
    };
    assert!(std::rc::Rc::ptr_eq(global, &saved.canonical));
    if fixed {
        assert_session_guard_fixed_slot(&tables, saved.slot);
    } else {
        assert!(tables.session_meanings[saved.slot].is_none());
    }
    let captured = &saved.canonical.guards[0];
    assert_eq!(captured.entry, saved.original.guards[0].entry);
    assert_eq!(captured.predicate, saved.original.guards[0].predicate);
    assert!(std::rc::Rc::ptr_eq(
        &captured.witnesses,
        &saved.original.guards[0].witnesses
    ));
    assert!(!captured.witnesses.is_empty());
    for (_, witness) in captured.witnesses.iter() {
        assert_eq!(witness.as_type(), raw);
    }
    let direct = &saved.canonical.guards[1];
    assert_ne!(direct.entry, captured.entry);
    assert!(direct.witnesses.is_empty());
    assert_eq!(
        session_guard_test_arguments(&direct.predicate, saved.test).unwrap(),
        std::slice::from_ref(raw)
    );
}

fn owned_partial_value(
    machine: &Machine,
    saved: &OwnedSessionCopySaved,
    partial: &std::rc::Rc<fipe::value::Closure>,
    fixed: bool,
) {
    assert_eq!(partial.body, saved.canonical.body);
    assert_eq!(machine.program().unwrap().bodies[partial.body].arity, 2);
    assert_eq!(partial.filled, [Value::Int(7)]);
    assert_eq!(partial.captures, saved.canonical.captures);
    assert!(!std::rc::Rc::ptr_eq(partial, &saved.canonical));
    assert!(!std::rc::Rc::ptr_eq(partial, &saved.original));
    let tables = machine.program().unwrap().tables.borrow();
    assert_eq!(partial.meanings.len(), saved.canonical.meanings.len());
    assert_session_guard_box(
        &partial.meanings[saved.meaning_position],
        &tables,
        fixed.then_some(saved.slot),
    );
    assert_eq!(
        partial.guards.len(),
        2,
        "both ordered guard records survive the Integer prefix"
    );
    for (guard, original) in partial.guards.iter().zip(&saved.canonical.guards)
    {
        assert_eq!(guard.entry, original.entry);
        assert_eq!(guard.spot, 1);
        assert_eq!(
            guard.predicate, original.predicate,
            "raw Predicate remains unchanged even when owned meanings resolve"
        );
        assert!(std::rc::Rc::ptr_eq(&guard.witnesses, &original.witnesses));
        assert_eq!(guard.witnesses.len(), original.witnesses.len());
        for ((id, witness), (old_id, old_witness)) in
            guard.witnesses.iter().zip(original.witnesses.iter())
        {
            assert_eq!(id, old_id);
            assert!(std::rc::Rc::ptr_eq(witness, old_witness));
        }
    }
    drop(tables);
    owned_partial_raw_aliases(machine, saved, fixed);
}

fn owned_partial_entry_position(body: &fipe::mir::Body) -> usize {
    use fipe::mir::{Statement, Terminator};
    for block in &body.blocks {
        let edges: Vec<_> = match &block.terminator {
            Terminator::Goto(target) | Terminator::Call { target, .. } => {
                vec![*target]
            }
            Terminator::Switch {
                cases, otherwise, ..
            } => cases
                .iter()
                .map(|(_, target)| *target)
                .chain(otherwise.iter().copied())
                .collect(),
            Terminator::Test { pass, fail, .. } => vec![*pass, *fail],
            Terminator::Return | Terminator::Unreachable => Vec::new(),
        };
        assert!(
            !edges.contains(&0),
            "source entry must not have a backedge when counting activations"
        );
    }
    body.blocks[0]
        .statements
        .iter()
        .position(|statement| !matches!(statement, Statement::Charge(_)))
        .unwrap_or(body.blocks[0].statements.len())
}

fn owned_partial_metadata(
    program: &fipe::mir::Program,
    offset: usize,
    test: usize,
) -> (usize, usize, usize, usize) {
    use fipe::{ir, mir};
    let tables = program.tables.borrow();
    let definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "apply_one"
                && definition.span.start
                    == offset
                        + OWNED_SESSION_PARTIAL_DRIVER
                            .find("apply_one")
                            .unwrap()
        })
        .expect("source apply_one binding");
    let helper = program.definitions[definition].unwrap();
    let run_definition = tables
        .definitions
        .iter()
        .position(|definition| {
            definition.label == "run"
                && definition.span.start
                    == offset
                        + OWNED_SESSION_PARTIAL_DRIVER.find("run:").unwrap()
        })
        .expect("source run binding");
    let run = program.definitions[run_definition].unwrap();
    assert_eq!(program.bodies[helper].arity, 3);
    assert!(
        program.bodies[helper]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .all(|statement| !matches!(
                statement,
                mir::Statement::Assign(_, mir::Rvalue::Closure { .. })
            )),
        "dynamic helper must not manufacture a wrapper"
    );
    for body in [program.main, run, helper] {
        for statement in program.bodies[body]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
        {
            if let mir::Statement::Enter { entry, .. } = statement {
                assert!(
                    session_guard_test_arguments(
                        &tables.entries[*entry].predicate,
                        test
                    )
                    .is_none(),
                    "caller-side Sorted entry must not mask retained guards"
                );
            }
        }
    }
    let start = offset + OWNED_SESSION_PARTIAL_DRIVER.find("fn x\n").unwrap();
    let mut found = Vec::new();
    for block in &program.bodies[helper].blocks {
        let mir::Terminator::Call {
            callee: mir::Callee::Value(_),
            args,
            dest,
            target,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let Some(node) = block.nodes.last() else {
            continue;
        };
        let site = &tables.nodes[*node];
        if site.span.start != start || site.span.end != start + "fn x".len() {
            continue;
        }
        let ir::Kind::Call { open, .. } = &site.kind else {
            panic!("source fn x IR")
        };
        assert!(open.is_empty());
        assert_eq!(args.len(), 1);
        assert!(matches!(
            program.bodies[helper].locals[*dest].ty,
            ir::Type::Function { .. }
        ));
        found.push((*dest, *target, *node));
    }
    assert_eq!(found.len(), 1);
    (helper, found[0].0, found[0].1, found[0].2)
}

#[derive(Default, Debug)]
struct OwnedPartialObservation {
    before: Option<u64>,
    ready: Option<u64>,
    relay: bool,
    test_entries: usize,
    body_entered: bool,
    calls: Vec<String>,
    partial: Option<std::rc::Rc<fipe::value::Closure>>,
    result: Option<Result<Value, fipe::vm::Fault>>,
}

#[derive(Clone, Copy)]
struct OwnedPartialSite {
    helper: usize,
    dest: usize,
    target: usize,
    call_node: usize,
    test_body: usize,
    test_position: usize,
    body_position: usize,
}

fn observe_owned_partial_before(
    machine: &Machine,
    saved: &OwnedSessionCopySaved,
    site: &OwnedPartialSite,
    fixed: bool,
    healthy: bool,
    stop_ready: bool,
    observed: &mut OwnedPartialObservation,
) -> bool {
    use fipe::{mir, vm::Frame};
    if let Some(Frame::Body(frame)) = machine.frames().last() {
        let joined = machine.program().unwrap();
        if frame.body == site.helper {
            let block = &joined.bodies[site.helper].blocks[frame.block];
            if frame.statement == block.statements.len()
                && block.nodes.last() == Some(&site.call_node)
            {
                let mir::Terminator::Call {
                    callee: mir::Callee::Value(mir::Operand::Local(local)),
                    args,
                    ..
                } = &block.terminator
                else {
                    panic!("actual closed fn x Local call")
                };
                let Value::Function(function) = &frame.locals[*local] else {
                    panic!("actual Host-loaded canonical Function")
                };
                assert!(std::rc::Rc::ptr_eq(function, &saved.canonical));
                assert!(function.filled.is_empty());
                assert_eq!(joined.bodies[function.body].arity, 2);
                assert_eq!(args.len(), 1);
                owned_partial_raw_aliases(machine, saved, fixed);
                assert!(observed.before.replace(machine.charged()).is_none());
            }
            if frame.block == site.target
                && observed.before.is_some()
                && observed.ready.is_none()
            {
                let Value::Function(partial) = &frame.locals[site.dest] else {
                    panic!("actual completed partial destination")
                };
                owned_partial_value(machine, saved, partial, fixed);
                assert!(!observed.relay && observed.test_entries == 0);
                observed.partial = Some(partial.clone());
                observed.ready = Some(machine.charged());
                if stop_ready {
                    return true;
                }
            }
        }
        if frame.body == site.test_body
            && frame.block == 0
            && frame.statement == site.test_position
        {
            assert!(
                observed.relay && !observed.body_entered,
                "retained guards run after relay and before keep body"
            );
            let tables = joined.tables.borrow();
            assert_eq!(
                joined.bodies[site.test_body].hidden.len(),
                frame.meanings.len()
            );
            assert!(!frame.meanings.is_empty());
            for meaning in &frame.meanings {
                assert_session_guard_box(
                    meaning,
                    &tables,
                    fixed.then_some(saved.slot),
                );
            }
            observed.test_entries += 1;
            assert!(
                observed.test_entries <= 2,
                "exactly E0/E1, not duplicate replay or caller gates"
            );
        }
        if frame.body == saved.canonical.body
            && frame.block == 0
            && frame.statement == site.body_position
        {
            assert_eq!(
                observed.test_entries, 2,
                "both owned guards precede actual body entry"
            );
            assert!(healthy);
            observed.body_entered = true;
        }
    }
    false
}

fn owned_partial_reply(
    machine: &Machine,
    saved: &OwnedSessionCopySaved,
    fixed: bool,
    healthy: bool,
    observed: &mut OwnedPartialObservation,
    label: &str,
    arguments: &[Value],
) -> Value {
    match label {
        "load" => {
            assert_eq!(observed.calls, ["load"]);
            assert_eq!(arguments, [Value::Unit]);
            Value::Function(saved.canonical.clone())
        }
        "items" => {
            assert_eq!(observed.calls, ["load", "items"]);
            let mut items = assert_session_empty_items(machine, arguments);
            if !healthy {
                items.reverse();
            }
            Value::seq(items)
        }
        "relay" => {
            assert_eq!(observed.calls, ["load", "items", "relay"]);
            let [Value::Function(partial)] = arguments else {
                panic!("actual private partial publication")
            };
            owned_partial_value(machine, saved, partial, fixed);
            assert!(std::rc::Rc::ptr_eq(
                partial,
                observed
                    .partial
                    .as_ref()
                    .expect("completed private partial observed")
            ));
            assert_eq!(observed.test_entries, 0);
            observed.relay = true;
            arguments[0].clone()
        }
        "after" => {
            assert!(healthy && observed.body_entered);
            assert_eq!(observed.test_entries, 2);
            assert_eq!(arguments, [Value::Unit]);
            Value::Int(7)
        }
        _ => panic!("unexpected source effect {label}"),
    }
}

fn run_owned_session_partial(
    machine: &mut Machine,
    program: &fipe::mir::Program,
    saved: &OwnedSessionCopySaved,
    driver_offset: usize,
    fixed: bool,
    healthy: bool,
    stop_ready: bool,
) -> OwnedPartialObservation {
    use fipe::vm::Stop;
    let budget = machine.budget();
    machine.begin(program);
    let (helper, dest, target, call_node) = owned_partial_metadata(
        machine.program().unwrap(),
        driver_offset,
        saved.test,
    );
    let (test_body, test_position, body_position) = {
        let joined = machine.program().unwrap();
        let test_body =
            joined.tests[saved.test].expect("actual compiled source Test");
        (
            test_body,
            owned_partial_entry_position(&joined.bodies[test_body]),
            owned_partial_entry_position(&joined.bodies[saved.canonical.body]),
        )
    };
    let site = OwnedPartialSite {
        helper,
        dest,
        target,
        call_node,
        test_body,
        test_position,
        body_position,
    };
    let mut observed = OwnedPartialObservation::default();
    for _ in 0..100_000 {
        let stop = machine.next_stop();
        if matches!(&stop, Stop::Before(_))
            && observe_owned_partial_before(
                machine,
                saved,
                &site,
                fixed,
                healthy,
                stop_ready,
                &mut observed,
            )
        {
            return observed;
        }
        match stop {
            Stop::Before(_) | Stop::After(..) => {}
            Stop::Request(Request::Call {
                declaration,
                arguments,
                environment,
            }) => {
                assert!(environment.is_empty());
                let label =
                    machine.program().unwrap().tables.borrow().declarations
                        [declaration]
                        .label
                        .clone();
                observed.calls.push(label.clone());
                let reply = owned_partial_reply(
                    machine,
                    saved,
                    fixed,
                    healthy,
                    &mut observed,
                    &label,
                    &arguments,
                );
                machine.answer(Reply::Value(Ok(reply)));
            }
            Stop::Request(other) => {
                panic!("ordinary typed callback only: {other:?}")
            }
            Stop::Done(result) => {
                println!("OWNED_PARTIAL fixed={fixed} healthy={healthy} budget={budget:?} charged={} observed={observed:?} result={result:?}", machine.charged());
                observed.result = Some(result);
                owned_partial_raw_aliases(machine, saved, fixed);
                return observed;
            }
        }
    }
    panic!("bounded owned Session partial must finish");
}

fn assert_owned_partial_measured(measured: &OwnedPartialObservation) {
    assert!(matches!(measured.result, Some(Ok(Value::Int(7)))));
    assert_eq!(measured.test_entries, 2);
    assert!(measured.relay && measured.body_entered);
    assert_eq!(measured.calls, ["load", "items", "relay", "after"]);
}

fn assert_owned_partial_reversed(
    reversed: &OwnedPartialObservation,
    saved: &OwnedSessionCopySaved,
) {
    let Some(Err(fipe::vm::Fault {
        reason: fipe::value::Reason::Failed { entry, .. },
        ..
    })) = &reversed.result
    else {
        panic!("reversed input must fail its first retained guard")
    };
    assert_eq!(*entry, saved.canonical.guards[0].entry);
    assert_eq!(reversed.test_entries, 1);
    assert!(!reversed.body_entered);
    assert_eq!(reversed.calls, ["load", "items", "relay"]);
}

fn assert_owned_partial_refused(
    machine: &Machine,
    refused: &OwnedPartialObservation,
    short: u64,
) {
    assert!(matches!(
        refused.result,
        Some(Err(fipe::vm::Fault {
            reason: fipe::value::Reason::Budget,
            ..
        }))
    ));
    assert_eq!(machine.charged(), short);
    assert!(
        refused.ready.is_none()
            && !refused.relay
            && refused.test_entries == 0
            && !refused.body_entered
    );
    assert_eq!(refused.calls, ["load", "items"]);
}

fn assert_owned_partial_exact(
    machine: &Machine,
    exact: &OwnedPartialObservation,
    ready: u64,
) {
    assert_eq!(exact.ready, Some(ready));
    assert_eq!(machine.charged(), ready);
    assert!(
        exact.result.is_none()
            && !exact.relay
            && exact.test_entries == 0
            && !exact.body_entered
    );
    assert_eq!(exact.calls, ["load", "items"]);
}

fn fix_owned_session_partial(session: &mut Session, machine: &mut Machine) {
    let fix = session_guard_line(
        session,
        "fixed: [host.Box<Integer>; 2] = boxes\n",
        "owned Session fixed array admission",
    );
    assert!(matches!(
        machine.run(&compile::compile(fix.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
}

fn owned_session_partial_case(fixed: bool) {
    let mut declarations = Declarations::new();
    declarations.module("host", SESSION_GUARD_HOST).unwrap();
    let mut session = Session::new(declarations);
    let mut machine = Machine::new(None);
    let saved = create_owned_session_meaning(&mut session, &mut machine);
    let driver = session_guard_line(
        &mut session,
        OWNED_SESSION_PARTIAL_DRIVER,
        "owned Session partial driver admission",
    );
    assert!(matches!(
        machine.run(&compile::compile(driver.unit.as_ref().unwrap())),
        Outcome::Done(Ok(_))
    ));
    if fixed {
        fix_owned_session_partial(&mut session, &mut machine);
    }
    let call = session_guard_line(
        &mut session,
        if fixed { "run fixed\n" } else { "run boxes\n" },
        "owned Session partial call admission",
    );
    let program = compile::compile(call.unit.as_ref().unwrap());
    machine.set_budget(None);
    let warm = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        true,
        false,
    );
    assert!(matches!(warm.result, Some(Ok(Value::Int(7)))));
    machine.set_budget(None);
    let measured = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        true,
        false,
    );
    assert_owned_partial_measured(&measured);
    machine.set_budget(None);
    let reversed = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        false,
        false,
    );
    assert_owned_partial_reversed(&reversed, &saved);
    let before = measured.before.expect("actual before dynamic partial");
    let ready = measured.ready.expect("actual completed partial checkpoint");
    assert!(ready > before);
    let short = ready.checked_sub(1).unwrap();
    machine.set_budget(Some(short));
    let refused = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        true,
        false,
    );
    assert_owned_partial_refused(&machine, &refused, short);
    machine.set_budget(Some(ready));
    let exact = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        true,
        true,
    );
    assert_owned_partial_exact(&machine, &exact, ready);
    machine.set_budget(None);
    let retry = run_owned_session_partial(
        &mut machine,
        &program,
        &saved,
        driver.offset,
        fixed,
        true,
        false,
    );
    assert!(matches!(retry.result, Some(Ok(Value::Int(7)))));
    assert_eq!(retry.test_entries, 2);
    assert_eq!(retry.calls, ["load", "items", "relay", "after"]);
    println!("OWNED_PARTIAL_BOUNDARY fixed={fixed} before={before} ready={ready} transaction={}", ready - before);
}

#[test]
fn owned_guarded_partial_keeps_unfixed_session_meanings_and_budget_boundaries()
{
    owned_session_partial_case(false);
}
#[test]
fn owned_guarded_partial_resolves_fixed_session_meanings_and_budget_boundaries()
{
    owned_session_partial_case(true);
}
