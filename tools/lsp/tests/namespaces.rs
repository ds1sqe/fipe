//! Nested host declarations work from files and through editor operations.
use fipe_lsp::{actions::complete, analysis::Analysis, loader};
use std::{fs, path::Path};

#[test]
fn dotted_host_files_check_navigate_and_complete() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lsp-namespaces");
    fs::create_dir_all(&dir).unwrap();
    let source = "## Open text.\npub new: (handle: Integer) -> Integer";
    fs::write(dir.join("text.open.fipe"), source).unwrap();
    fs::write(dir.join("client.open.fipe"), source).unwrap();
    let host = loader::host(Some(&dir));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    let analyse = |text: &str| Analysis::new(text, None, &host, &|_| None);
    for text in ["use text.open\nopen.new 1", "use text\ntext.open.new 1"] {
        let analysis = analyse(text);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let offset = text.rfind("new").unwrap();
        assert!(analysis.hover(&host, offset).unwrap().contains("Integer"));
        let (_, target) = analysis.target_at(offset).unwrap();
        assert_eq!(analysis.definition(&host, &target).len(), 1);
    }
    for (text, expected) in [
        ("use text.", "open"),
        ("text.open.", "new"),
        ("use text.open\nopen.", "new"),
        ("text.open.new -", "-handle="),
    ] {
        let analysis = analyse(text);
        let offered =
            complete::complete(&analysis, &host, None, text.len(), &analyse);
        assert!(
            offered.iter().any(|item| item.label == expected),
            "{text}: {offered:?}"
        );
    }
}

#[test]
fn selected_host_failures_are_reported() {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lsp-host-missing");
    let _ = fs::remove_dir_all(&path);
    let host = loader::host(Some(&path));
    assert!(
        !host.errors.is_empty(),
        "missing selected directory was ignored"
    );
}

#[test]
fn host_loading_keeps_read_and_located_declaration_errors() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lsp-host-errors");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    assert!(loader::host(None).errors.is_empty());
    assert!(loader::host(Some(&dir)).errors.is_empty());
    fs::write(dir.join("bytes.fipe"), [255]).unwrap();
    fs::write(dir.join("bad.fipe"), "pub ???").unwrap();
    let host = loader::host(Some(&dir));
    assert!(
        host.errors
            .iter()
            .any(|error| error.contains("bytes.fipe: cannot read:")),
        "{:?}",
        host.errors
    );
    assert!(
        host.errors
            .iter()
            .any(|error| error.contains("bad.fipe:1:")),
        "{:?}",
        host.errors
    );
    let host = loader::host(Some(&dir.join("bytes.fipe")));
    assert!(host.errors[0].contains("cannot enumerate:"));
}
