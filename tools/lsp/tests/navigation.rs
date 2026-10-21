//! Navigation and editing at the edges: positions after non-ASCII text
//! and at a name's end, members, flags, declared names, quotes, line
//! endings and the diagnostics of used files.
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

fn references(id: u64, uri: &str, (line, character): (u32, u32)) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/references",
        "params": {
            "textDocument": {"uri": uri},
            "position": {"line": line, "character": character},
            "context": {"includeDeclaration": true},
        },
    })
}

/// The first result of request `id`, as JSON.
fn result(replies: &[Value], id: u64) -> &Value {
    &message(replies, id)["result"]
}

/// The hover text at `position` of `text`, written at `path`.
fn hover(path: &Path, text: &str, position: (u32, u32)) -> String {
    let uri = uri(path);
    let replies = exchange(&[
        open(&uri, text),
        at(1, "textDocument/hover", &uri, position),
    ]);
    result(&replies, 1)["contents"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn text_after_a_non_ascii_character_does_not_end_the_server() {
    let path = root().join("scratch/unicode.fipe");
    let uri = uri(&path);
    let nbsp = "x = \"a\u{a0}b\"\ny = x\n";
    let dash = "x = \"\u{2014}ab\"\ny = x\n";
    let replies = exchange(&[
        open(&uri, nbsp),
        at(1, "textDocument/completion", &uri, (0, 8)),
        at(2, "textDocument/completion", &uri, (1, 5)),
        open(&uri, dash),
        references(3, &uri, (0, 7)),
        rename(4, &uri, (0, 7), "z"),
        references(5, &uri, (1, 4)),
    ]);
    for id in 1..=5 {
        let reply = message(&replies, id);
        assert!(
            reply.get("result").is_some() || reply.get("error").is_some(),
            "{reply:?}"
        );
    }
    // `y = x`: the use of x is found from its line.
    let found = result(&replies, 5).as_array();
    assert!(found.is_some_and(|found| !found.is_empty()), "{replies:?}");
}

#[test]
fn a_position_just_after_a_name_is_on_the_name() {
    let path = root().join("scratch/end.fipe");
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, "inc = n => n + 1\ny = inc 2\n"),
        at(1, "textDocument/definition", &uri, (1, 7)),
    ]);
    let locations = result(&replies, 1).as_array();
    let locations = locations.unwrap_or_else(|| panic!("{replies:?}"));
    assert_eq!(locations.len(), 1, "{replies:?}");
    assert_eq!(locations[0]["range"]["start"]["line"], 0);
}

const RECORD: &str = "type R { pub v: Integer, w: Integer = 0 }\n\
                      impl R { twice = self => self.v * 2 }\n\
                      b = R 4\n\
                      y = b.v\n\
                      k = b.twice\n";

#[test]
fn rename_at_a_member_is_refused_and_at_its_head_renames_the_head() {
    let path = root().join("scratch/record.fipe");
    let uri = uri(&path);
    let member = find(RECORD, "b.v");
    let replies = exchange(&[
        open(&uri, RECORD),
        rename(1, &uri, (member.0, member.1 + 2), "zz"),
        rename(2, &uri, member, "c"),
        rename(3, &uri, find(RECORD, "b.twice"), "y"),
    ]);
    assert!(message(&replies, 1).get("error").is_some(), "{replies:?}");
    let edits = result(&replies, 2)["changes"][&uri].as_array().unwrap();
    // b's binding and its two uses; never the field.
    assert_eq!(edits.len(), 3, "{edits:?}");
    // `y` is already written in the file.
    assert!(message(&replies, 3).get("error").is_some(), "{replies:?}");
}

#[test]
fn definition_reaches_members_and_prelude_declarations() {
    let path = root().join("scratch/members.fipe");
    let uri = uri(&path);
    let text = format!("{RECORD}n = [1, 2] | filter (x => x > 1)\n");
    let twice = find(&text, "b.twice");
    let field = find(&text, "self.v");
    let replies = exchange(&[
        open(&uri, &text),
        at(1, "textDocument/definition", &uri, (twice.0, twice.1 + 3)),
        at(2, "textDocument/definition", &uri, (field.0, field.1 + 5)),
        at(3, "textDocument/definition", &uri, find(&text, "filter")),
    ]);
    let at_line = |id: u64| {
        let locations = result(&replies, id).as_array();
        let first = locations.and_then(|locations| locations.first());
        first.unwrap_or_else(|| panic!("{id}: {replies:?}"))["range"]["start"]
            ["line"]
            .clone()
    };
    assert_eq!(at_line(1), 1, "{replies:?}");
    assert_eq!(at_line(2), 0, "{replies:?}");
    let prelude = &result(&replies, 3)[0]["uri"];
    assert!(
        prelude
            .as_str()
            .is_some_and(|uri| uri.ends_with("prelude.fipe")),
        "{replies:?}"
    );
}

#[test]
fn hover_shows_declared_names_patterns_and_a_flags_spot() {
    let path = root().join("scratch/hover.fipe");
    let text = format!(
        "{RECORD}f = days e => e + days\n\
         [a, c] = [1, 2]\n\
         l = ls -l /var/log\n"
    );
    let type_name = hover(&path, &text, find(&text, "R {"));
    assert!(type_name.contains("type R"), "{type_name}");
    let field = hover(&path, &text, find(&text, "v: Integer"));
    assert!(field.contains("pub v: Integer"), "{field}");
    let member = hover(&path, &text, find(&text, "twice ="));
    assert!(member.contains("twice"), "{member}");
    let parameter = hover(&path, &text, find(&text, "days e"));
    assert!(parameter.starts_with("```fipe\ndays:"), "{parameter}");
    let bound = hover(&path, &text, find(&text, "a, c"));
    assert!(bound.starts_with("```fipe\na:"), "{bound}");
    let flag = hover(&path, &text, find(&text, "-l"));
    assert!(flag.contains("long: Bool"), "{flag}");
    assert!(flag.contains("include detailed entries"), "{flag}");
}

#[test]
fn completion_offers_nothing_in_a_quote_and_names_at_a_flags_value() {
    let path = root().join("scratch/quote.fipe");
    let none: Vec<String> = Vec::new();
    assert_eq!(labels(&path, "words = 1\nx = \"hello wor"), none);
    assert_eq!(labels(&path, "words = 1\nx = 1 # wor"), none);
    let value = labels(&path, "depth = 2\nx = ls -depth=de");
    assert!(value.contains(&String::from("depth")), "{value:?}");
    assert!(
        !value.iter().any(|label| label.starts_with('-')),
        "{value:?}"
    );
}

#[test]
fn formatting_keeps_crlf_line_endings() {
    let path = root().join("scratch/crlf.fipe");
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, "a  =  1\r\nc = a\r\n"),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/formatting",
            "params": {
                "textDocument": {"uri": uri},
                "options": {"tabSize": 4, "insertSpaces": true},
            },
        }),
    ]);
    let edits = result(&replies, 1).as_array().unwrap();
    assert_eq!(edits[0]["newText"], "a = 1\r\nc = a\r\n", "{replies:?}");
}

#[test]
fn a_used_files_errors_are_shown_once_at_its_use() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.fipe"), "pub x = )\npub y = 2\nz = (\n")
        .unwrap();
    let main = dir.join("main.fipe");
    let text = "use bad\nz = bad.y\n";
    std::fs::write(&main, text).unwrap();
    let uri = uri(&main);
    let replies = exchange(&[open(&uri, text)]);
    let diagnostics = replies
        .iter()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
        .unwrap()["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["range"]["start"]["line"], 0);
    let message = diagnostics[0]["message"].as_str().unwrap();
    assert!(message.starts_with("in bad.fipe, line 1:"), "{message}");
}

#[test]
fn a_cycle_through_the_document_is_reported_in_its_own_words() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cycle");
    std::fs::create_dir_all(&dir).unwrap();
    let c1 = dir.join("c1.fipe");
    std::fs::write(&c1, "use c2\npub a = 1\n").unwrap();
    std::fs::write(dir.join("c2.fipe"), "use c1\npub b = 2\n").unwrap();
    let uri = uri(&c1);
    let replies = exchange(&[open(&uri, "use c2\npub a = 1\n")]);
    let text = replies
        .iter()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
        .unwrap()
        .to_string();
    assert!(!text.contains("in c1.fipe"), "{text}");
    assert!(text.contains("cycle"), "{text}");
}

#[test]
fn a_dot_after_a_tuple_or_a_vector_offers_its_own_members() {
    let path = root().join("scratch/dot.fipe");
    let tuple = labels(&path, "t = (1, 2)\nu = t.");
    assert!(!tuple.contains(&String::from("add")), "{tuple:?}");
    let vector = labels(&path, "x = [1, 2]\ny = x.");
    assert!(vector.contains(&String::from("len")), "{vector:?}");
}
