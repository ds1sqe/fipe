//! Nested host modules preserve their identity through imports, types and calls.
use fipe::{
    check, compile,
    decl::{Declarations, Reason},
    embed::{Outcome, Reply, Request},
    lex::lex,
    parse::parse,
    session::Session,
    value::Value,
    vm::Machine,
};

fn host() -> Declarations {
    let mut host = Declarations::new();
    host.module("text.open", "## A buffer.\npub type Buffer { pub id: Integer }\n## Open text.\npub new: Integer -> Integer\n## Hidden.\nhidden: Integer").unwrap();
    host.module(
        "client.open",
        "## Show a region.\npub new: Integer -> Integer",
    )
    .unwrap();
    host.module(
        "text.history.edits",
        "## Number of edits.\npub count: Integer",
    )
    .unwrap();
    host
}

fn program(source: &str, host: &Declarations) -> fipe::mir::Program {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(&module, host, &[], check::Options::default())
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    compile::compile(&unit)
}

#[test]
fn every_import_form_calls_the_same_host_module() {
    let host = host();
    for source in [
        "use text.open\ntext.open.new 7",
        "use text.open\nopen.new 7",
        "use text\ntext.open.new 7",
        "use text.open.new\nnew 7",
        "text.open.new 7",
    ] {
        let program = program(source, &host);
        let mut machine = Machine::new(Some(10_000));
        let Outcome::Request(Request::Call {
            declaration,
            arguments,
            ..
        }) = machine.run(&program)
        else {
            panic!("{source}: expected a host call");
        };
        let tables = program.tables.borrow();
        let declaration = &tables.declarations[declaration];
        assert_eq!(tables.namespaces[declaration.namespace], "text.open");
        assert_eq!(declaration.label, "new");
        assert_eq!(arguments, [Value::Int(7)]);
        drop(tables);
        assert!(matches!(
            machine.reply(&program, Reply::Value(Ok(Value::Int(9)))),
            Outcome::Done(Ok(Value::Int(9)))
        ));
    }
}

#[test]
fn same_leaf_names_and_deeper_modules_remain_distinct() {
    let program = program("use text\nuse client\ntext.open.new (client.open.new text.history.edits.count)", &host());
    let mut machine = Machine::new(Some(10_000));
    let mut outcome = machine.run(&program);
    for expected in ["text.history.edits", "client.open", "text.open"] {
        let Outcome::Request(Request::Call { declaration, .. }) = outcome
        else {
            panic!("expected {expected}");
        };
        let tables = program.tables.borrow();
        assert_eq!(
            tables.namespaces[tables.declarations[declaration].namespace],
            expected
        );
        drop(tables);
        outcome = machine.reply(&program, Reply::Value(Ok(Value::Int(3))));
    }
    assert!(matches!(outcome, Outcome::Done(Ok(Value::Int(3)))));
}

#[test]
fn qualified_and_imported_types_agree_in_scripts_and_host_signatures() {
    let mut host = host();
    host.module("client.inspect", "use text.open\n## Read a buffer.\npub id: open.Buffer -> Integer\n## Read a qualified buffer.\npub full: text.open.Buffer -> Integer").unwrap();
    for source in [
        "use text\nb: text.open.Buffer = text.open.Buffer { id: 4 }\nclient.inspect.id b",
        "use text.open\nb: open.Buffer = open.Buffer { id: 4 }\nclient.inspect.full b",
        "use text.open.Buffer\nb: Buffer = Buffer { id: 4 }\nclient.inspect.id b",
    ] {
        program(source, &host);
    }
    host.module(
        "client.read",
        "use text.open.Buffer\n## Read a buffer.\npub id: Buffer -> Integer",
    )
    .unwrap();
    program("client.read.id (text.open.Buffer { id: 4 })", &host);
}

#[test]
fn sessions_keep_imported_submodules_without_requesting_files() {
    let mut session = Session::new(host());
    for source in [
        "use text.open",
        "open.new 1",
        "use text.history.edits",
        "edits.count",
    ] {
        let Outcome::Done(report) = session.line(source) else {
            panic!("host namespaces must not request files: {source}");
        };
        assert!(
            report.diagnostics.is_empty(),
            "{source}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn missing_private_and_shadowed_members_do_not_resolve_as_submodules() {
    let host = host();
    for source in [
        "use text.missing",
        "use text.open.missing",
        "use text.open.hidden",
        "text.open.hidden",
        "use text.open.new.extra",
        "text = 1\ntext.open.new 1",
    ] {
        let module = parse(lex(source)).unwrap();
        assert!(
            check::check(&module, &host, &[], check::Options::default())
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn parents_can_be_registered_before_or_after_children() {
    for child_first in [false, true] {
        let mut host = Declarations::new();
        let declarations = [
            ("text", "## Version.\npub version = 1"),
            ("text.open", "## Open.\npub new: Integer -> Integer"),
        ];
        for index in if child_first { [1, 0] } else { [0, 1] } {
            let (name, source) = declarations[index];
            host.module(name, source).unwrap();
        }
        assert_eq!(host.submodules("text").count(), 1);
        program("text.open.new text.version", &host);
    }
}

#[test]
fn invalid_names_and_member_collisions_are_rejected_without_mutation() {
    let mut host = host();
    let before = host.clone();
    for name in [
        "",
        "text..open",
        ".text",
        "text.",
        "Text.open",
        "text.Open",
        "text/open",
        "text open",
        " text",
        "text\n",
        "use",
        "commands",
        "prelude.child",
    ] {
        let errors = host.module(name, "").unwrap_err();
        assert!(
            matches!(errors[0].reason, Reason::ModuleName { .. }),
            "{name}"
        );
        assert_eq!(host, before);
    }
    let errors = host
        .module("text", "## Collision.\npub open = 1")
        .unwrap_err();
    assert!(matches!(errors[0].reason, Reason::ModuleConflict { .. }));
    assert_eq!(host, before);
    host.module("other", "## Collision.\npub open = 1").unwrap();
    let before = host.clone();
    assert!(host.module("other.open.deep", "").is_err());
    assert_eq!(host, before);
}

#[test]
fn nested_type_members_and_api_markers_keep_their_module_identity() {
    let mut host = Declarations::new();
    host.module("caps", "## A capability.\npub type Handle<Api>")
        .unwrap();
    host.module("text.open", "## A buffer.\npub type Buffer { pub id: Integer }\nimpl Buffer { pub zero: Buffer }\n## Keep a handle.\npub keep: caps.Handle<text.open> -> caps.Handle<text.open>")
        .unwrap();
    for source in [
        "text.open.Buffer.zero",
        "use text.open\nopen.Buffer.zero",
        "use text.open\nf: caps.Handle<open> -> caps.Handle<text.open> = h => open.keep h",
    ] {
        program(source, &host);
    }
    host.module(
        "client.open",
        "## A different buffer.\npub type Buffer { pub id: Integer }",
    )
    .unwrap();
    let module =
        parse(lex("b: client.open.Buffer = text.open.Buffer.zero")).unwrap();
    assert!(
        check::check(&module, &host, &[], check::Options::default()).is_err()
    );
}

#[test]
fn invalid_middle_import_does_not_bind_its_final_segment() {
    let mut host = Declarations::new();
    host.module("a", "pub n: Integer\npub type B {}").unwrap();
    for (import, tail) in [("a.n.c", "c"), ("a.B.x", "x"), ("a.nope", "nope")] {
        let source = format!("use {import}\n{tail}");
        let parsed = parse(lex(&source)).unwrap();
        let errors =
            check::check(&parsed, &host, &[], check::Options::default())
                .unwrap_err();
        assert!(
            errors.iter().any(|error| matches!(
                error,
                check::Error::Resolve(error)
                    if error.reason == fipe::resolve::Reason::NotExported
                        && &source[error.span.start..error.span.end] == import
            )),
            "{source}: {errors:?}"
        );
        assert!(
            errors.iter().any(|error| matches!(
                error,
                check::Error::Resolve(error)
                    if error.reason == fipe::resolve::Reason::UnknownName
                        && error.span.start == source.len() - tail.len()
                        && error.span.end == source.len()
            )),
            "final segment leaked: {source}: {errors:?}"
        );
    }
}
