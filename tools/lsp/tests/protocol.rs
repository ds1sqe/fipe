//! The base protocol and the lifecycle: framing faults, malformed
//! messages, requests out of order, document synchronization and the
//! diagnostics of documents that use a changed one.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use fipe_lsp::{serve, transport, Options};
use serde_json::{json, Value};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn uri(path: &Path) -> String {
    fipe_lsp::uri::of(path).as_str().to_string()
}

fn framed(messages: &[Value]) -> Vec<u8> {
    let mut input = Vec::new();
    for message in messages {
        transport::write(&mut input, message).unwrap();
    }
    input
}

/// Serve raw `input`; whether the server was shut down, and every
/// message it wrote.
fn run(input: Vec<u8>) -> (bool, Vec<Value>) {
    let mut output = Vec::new();
    let clean =
        serve(&mut Cursor::new(input), &mut output, &Options::default())
            .unwrap();
    let mut read = Cursor::new(output);
    let mut replies = Vec::new();
    while let Some(message) = transport::read(&mut read).unwrap() {
        replies.push(message);
    }
    (clean, replies)
}

fn request(id: u64, method: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {}})
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "capabilities": {"general": {"positionEncodings": ["utf-8"]}},
            "initializationOptions": {
                "host": root().join("tests/fixtures/host"),
            },
        },
    })
}

fn shutdown_and_exit() -> [Value; 2] {
    [
        request(999, "shutdown"),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ]
}

fn code(replies: &[Value], id: u64) -> i64 {
    replies
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))["error"]
        ["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("no error for {id}: {replies:?}"))
}

#[test]
fn a_body_that_is_not_json_is_a_parse_error_and_the_server_goes_on() {
    let mut input = framed(&[initialize()]);
    input.extend_from_slice(b"Content-Length: 5\r\n\r\n{bad}");
    input.extend(framed(&[request(1, "shutdown")]));
    input.extend(framed(&[json!({"jsonrpc": "2.0", "method": "exit"})]));
    let (clean, replies) = run(input);
    assert!(clean, "{replies:?}");
    let parse = replies
        .iter()
        .find(|message| message["id"].is_null())
        .unwrap_or_else(|| panic!("{replies:?}"));
    assert_eq!(parse["error"]["code"], -32700);
    assert!(replies.iter().any(|message| message["id"] == 1));
}

#[test]
fn a_body_too_long_is_dropped_unread_and_the_server_goes_on() {
    let mut input = framed(&[initialize()]);
    let length = transport::MAX_LENGTH + 1;
    input.extend_from_slice(
        format!("Content-Length: {length}\r\n\r\n").as_bytes(),
    );
    input.resize(input.len() + length, b' ');
    input.extend(framed(&shutdown_and_exit()));
    let (clean, replies) = run(input);
    assert!(clean, "{replies:?}");
    assert_eq!(replies[1]["error"]["code"], -32700, "{replies:?}");
    assert!(replies.iter().any(|message| message["id"] == 999));
}

#[test]
fn a_length_that_cannot_be_held_ends_the_input_without_a_panic() {
    let mut input = framed(&[initialize()]);
    input.extend_from_slice(b"Content-Length: 18446744073709551615\r\n\r\n{}");
    let (clean, replies) = run(input);
    assert!(!clean);
    assert_eq!(replies.len(), 1, "{replies:?}");
}

#[test]
fn requests_out_of_the_lifecycle_are_refused() {
    let (clean, replies) = run(framed(&[
        request(1, "textDocument/hover"),
        initialize(),
        request(2, "initialize"),
        request(3, "shutdown"),
        request(4, "textDocument/hover"),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ]));
    assert!(clean);
    assert_eq!(code(&replies, 1), -32002);
    assert_eq!(code(&replies, 2), -32600);
    assert!(replies
        .iter()
        .any(|m| m["id"] == 3 && m["result"].is_null()));
    assert_eq!(code(&replies, 4), -32600);
}

#[test]
fn exit_without_shutdown_is_not_a_clean_end() {
    let (clean, _) = run(framed(&[
        initialize(),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ]));
    assert!(!clean);
}

#[test]
fn a_document_uri_that_does_not_parse_publishes_nothing() {
    let mut messages = vec![
        initialize(),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {"textDocument": {
                "uri": "not a uri", "languageId": "fipe", "version": 1,
                "text": "x = 1\n",
            }},
        }),
    ];
    messages.extend(shutdown_and_exit());
    let (_, replies) = run(framed(&messages));
    assert!(replies.iter().all(Value::is_object), "{replies:?}");
    assert_eq!(replies.len(), 2, "{replies:?}");
}

fn open(uri: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": uri, "languageId": "fipe", "version": 1, "text": text,
        }},
    })
}

fn diagnostics_of<'a>(replies: &'a [Value], uri: &str) -> Vec<&'a Value> {
    replies
        .iter()
        .filter(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
        })
        .map(|message| &message["params"]["diagnostics"])
        .collect()
}

#[test]
fn a_ranged_change_edits_the_range_only() {
    let path = root().join("scratch/ranged.fipe");
    let uri = uri(&path);
    let mut messages = vec![
        initialize(),
        open(&uri, "a = 1\nb = a\n"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{
                    "range": {
                        "start": {"line": 1, "character": 4},
                        "end": {"line": 1, "character": 5},
                    },
                    "text": "zz",
                }],
            },
        }),
    ];
    messages.extend(shutdown_and_exit());
    let (_, replies) = run(framed(&messages));
    let published = diagnostics_of(&replies, &uri);
    assert_eq!(published[0], &json!([]), "{replies:?}");
    // `b = zz`: an unknown name on line 1, not a one-line document.
    let second = published[1].as_array().unwrap();
    assert_eq!(second.len(), 1, "{second:?}");
    assert_eq!(
        second[0]["range"]["start"],
        json!({"line": 1, "character": 4})
    );
}

#[test]
fn a_change_to_a_used_file_republishes_the_documents_that_use_it() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("stale");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("lib.fipe");
    let main = dir.join("main.fipe");
    std::fs::write(&lib, "pub inc = n => n + 1\n").unwrap();
    std::fs::write(&main, "use lib.inc\ny = inc 2\n").unwrap();
    let (lib_uri, main_uri) = (uri(&lib), uri(&main));
    let mut messages = vec![
        initialize(),
        open(&main_uri, "use lib.inc\ny = inc 2\n"),
        open(&lib_uri, "pub inc = n => n + 1\n"),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": lib_uri, "version": 2},
                "contentChanges": [{"text": "pub dec = n => n - 1\n"}],
            },
        }),
    ];
    messages.extend(shutdown_and_exit());
    let (_, replies) = run(framed(&messages));
    let main_published = diagnostics_of(&replies, &main_uri);
    assert!(main_published.len() >= 2, "{replies:?}");
    assert_eq!(main_published[0], &json!([]));
    assert!(
        !main_published
            .last()
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty(),
        "{replies:?}"
    );
}
