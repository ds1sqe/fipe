//! Completion, rename and formatting through the wire, over the
//! examples, the conformance host and files written for the test.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use fipe_lsp::{serve, transport, Options};
use serde_json::{json, Value};

/// The repository the package sits in.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn examples() -> PathBuf {
    root().join("Documentation/examples")
}

fn uri(path: &Path) -> String {
    fipe_lsp::uri::of(path).as_str().to_string()
}

/// Every message the server writes for `messages`, after an
/// `initialize` offering UTF-8, with the conformance host.
fn exchange(messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    let initialize = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "capabilities": {"general": {"positionEncodings": ["utf-8"]}},
            "initializationOptions": {
                "host": root().join("tests/fixtures/host"),
            },
        },
    });
    let tail = [
        json!({"jsonrpc": "2.0", "id": 999, "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ];
    for message in std::iter::once(&initialize).chain(messages).chain(&tail) {
        transport::write(&mut input, message).unwrap();
    }
    let mut output = Vec::new();
    serve(&mut Cursor::new(input), &mut output, &Options::default()).unwrap();
    let mut read = Cursor::new(output);
    let mut replies = Vec::new();
    while let Some(message) = transport::read(&mut read).unwrap() {
        replies.push(message);
    }
    replies
}

fn open(uri: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": uri, "languageId": "fipe", "version": 1, "text": text,
        }},
    })
}

fn at(
    id: u64,
    method: &str,
    uri: &str,
    (line, character): (u32, u32),
) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": method,
        "params": {
            "textDocument": {"uri": uri},
            "position": {"line": line, "character": character},
        },
    })
}

fn rename(id: u64, uri: &str, at: (u32, u32), name: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/rename",
        "params": {
            "textDocument": {"uri": uri},
            "position": {"line": at.0, "character": at.1},
            "newName": name,
        },
    })
}

fn message(replies: &[Value], id: u64) -> &Value {
    replies
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))
}

/// The (line, character) of the end of `text`.
fn end(text: &str) -> (u32, u32) {
    let line = text.matches('\n').count();
    let start = text.rfind('\n').map_or(0, |at| at + 1);
    (
        u32::try_from(line).unwrap(),
        u32::try_from(text.len() - start).unwrap(),
    )
}

/// The (line, character) of the first `needle` in `text`, in bytes.
fn find(text: &str, needle: &str) -> (u32, u32) {
    let at = text.find(needle).unwrap_or_else(|| panic!("no {needle}"));
    let line = text[..at].matches('\n').count();
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    (
        u32::try_from(line).unwrap(),
        u32::try_from(at - start).unwrap(),
    )
}

/// The labels a completion at the end of `text` offers.
fn labels(path: &Path, text: &str) -> Vec<String> {
    let uri = uri(path);
    let replies = exchange(&[
        open(&uri, text),
        at(1, "textDocument/completion", &uri, end(text)),
    ]);
    message(&replies, 1)["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{replies:?}"))
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_string())
        .collect()
}

/// The text each edit of a rename replaces, by file name.
fn renamed(replies: &[Value], id: u64) -> Vec<(String, String)> {
    let changes = message(replies, id)["result"]["changes"]
        .as_object()
        .unwrap_or_else(|| panic!("{replies:?}"));
    let mut replaced = Vec::new();
    for (uri, edits) in changes {
        let path = PathBuf::from(uri.trim_start_matches("file://"));
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        for edit in edits.as_array().unwrap() {
            let offset = |position: &Value| {
                let line = usize::try_from(position["line"].as_u64().unwrap())
                    .unwrap();
                let character =
                    usize::try_from(position["character"].as_u64().unwrap())
                        .unwrap();
                let start: usize =
                    text.split('\n').take(line).map(|l| l.len() + 1).sum();
                start + character
            };
            let range = &edit["range"];
            let old = &text[offset(&range["start"])..offset(&range["end"])];
            replaced.push((name.clone(), old.to_string()));
        }
    }
    replaced.sort();
    replaced
}

#[test]
fn a_command_completes_its_labeled_spots_as_flags() {
    let path = root().join("scratch/flags.fipe");
    let offered = labels(&path, "ls -");
    for flag in ["-path", "-long", "-recursive", "-depth", "-depth=", "-name"] {
        assert!(offered.contains(&flag.to_string()), "{flag}: {offered:?}");
    }
    // A Bool spot is a bare flag: no `-long=`.
    assert!(!offered.contains(&"-long=".to_string()), "{offered:?}");
    assert_eq!(labels(&path, "x = ls -de"), ["-depth", "-depth="]);
    // `cat` has no labeled spot, so it has no flag.
    assert_eq!(labels(&path, "cat -"), Vec::<String>::new());
    // After a pipe, the stage's own command counts.
    let offered = labels(&path, "ls | sort -");
    assert!(offered.contains(&"-by".to_string()), "{offered:?}");
}

#[test]
fn names_in_scope_complete_with_locals_inside_an_arm() {
    let path = root().join("scratch/names.fipe");
    let offered = labels(&path, "count = 3\nf = number => number + co");
    assert!(offered.contains(&"count".to_string()), "{offered:?}");
    assert!(!offered.contains(&"number".to_string()), "{offered:?}");
    let offered = labels(&path, "count = 3\nf = number => number + nu");
    assert_eq!(offered, ["number"]);
    // Prelude and host names are in scope too.
    let offered = labels(&path, "x = ma");
    assert!(offered.contains(&"map".to_string()), "{offered:?}");
}

#[test]
fn use_completes_the_host_modules_and_the_files_beside_the_document() {
    let path = examples().join("scratch.fipe");
    let offered = labels(&path, "use ge");
    assert_eq!(offered, ["geometry"]);
    let offered = labels(&path, "use f");
    assert!(offered.contains(&"fs".to_string()), "{offered:?}");
}

#[test]
fn a_dot_completes_module_members_and_a_values_fields() {
    let path = examples().join("scratch.fipe");
    let offered = labels(&path, "use fs\nfs.");
    assert!(offered.contains(&"list".to_string()), "{offered:?}");
    let offered = labels(&path, "use geometry.Point\np = Point 3 4\nx = p.");
    for member in ["x", "y", "show", "read"] {
        assert!(
            offered.contains(&member.to_string()),
            "{member}: {offered:?}"
        );
    }
    let offered = labels(&path, "use geometry\nx = geometry.Po");
    assert_eq!(offered, ["Point", "Polar"]);
}

#[test]
fn renaming_a_pub_type_rewrites_its_uses_in_the_importing_example() {
    let path = examples().join("geometry.fipe");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, &text),
        rename(1, &uri, find(&text, "Polar:"), "Radial"),
    ]);
    let replaced = renamed(&replies, 1);
    // `use geometry.Point geometry.Polar` and `Polar.from`.
    let in_example = replaced
        .iter()
        .filter(|(file, _)| file == "05-modules.fipe")
        .count();
    assert_eq!(in_example, 2, "{replaced:?}");
    assert!(
        replaced.iter().all(|(_, old)| old == "Polar"),
        "{replaced:?}"
    );
    assert!(
        replaced
            .iter()
            .filter(|(file, _)| file == "geometry.fipe")
            .count()
            >= 3,
        "{replaced:?}"
    );
}

/// A library and the file that imports from it, written for a test.
fn library() -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rename");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("lib.fipe");
    let main = dir.join("main.fipe");
    std::fs::write(&lib, "pub inc = n => n + 1\n").unwrap();
    std::fs::write(&main, "use lib.inc\nx = inc 2\ny = inc x\n").unwrap();
    (lib, main)
}

#[test]
fn renaming_a_pub_binding_rewrites_it_in_the_file_that_imports_it() {
    let (lib, main) = library();
    let text = std::fs::read_to_string(&lib).unwrap();
    let replies = exchange(&[
        open(&uri(&lib), &text),
        rename(1, &uri(&lib), find(&text, "inc"), "step"),
    ]);
    let replaced = renamed(&replies, 1);
    let expected: Vec<(String, String)> = [
        ("lib.fipe", "inc"),
        ("main.fipe", "inc"),
        ("main.fipe", "inc"),
        ("main.fipe", "inc"),
    ]
    .iter()
    .map(|(file, old)| (file.to_string(), old.to_string()))
    .collect();
    assert_eq!(replaced, expected);
    // From the importing file, the definition and the `use` move too.
    let text = std::fs::read_to_string(&main).unwrap();
    let replies = exchange(&[
        open(&uri(&main), &text),
        rename(2, &uri(&main), find(&text, "inc 2"), "step"),
    ]);
    assert_eq!(renamed(&replies, 2), expected);
    // Definition of the imported value answers its binding in the
    // library.
    let replies = exchange(&[
        open(&uri(&main), &text),
        at(
            3,
            "textDocument/definition",
            &uri(&main),
            find(&text, "inc 2"),
        ),
    ]);
    let found = &message(&replies, 3)["result"][0];
    assert_eq!(found["uri"], uri(&lib).as_str(), "{replies:?}");
    assert_eq!(found["range"]["start"]["character"], 4);
}

#[test]
fn a_local_renames_within_its_arm_and_a_prelude_name_is_refused() {
    let path = root().join("scratch/local.fipe");
    let text = "f = n => n + n\nn = 1\n";
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, text),
        rename(1, &uri, (0, 9), "m"),
        rename(2, &uri, (0, 4), "Big"),
        open(&uri, "x = len [1]\n"),
        rename(3, &uri, (0, 5), "size"),
    ]);
    let edits = &message(&replies, 1)["result"]["changes"][&uri];
    // The parameter and its two uses; the later top-level `n` stays.
    assert_eq!(edits.as_array().unwrap().len(), 3, "{replies:?}");
    assert_eq!(message(&replies, 2)["error"]["code"], -32602);
    assert_eq!(message(&replies, 3)["error"]["code"], -32803);
}

fn formatting(id: u64, uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/formatting",
        "params": {
            "textDocument": {"uri": uri},
            "options": {"tabSize": 4, "insertSpaces": true},
        },
    })
}

#[test]
fn formatting_lays_out_a_file_and_refuses_one_with_errors() {
    let path = root().join("scratch/format.fipe");
    let uri = uri(&path);
    let messy = "x  =   1   \nf = n => {\nm = n + x\n        m\n}\n\n\n";
    let replies = exchange(&[
        open(&uri, messy),
        formatting(1, &uri),
        open(&uri, "x = = 1\n"),
        formatting(2, &uri),
    ]);
    let edits = message(&replies, 1)["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1, "{replies:?}");
    assert_eq!(
        edits[0]["newText"],
        "x = 1\nf = n => {\n    m = n + x\n    m\n}\n"
    );
    assert_eq!(message(&replies, 2)["error"]["code"], -32803);
}

#[test]
fn the_examples_are_at_a_fixed_point_and_come_back_to_it() {
    for entry in std::fs::read_dir(examples()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "fipe") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let formatted = fipe_lsp::format::format(&text).unwrap();
        assert_eq!(formatted, text, "{}", path.display());
        // Flattened indentation and spread gaps come back to the same
        // text, which therefore parses to the same tree.
        // A doc's trailing spaces are its own (a Markdown line break).
        let messy: String = text
            .lines()
            .map(|line| {
                let line = line.trim_start();
                if line.starts_with("##") {
                    format!("{line}\n")
                } else {
                    format!("{line}  \n")
                }
            })
            .collect();
        let again = fipe_lsp::format::format(&messy).unwrap();
        assert_eq!(again, text, "{}", path.display());
        assert_eq!(
            fipe::parse::parse(fipe::lex::lex(&again)),
            fipe::parse::parse(fipe::lex::lex(&text))
        );
        // Over the wire, an example formats to no edit.
        let uri = uri(&path);
        let replies = exchange(&[open(&uri, &text), formatting(1, &uri)]);
        assert_eq!(
            message(&replies, 1)["result"],
            json!([]),
            "{}",
            path.display()
        );
    }
}

#[test]
fn formatting_is_syntax_based_even_with_unknown_names() {
    let uri = uri(&root().join("scratch/format-semantic.fipe"));
    let text = "unknown  =   missing\n";
    let replies = exchange(&[open(&uri, text), formatting(1, &uri)]);
    let edits = message(&replies, 1)["result"].as_array().unwrap();
    assert_eq!(edits[0]["newText"], fipe::format::format(text).unwrap());
}
