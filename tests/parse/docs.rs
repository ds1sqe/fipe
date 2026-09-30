use fipe::{
    ast::{Entry, ExprKind, ImplMember, ItemKind, Member},
    lex::lex,
    parse::{parse, Reason},
};

#[test]
fn docs_attach_to_each_owner_and_preserve_trivia() {
    let source = "##! Module\nuse fs\n# ordinary\n## a\n##\n## b\n\nx = 1\n## Record\ntype P {\n## horizontal\npub x: I64,\n}\ntype S {\n## choice\nC,\n}\nimpl P {\n## member\nf = self => self,\n## constructor\nx => Self { x },\n}\nf = {\n## local\ny = 2\ny\n}";
    let module = parse(lex(source)).unwrap();
    assert_eq!(module.doc.as_deref(), Some("Module"));
    assert_eq!(module.comments.len(), 1);
    assert_eq!(
        &source[module.comments[0].start..module.comments[0].end],
        "# ordinary"
    );
    let Entry::Line(id) = module.entries[1] else {
        panic!()
    };
    assert!(
        matches!(&module.expr(id).unwrap().kind, ExprKind::Bind { doc: Some(doc), .. } if doc == "a\n\nb")
    );
    let Entry::Item(item) = &module.entries[2] else {
        panic!()
    };
    assert_eq!(item.doc.as_deref(), Some("Record"));
    let ItemKind::Type {
        members: Some(members),
        ..
    } = &item.kind
    else {
        panic!()
    };
    assert!(
        matches!(&members[0], Member::Field(field) if field.doc.as_deref() == Some("horizontal"))
    );
    let Entry::Item(item) = &module.entries[3] else {
        panic!()
    };
    let ItemKind::Type {
        members: Some(members),
        ..
    } = &item.kind
    else {
        panic!()
    };
    assert!(
        matches!(&members[0], Member::Constructor { doc: Some(doc), .. } if doc == "choice")
    );
    let Entry::Item(item) = &module.entries[4] else {
        panic!()
    };
    let ItemKind::Impl { members, .. } = &item.kind else {
        panic!()
    };
    assert!(
        matches!(&members[0], ImplMember::Named { doc: Some(doc), .. } if doc == "member")
    );
    assert!(
        matches!(&members[1], ImplMember::Constructor { doc: Some(doc), .. } if doc == "constructor")
    );
    let Entry::Line(id) = module.entries[5] else {
        panic!()
    };
    let ExprKind::Bind { value, .. } = module.expr(id).unwrap().kind else {
        panic!()
    };
    let ExprKind::Block(lines) = &module.expr(value).unwrap().kind else {
        panic!()
    };
    assert!(
        matches!(&module.expr(lines[0]).unwrap().kind, ExprKind::Bind { doc: Some(doc), .. } if doc == "local")
    );
}

#[test]
fn misplaced_docs_are_syntax_errors() {
    for source in [
        "## orphan",
        "## doc\n1",
        "## doc\nuse fs",
        "## doc\n##! module",
        "type P {\n## orphan\n}",
    ] {
        let errors = parse(lex(source)).unwrap_err();
        assert_eq!(errors[0].reason, Reason::UnattachedDoc, "{source}");
    }
    for source in ["x = 1\n##! late", "{\n##! nested\nx = 1\n}"] {
        assert_eq!(
            parse(lex(source)).unwrap_err()[0].reason,
            Reason::InnerDocPosition
        );
    }
    assert!(parse(lex("type S { C(\n## field\nx: I64) }")).is_err());
}

#[test]
fn invalid_comment_spans_are_rejected() {
    use fipe::token::Span;
    for spans in [
        vec![Span { start: 0, end: 1 }],
        vec![Span { start: 2, end: 10 }],
        vec![Span { start: 2, end: 2 }],
        vec![Span { start: 2, end: 5 }, Span { start: 3, end: 4 }],
    ] {
        let mut input = lex("x # c");
        input.comments = spans;
        assert_eq!(
            parse(input).unwrap_err()[0].reason,
            Reason::InvalidTokenStream
        );
    }
}

#[test]
fn separated_doc_runs_preserve_paragraphs() {
    let module =
        parse(lex("##! module\n## first\n\n\n## second\nx = 1")).unwrap();
    assert_eq!(module.doc.as_deref(), Some("module"));
    let Entry::Line(id) = module.entries[0] else {
        panic!()
    };
    assert!(
        matches!(&module.expr(id).unwrap().kind, ExprKind::Bind { doc: Some(doc), .. } if doc == "first\n\nsecond")
    );
}

#[test]
fn the_ruled_doc_example_parses() {
    let chapter = include_str!("../../Documentation/spec/syntax.md");
    let source = chapter
        .rsplit_once("```text\n")
        .unwrap()
        .1
        .split("```")
        .next()
        .unwrap();
    let module = parse(lex(source)).unwrap();
    assert_eq!(
        module.doc.as_deref(),
        Some("Files: the shell reading of Fipe.")
    );
    let Entry::Line(id) = module.entries[1] else {
        panic!()
    };
    assert!(
        matches!(&module.expr(id).unwrap().kind, ExprKind::Bind { doc: Some(doc), .. } if doc.contains("- depth: how far down to look"))
    );
}
