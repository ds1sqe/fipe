use super::*;

#[test]
fn docs_keep_text_and_comments_keep_spans() {
    let input = "# ordinary\n##x\n## x\n##  x\n##! module\n";
    let result = lex(input);
    assert_eq!(result.errors, []);
    assert_eq!(result.comments, [Span { start: 0, end: 10 }]);
    assert_eq!(
        Source::new(input).slice(result.comments[0]),
        Some("# ordinary")
    );
    tokens(
        input,
        vec![
            Kind::Newline,
            Kind::Doc("x".into()),
            Kind::Newline,
            Kind::Doc("x".into()),
            Kind::Newline,
            Kind::Doc(" x".into()),
            Kind::Newline,
            Kind::InnerDoc("module".into()),
            Kind::Newline,
        ],
    );
    tokens(
        "a | ## doc\nb",
        vec![name("a"), S(Pipe), Kind::Doc("doc".into()), name("b")],
    );
    for input in ["##\0", "##!\u{200b}"] {
        assert_eq!(lex(input).errors.len(), 1);
    }
    for kind in [Kind::Doc(" x".into()), Kind::InnerDoc(String::new())] {
        assert_eq!(lex(&kind.to_string()).tokens[0].kind, kind);
    }
}

#[test]
fn multiline_quotes_keep_bytes_affixes_and_physical_positions() {
    let input = "re'한\n # \"'i |\nnext";
    let result = lex(input);
    assert_eq!(result.errors, []);
    assert_eq!(result.comments, []);
    assert_eq!(
        result.tokens[0].kind,
        affixed(
            "re",
            Written::Text {
                value: "한\n # \"".as_bytes().to_vec(),
                raw: true,
            },
            "i"
        )
    );
    assert_eq!(
        Source::new(input).slice(result.tokens[0].span),
        Some("re'한\n # \"'i")
    );
    assert_eq!(
        Source::new(input).line_col(result.tokens[2].span.start),
        (3, 1)
    );
    tokens(
        "'\n  Fipe\n    a pipeline language\n'",
        vec![string("\n  Fipe\n    a pipeline language\n", true)],
    );
    tokens(
        "\"the file \\\n       was not found\"",
        vec![string("the file was not found", false)],
    );
    tokens(
        "\"line one\\n\\\n\t\u{3000}line two\"",
        vec![string("line one\nline two", false)],
    );
    tokens("b\"a\\\n  \\xFF\"", vec![text("b", b"a\xff", "")]);
    let result = lex("'unfinished\nnext");
    assert_eq!(
        result.errors[0].reason,
        fipe::lex::Reason::UnterminatedQuote
    );
    assert_eq!(result.errors[0].span, Span { start: 0, end: 16 });
    assert_eq!(result.tokens.len(), 1);
    let result = lex("\"unfinished\nnext");
    assert_eq!(result.errors[0].span.end, 11);
    assert_eq!(result.tokens[1].kind, name("next"));
}
