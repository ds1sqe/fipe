//! Every `fipe` snippet in the README, tour and agent guide is a
//! whole script that checks against the std host.
use std::{fs, path::Path, process::Command};

#[test]
fn every_tour_snippet_checks() {
    let tour =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Documentation/tour");
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tour");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let mut pages: Vec<_> = fs::read_dir(&tour)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    pages.push(tour.join("../../README.md"));
    pages.push(tour.join("../agents.md"));
    pages.sort();
    let mut snippets = 0;
    for page in pages {
        let text = fs::read_to_string(&page).unwrap();
        let stem = page.file_stem().unwrap().to_string_lossy().into_owned();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("```fipe\n") {
            let body = &rest[start + 8..];
            let end = body.find("\n```").expect("a closed fence");
            let script = scratch.join(format!("{stem}-{snippets}.fipe"));
            fs::write(&script, &body[..=end]).unwrap();
            let checked = Command::new(env!("CARGO_BIN_EXE_fipe"))
                .arg("check")
                .arg(&script)
                .current_dir(&scratch)
                .output()
                .unwrap();
            assert!(
                checked.status.success(),
                "{}: snippet {snippets} does not check:\n{}",
                page.display(),
                String::from_utf8_lossy(&checked.stderr)
            );
            snippets += 1;
            rest = &body[end + 4..];
        }
    }
    assert!(snippets >= 10, "{snippets} snippets");
}
