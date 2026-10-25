use fipe::{
    analysis::{self, InputError, Limits, Options, Severity, Sources},
    check,
    decl::Declarations,
    lex::lex,
    parse::parse_partial,
    resolve::Origin,
};

fn analyze(text: &str) -> analysis::Snapshot {
    let module = parse_partial(lex(text));
    analysis::analyze(
        &module,
        &Declarations::new(),
        &[],
        Sources {
            texts: &[text],
            current: 0,
            origins: &[],
            current_file: None,
        },
        Options::default(),
    )
    .unwrap()
}

#[test]
fn analysis_keeps_partial_stages_and_explicit_codes() {
    let snapshot = analyze("a = 1\na + unknown");
    assert!(snapshot.resolved.expressions().next().is_some());
    assert!(snapshot.typed.bindings().next().is_some());
    assert!(snapshot.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "resolve.unknown-name"
            && diagnostic.severity == Severity::Error
    }));
    assert!(!analyze("a = (").diagnostics.is_empty());
}

#[test]
fn analysis_matches_check_diagnostics_without_message_codes() {
    let text = "a: Integer = True";
    let module = parse_partial(lex(text));
    let declarations = Declarations::new();
    let errors =
        check::check(&module, &declarations, &[], check::Options::default())
            .unwrap_err();
    let snapshot = analyze(text);
    assert_eq!(snapshot.diagnostics.len(), errors.len());
    assert!(snapshot
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "type.mismatch"));
    let changed = analyze("a: Integer = False");
    assert_eq!(snapshot.diagnostics[0].code, changed.diagnostics[0].code);
    assert!(!analyze("a = 1\na + 2")
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.severity == Severity::Error }));
}

#[test]
fn analysis_preserves_unicode_crlf_and_eof_byte_spans() {
    let text = "label = \"é\"\r\nmissing";
    let diagnostic = analyze(text)
        .diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == "resolve.unknown-name")
        .unwrap();
    assert_eq!(
        &text[diagnostic.location.span.start..diagnostic.location.span.end],
        "missing"
    );
    assert_eq!(
        diagnostic.location.span.start,
        text.find("missing").unwrap()
    );
    for diagnostic in analyze("a = (").diagnostics {
        assert!(diagnostic.location.span.end <= "a = (".len());
    }
}

#[test]
fn analysis_file_error_retains_its_source_and_related_use() {
    let main = "use broken\nbroken.value";
    let file = "pub value: Integer = True";
    let files = vec![(String::from("broken"), parse_partial(lex(file)))];
    let origins = [(Origin::File(String::from("broken")), 1)];
    let snapshot = analysis::analyze(
        &parse_partial(lex(main)),
        &Declarations::new(),
        &files,
        Sources {
            texts: &[main, file],
            current: 0,
            origins: &origins,
            current_file: None,
        },
        Options::default(),
    )
    .unwrap();
    let diagnostic = snapshot
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.location.source == 1)
        .unwrap();
    assert!(file
        .get(diagnostic.location.span.start..diagnostic.location.span.end)
        .is_some());
    assert!(diagnostic.related.iter().any(|related| {
        related.location.source == 0
            && related.message == "imported here"
            && &main[related.location.span.start..related.location.span.end]
                == "broken"
    }));
}

#[test]
fn analysis_missing_module_is_a_diagnostic_without_loading() {
    assert!(analyze("use absent")
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.code == "resolve.unknown-module" }));
}

#[test]
fn analysis_admission_errors_are_not_clean_snapshots() {
    fn error<E: core::error::Error>() {}
    error::<InputError>();
    let text = "1";
    let module = parse_partial(lex(text));
    let declarations = Declarations::new();
    let sources = Sources {
        texts: &[text],
        current: 0,
        origins: &[],
        current_file: None,
    };
    for (limits, expected) in [
        (
            Limits {
                max_source_bytes: Some(0),
                ..Limits::default()
            },
            InputError::SourceLimit { source: 0 },
        ),
        (
            Limits {
                max_total_bytes: Some(0),
                ..Limits::default()
            },
            InputError::TotalLimit,
        ),
        (
            Limits {
                max_sources: Some(0),
                ..Limits::default()
            },
            InputError::SourceCountLimit,
        ),
    ] {
        assert!(matches!(analysis::analyze(
            &module, &declarations, &[], sources,
            Options { limits, ..Options::default() },
        ), Err(actual) if actual == expected));
    }
    assert!(analysis::analyze(
        &module,
        &declarations,
        &[],
        sources,
        Options {
            limits: Limits {
                max_source_bytes: Some(1),
                max_total_bytes: Some(1),
                max_sources: Some(1),
            },
            ..Options::default()
        },
    )
    .is_ok());
    let bad = Sources {
        current: 1,
        ..sources
    };
    assert!(matches!(
        analysis::analyze(&module, &declarations, &[], bad, Options::default(),),
        Err(InputError::InvalidSourceMap)
    ));
}

#[test]
fn analysis_import_cycle_alias_and_two_file_origins_are_exact() {
    let main = "use a";
    let other = "use main";
    let files = vec![
        (String::from("a"), parse_partial(lex(other))),
        (String::from("main"), parse_partial(lex(main))),
    ];
    let origins = [
        (Origin::File(String::from("a")), 1),
        (Origin::File(String::from("main")), 0),
    ];
    let snapshot = analysis::analyze(
        &parse_partial(lex(main)),
        &Declarations::new(),
        &files,
        Sources {
            texts: &[main, other],
            current: 0,
            origins: &origins,
            current_file: Some("main"),
        },
        Options::default(),
    )
    .unwrap();
    assert!(snapshot
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "resolve.import-cycle"));
    let main = "use one\nuse two";
    let one = "pub first: Integer = True";
    let two = "pub second: I64 = 2.5";
    let files = vec![
        (String::from("one"), parse_partial(lex(one))),
        (String::from("two"), parse_partial(lex(two))),
    ];
    let origins = [
        (Origin::File(String::from("one")), 1),
        (Origin::File(String::from("two")), 2),
    ];
    let snapshot = analysis::analyze(
        &parse_partial(lex(main)),
        &Declarations::new(),
        &files,
        Sources {
            texts: &[main, one, two],
            current: 0,
            origins: &origins,
            current_file: None,
        },
        Options::default(),
    )
    .unwrap();
    for (source, code) in [(1, "type.mismatch"), (2, "type.not-exact")] {
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.location.source == source && diagnostic.code == code
        }));
    }
}

#[test]
fn analysis_host_declaration_location_is_not_the_current_source() {
    let text = "f = _ => Wrap 1";
    let supplied = format!(
        "# {}\npub type Wrap {{ pub value: Integer }}\nimpl Wrap {{ x => missing }}",
        "padding".repeat(200),
    );
    let mut declarations = Declarations::new();
    declarations.commands(&supplied).unwrap();
    let module = parse_partial(lex(text));
    let origins = [(Origin::Commands, 1)];
    let snapshot = analysis::analyze(
        &module,
        &declarations,
        &[],
        Sources {
            texts: &[text, &supplied],
            current: 0,
            origins: &origins,
            current_file: None,
        },
        Options::default(),
    )
    .unwrap();
    let diagnostic = snapshot
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "type.declaration-value")
        .unwrap();
    assert_eq!(diagnostic.location.source, 0);
    assert!(diagnostic.related.iter().any(|related| {
        related.location.source == 1
            && related.message == "declared here"
            && related.location.span.start > text.len()
    }));
    assert!(matches!(
        analysis::analyze(
            &module,
            &declarations,
            &[],
            Sources {
                texts: &[text],
                current: 0,
                origins: &[],
                current_file: None
            },
            Options::default(),
        ),
        Err(InputError::InvalidSourceMap)
    ));
}

#[test]
fn analysis_rejects_conflicting_source_maps() {
    let text = "1";
    let module = parse_partial(lex(text));
    let declarations = Declarations::new();
    for origins in [
        vec![(Origin::This, 1)],
        vec![(Origin::File(String::from("main")), 1)],
        vec![(Origin::This, 0), (Origin::This, 0)],
    ] {
        assert!(matches!(
            analysis::analyze(
                &module,
                &declarations,
                &[],
                Sources {
                    texts: &[text, "2"],
                    current: 0,
                    origins: &origins,
                    current_file: Some("main")
                },
                Options::default(),
            ),
            Err(InputError::InvalidSourceMap)
        ));
    }
}
