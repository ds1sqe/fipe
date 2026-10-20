//! The server through the wire: framed requests in, framed replies and
//! notifications out, over the examples and the conformance host.

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
/// `initialize` offering `encodings`, with the conformance host.
fn exchange(encodings: &[&str], messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    let initialize = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "capabilities": {"general": {"positionEncodings": encodings}},
            "initializationOptions": {
                "host": root().join("tests/fixtures/host"),
            },
        },
    });
    let initialized =
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}});
    let tail = [
        json!({"jsonrpc": "2.0", "id": 999, "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ];
    for message in [initialize, initialized]
        .iter()
        .chain(messages)
        .chain(&tail)
    {
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

fn change(uri: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": text}],
        },
    })
}

fn at(id: u64, method: &str, uri: &str, line: u32, character: u32) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": method,
        "params": {
            "textDocument": {"uri": uri},
            "position": {"line": line, "character": character},
            "context": {"includeDeclaration": true},
        },
    })
}

fn reply(replies: &[Value], id: u64) -> &Value {
    &replies
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))["result"]
}

fn published(replies: &[Value]) -> Vec<&Value> {
    replies
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
        })
        .map(|message| &message["params"]["diagnostics"])
        .collect()
}

/// The (line, character) of the first `needle` in `text`, in bytes.
fn find(text: &str, needle: &str) -> (u32, u32) {
    let at = text.find(needle).unwrap_or_else(|| panic!("no {needle}"));
    let line = text[..at].matches('\n').count();
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let column = at - start;
    (u32::try_from(line).unwrap(), u32::try_from(column).unwrap())
}

#[test]
fn initialize_takes_utf8_when_offered_and_utf16_otherwise() {
    let replies = exchange(&["utf-8", "utf-16"], &[]);
    let capabilities = &reply(&replies, 0)["capabilities"];
    assert_eq!(capabilities["positionEncoding"], "utf-8");
    assert_eq!(capabilities["definitionProvider"], true);
    let replies = exchange(&[], &[]);
    assert_eq!(
        reply(&replies, 0)["capabilities"]["positionEncoding"],
        "utf-16"
    );
}

#[test]
fn a_check_error_is_published_at_its_span_and_cleared_when_fixed() {
    let path = root().join("scratch/guard.fipe");
    let uri = uri(&path);
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, "find x | cat\n"),
            change(&uri, "find x |: cat\n"),
        ],
    );
    let published = published(&replies);
    assert_eq!(published.len(), 2, "{replies:?}");
    let first = published[0].as_array().unwrap();
    assert_eq!(first.len(), 1, "{first:?}");
    // The crate reports the outcome guard at the piped input, which is
    // where the server places it.
    let range = &first[0]["range"];
    assert_eq!(range["start"], json!({"line": 0, "character": 0}));
    assert_eq!(range["end"], json!({"line": 0, "character": 6}));
    assert_eq!(first[0]["message"], "expected Entry, found Maybe<Entry>");
    assert_eq!(published[1], &json!([]));
}

#[test]
fn a_malformed_line_is_reported_and_the_rest_still_answers() {
    let path = root().join("scratch/partial.fipe");
    let uri = uri(&path);
    let text = "a = 1\nb = )\nc = a + 1\n";
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, text),
            at(1, "textDocument/definition", &uri, 2, 4),
        ],
    );
    let diagnostics = published(&replies)[0].as_array().unwrap().clone();
    assert!(
        diagnostics.iter().any(|d| d["range"]["start"]["line"] == 1),
        "{diagnostics:?}"
    );
    let definition = reply(&replies, 1).as_array().unwrap();
    assert_eq!(
        definition[0]["range"]["start"],
        json!({"line": 0, "character": 0})
    );
}

#[test]
fn definition_of_a_used_name_answers_the_other_files_span() {
    let path = root().join("Documentation/examples/05-modules.fipe");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = uri(&path);
    let (line, character) = find(&text, "p: Point");
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, &text),
            at(1, "textDocument/definition", &uri, line, character + 3),
        ],
    );
    let locations = reply(&replies, 1).as_array().unwrap();
    assert_eq!(locations.len(), 1, "{replies:?}");
    let geometry = root().join("Documentation/examples/geometry.fipe");
    assert_eq!(locations[0]["uri"], uri_of_str(&geometry));
    let geometry_text = std::fs::read_to_string(&geometry).unwrap();
    let (line, character) = find(&geometry_text, "Point:");
    assert_eq!(
        locations[0]["range"]["start"],
        json!({"line": line, "character": character})
    );
}

fn uri_of_str(path: &Path) -> Value {
    Value::String(uri(path))
}

#[test]
fn references_list_every_use_and_the_declaration() {
    let path = root().join("scratch/refs.fipe");
    let uri = uri(&path);
    let text = "inc = n => n + 1\na = inc 1\nb = inc (inc 2)\n";
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, text),
            at(1, "textDocument/references", &uri, 1, 4),
        ],
    );
    let locations = reply(&replies, 1).as_array().unwrap();
    let mut starts: Vec<(u64, u64)> = locations
        .iter()
        .map(|l| {
            let start = &l["range"]["start"];
            (
                start["line"].as_u64().unwrap(),
                start["character"].as_u64().unwrap(),
            )
        })
        .collect();
    starts.sort_unstable();
    assert_eq!(starts, [(0, 0), (1, 4), (2, 4), (2, 9)], "{locations:?}");
}

#[test]
fn hover_shows_a_bindings_inferred_scheme() {
    let path = root().join("scratch/hover.fipe");
    let uri = uri(&path);
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, "add = a b => a + b\nadd 1 2\n"),
            at(1, "textDocument/hover", &uri, 0, 1),
            at(2, "textDocument/hover", &uri, 1, 1),
        ],
    );
    let scheme = reply(&replies, 1)["contents"]["value"].as_str().unwrap();
    assert!(scheme.contains("add: "), "{scheme}");
    assert!(scheme.contains("->"), "{scheme}");
    let used = reply(&replies, 2)["contents"]["value"].as_str().unwrap();
    assert!(used.contains("add: "), "{used}");
}

#[test]
fn hover_on_a_command_shows_its_declaration_and_doc() {
    let path = root().join("scratch/command.fipe");
    let uri = uri(&path);
    let replies = exchange(
        &["utf-8"],
        &[
            open(&uri, "cat ./notes\n"),
            at(1, "textDocument/hover", &uri, 0, 1),
        ],
    );
    let text = reply(&replies, 1)["contents"]["value"].as_str().unwrap();
    assert!(text.contains("pub cat: Entry -> String"), "{text}");
    assert!(text.contains("## Read an entry as text."), "{text}");
}

#[test]
fn an_unknown_request_is_method_not_found() {
    let replies = exchange(
        &[],
        &[json!({"jsonrpc": "2.0", "id": 5, "method": "nope", "params": {}})],
    );
    let error = &replies.iter().find(|m| m["id"] == 5).unwrap()["error"];
    assert_eq!(error["code"], -32601);
}
