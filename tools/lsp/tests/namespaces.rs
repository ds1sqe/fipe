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
