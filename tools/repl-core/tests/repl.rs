use fipe_repl_core::{Out, Repl};

const GEOMETRY: &str =
    include_str!("../../../Documentation/examples/geometry.fipe");
const HOST: [(&str, &str); 5] = [
    (
        "tests/fixtures/host/commands.fipe",
        include_str!("../../../tests/fixtures/host/commands.fipe"),
    ),
    (
        "tests/fixtures/host/fs.fipe",
        include_str!("../../../tests/fixtures/host/fs.fipe"),
    ),
    (
        "tests/fixtures/host/sys.fipe",
        include_str!("../../../tests/fixtures/host/sys.fipe"),
    ),
    (
        "tests/fixtures/host/cli.fipe",
        include_str!("../../../tests/fixtures/host/cli.fipe"),
    ),
    (
        "tests/fixtures/host/text.fipe",
        include_str!("../../../tests/fixtures/host/text.fipe"),
    ),
];

/// The written bytes, with any other output a test failure.
fn written(outs: Vec<Out>) -> String {
    let mut bytes = Vec::new();
    for out in outs {
        let Out::Write(chunk) = out else {
            panic!("unexpected {out:?}")
        };
        bytes.extend(chunk);
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn the_first_feed_prompts() {
    assert_eq!(Repl::new().feed(&[]), [Out::Write(b"fipe> ".to_vec())]);
}

#[test]
fn a_local_resolves_to_its_site() {
    let text = written(Repl::new().feed(b"x = 1\n:r x\n"));
    assert_eq!(
        text,
        "1:1 0..1 x: Local { site: Span { start: 0, end: 1 } }\nfipe> "
    );
}

#[test]
fn a_diagnostic_underlines_its_span() {
    let text = written(Repl::new().feed(b"nope 1\n"));
    assert_eq!(text, "1:1: UnknownName\nnope 1\n^^^^\nfipe> ");
}

#[test]
fn a_partial_line_waits_for_its_end_or_a_flush() {
    let mut repl = Repl::new();
    assert_eq!(repl.feed(b"nope"), []);
    assert_eq!(written(repl.flush()), "1:1: UnknownName\nnope\n^^^^\n");
    assert_eq!(repl.flush(), []);
}

#[test]
fn use_asks_for_the_file_and_resumes() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use geometry.Point\nPoint 1\n:r Point\n");
    assert_eq!(outs, [Out::File("geometry.fipe".into())]);
    let text = written(repl.file("geometry.fipe", Some(GEOMETRY)));
    assert!(
        text.contains(
            "1:1 0..5 Point: Constructor { origin: File(\"geometry\")"
        ),
        "{text}"
    );
}

#[test]
fn load_asks_for_the_stem() {
    let mut repl = Repl::new();
    assert_eq!(
        repl.feed(b":load geometry\n"),
        [Out::File("geometry.fipe".into())]
    );
    assert_eq!(
        written(repl.file("geometry.fipe", Some(GEOMETRY))),
        "fipe> "
    );
    let text = written(repl.feed(b":r geometry.Point 1\n"));
    assert!(text.contains("Constructor"), "{text}");
}

#[test]
fn an_absent_file_is_an_unknown_module() {
    let mut repl = Repl::new();
    let outs = repl.feed(b"use missing\n");
    assert_eq!(outs, [Out::File("missing.fipe".into())]);
    let text = written(repl.file("missing.fipe", None));
    assert_eq!(text, "1:5: UnknownModule\nuse missing\n    ^^^^^^^\nfipe> ");
}

#[test]
fn host_loads_the_fixtures_and_keeps_accepted_lines() {
    let mut repl = Repl::new();
    let mut outs = repl.feed(b"x = 1\n:host\n");
    for (index, (path, text)) in HOST.iter().enumerate() {
        assert_eq!(outs, [Out::File((*path).into())], "{index}");
        outs = repl.file(path, Some(text));
    }
    assert_eq!(written(outs), "fipe> ");
    let text = written(repl.feed(b"ls -l /etc\n:r ls\n:r x\n"));
    assert!(text.contains("ls: Value { origin: Commands"), "{text}");
    assert!(text.contains("x: Local"), "{text}");
}

#[test]
fn ast_shows_the_last_accepted_line() {
    let text = written(Repl::new().feed(b"x = 1\n:ast\n"));
    assert!(text.starts_with("Expr {\n    kind: Bind {"), "{text}");
}

#[test]
fn t_shows_the_last_binding_s_scheme() {
    let text = written(Repl::new().feed(b"add = a b => a + b\n:t\n"));
    let line = text.lines().next().unwrap();
    assert!(line.starts_with("add: "), "{text}");
    assert!(line.contains("->"), "{text}");
}

#[test]
fn t_queries_an_expression_s_type() {
    let text = written(Repl::new().feed(b":t 1 + 2\n"));
    assert_eq!(text, "1 + 2: i64\nfipe> ");
}

#[test]
fn t_reports_a_type_mismatch() {
    let text = written(Repl::new().feed(b":t 1 + 2.0\n"));
    let line = text.lines().next().unwrap();
    assert!(line.contains("expected"), "{text}");
    assert!(line.contains("found"), "{text}");
}

#[test]
fn q_exits() {
    assert_eq!(Repl::new().feed(b":q\nx\n"), [Out::Exit]);
}
