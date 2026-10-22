//! Signature help, document and workspace symbols, inlay hints and code
//! actions through the wire, over the examples, the conformance host
//! and files written for the test.

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

/// Every message the server writes for `messages`, after an
/// `initialize` offering UTF-8, with the conformance host and
/// `options` as the initialization options' other keys.
fn exchange_with(options: &Value, messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    let mut initialization =
        json!({ "host": root().join("tests/fixtures/host") });
    if let (Some(into), Some(from)) =
        (initialization.as_object_mut(), options.as_object())
    {
        for (key, value) in from {
            into.insert(key.clone(), value.clone());
        }
    }
    let initialize = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "capabilities": {"general": {"positionEncodings": ["utf-8"]}},
            "initializationOptions": initialization,
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

fn exchange(messages: &[Value]) -> Vec<Value> {
    exchange_with(&json!({}), messages)
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

fn result(replies: &[Value], id: u64) -> &Value {
    &replies
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))["result"]
}

/// The line and character after `needle`'s first occurrence.
fn after(text: &str, needle: &str) -> (u32, u32) {
    let at = text.find(needle).expect(needle) + needle.len();
    let line = text[..at].matches('\n').count();
    let character = at - text[..at].rfind('\n').map_or(0, |i| i + 1);
    (
        u32::try_from(line).unwrap(),
        u32::try_from(character).unwrap(),
    )
}

/// A scratch file `name` with `text`, under the test's target dir.
fn scratch(name: &str, text: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("features");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.canonicalize().unwrap()
}

#[test]
fn signature_help_names_a_commands_spots_and_the_active_one() {
    let path = scratch("signature.fipe", "use fs\nfs.list \nls ./ -l \n");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, &text),
        at(
            1,
            "textDocument/signatureHelp",
            &uri,
            after(&text, "fs.list "),
        ),
        at(
            2,
            "textDocument/signatureHelp",
            &uri,
            after(&text, "ls ./ -l "),
        ),
        at(3, "textDocument/signatureHelp", &uri, (0, 3)),
    ]);
    let help = result(&replies, 1);
    let signature = &help["signatures"][0];
    assert!(
        signature["label"]
            .as_str()
            .unwrap()
            .starts_with("fs.list (dir: Rd<fs>) (path: Src)"),
        "{signature}"
    );
    assert_eq!(signature["parameters"][0]["label"], "(dir: Rd<fs>)");
    assert_eq!(help["activeParameter"], 0);
    let help = result(&replies, 2);
    let label = help["signatures"][0]["label"].as_str().unwrap();
    assert!(label.starts_with("ls (path: Src)"), "{label}");
    assert_eq!(help["activeParameter"], 2, "{help}");
    assert_eq!(result(&replies, 3), &Value::Null);
}

#[test]
fn document_symbols_list_bindings_declarations_and_types_with_members() {
    let text = "use fs\n\
        type Point { x: F64, y: F64 }\n\
        type Shape { Dot, Line(a: Point, b: Point) }\n\
        origin = Point 0.0 0.0\n\
        double = n => n * 2\n\
        area: Shape -> F64\n";
    let path = scratch("symbols.fipe", text);
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, text),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "textDocument/documentSymbol",
            "params": {"textDocument": {"uri": uri}},
        }),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "workspace/symbol",
            "params": {"query": "pnt"},
        }),
    ]);
    let symbols = result(&replies, 1).as_array().unwrap();
    let names: Vec<(&str, u64)> = symbols
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s["kind"].as_u64().unwrap()))
        .collect();
    // Struct 23, Enum 10, Variable 13, Function 12.
    assert_eq!(
        names,
        [
            ("Point", 23),
            ("Shape", 10),
            ("origin", 13),
            ("double", 12),
            ("area", 12)
        ]
    );
    assert_eq!(symbols[0]["children"][1]["name"], "y");
    assert_eq!(symbols[0]["children"][1]["kind"], 8);
    assert_eq!(symbols[1]["children"][1]["name"], "Line");
    assert_eq!(symbols[1]["children"][1]["detail"], "2 fields");
    assert_eq!(symbols[0]["selectionRange"]["start"]["line"], 1);
    assert!(
        symbols[3]["detail"].as_str().unwrap().contains("->"),
        "{}",
        symbols[3]
    );
    let found = result(&replies, 2).as_array().unwrap();
    let names: Vec<&str> =
        found.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Point"]);
}

#[test]
fn workspace_symbols_reach_the_file_modules_beside_an_open_document() {
    let examples = root().join("Documentation/examples");
    let path = examples.join("05-modules.fipe");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, &text),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "workspace/symbol",
            "params": {"query": "polar"},
        }),
    ]);
    let found = result(&replies, 1).as_array().unwrap();
    let in_geometry = found.iter().any(|s| {
        s["location"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("geometry.fipe")
    });
    assert!(in_geometry, "{found:?}");
}

fn published(replies: &[Value], uri: &str) -> Vec<Value> {
    replies
        .iter()
        .rfind(|m| {
            m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == uri
        })
        .map(|m| m["params"]["diagnostics"].as_array().unwrap().clone())
        .unwrap_or_default()
}

#[test]
fn inlay_hints_show_an_unannotated_bindings_scheme_unless_turned_off() {
    let text = "double = n => n * 2\nwidth: Integer = 80\nname = \"x\"\n";
    let path = scratch("hints.fipe", text);
    let uri = uri(&path);
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {"uri": uri},
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 3, "character": 0}},
        },
    });
    let replies = exchange(&[open(&uri, text), request.clone()]);
    let hints = result(&replies, 1).as_array().unwrap();
    let shown: Vec<(u64, u64, &str)> = hints
        .iter()
        .map(|h| {
            (
                h["position"]["line"].as_u64().unwrap(),
                h["position"]["character"].as_u64().unwrap(),
                h["label"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert_eq!((shown[0].0, shown[0].1), (0, 6));
    assert!(shown[0].2.contains("->"), "{shown:?}");
    assert_eq!(shown[1], (2, 4, ": String"));
    let replies = exchange_with(
        &json!({ "inlayHints": false }),
        &[open(&uri, text), request],
    );
    let initialized =
        &replies[0]["result"]["capabilities"]["inlayHintProvider"];
    assert_eq!(initialized, &json!(false));
    assert_eq!(result(&replies, 1), &Value::Null);
}

fn code_action(
    id: u64,
    uri: &str,
    range: ((u32, u32), (u32, u32)),
    diagnostics: &[Value],
) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/codeAction",
        "params": {
            "textDocument": {"uri": uri},
            "range": {"start": {"line": range.0 .0, "character": range.0 .1},
                      "end": {"line": range.1 .0, "character": range.1 .1}},
            "context": {"diagnostics": diagnostics},
        },
    })
}

/// An action: its title, kind and edits as (start line, start char,
/// end line, end char, text).
type Found = (String, String, Vec<(u64, u64, u64, u64, String)>);

fn actions(replies: &[Value], id: u64, uri: &str) -> Vec<Found> {
    result(replies, id)
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            let edits = action["edit"]["changes"][uri]
                .as_array()
                .unwrap()
                .iter()
                .map(|edit| {
                    let r = &edit["range"];
                    (
                        r["start"]["line"].as_u64().unwrap(),
                        r["start"]["character"].as_u64().unwrap(),
                        r["end"]["line"].as_u64().unwrap(),
                        r["end"]["character"].as_u64().unwrap(),
                        edit["newText"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            (
                action["title"].as_str().unwrap().to_string(),
                action["kind"].as_str().unwrap().to_string(),
                edits,
            )
        })
        .collect()
}

#[test]
fn a_missing_use_is_offered_from_a_host_module_and_a_sibling_file() {
    scratch(
        "shapes.fipe",
        "pub twice: Integer -> Integer = n => n * 2\n",
    );
    let text = "use fs\nx = wrap 10 \"a\"\ny = twice 2\n";
    let path = scratch("missing.fipe", text);
    let uri = uri(&path);
    let opened = open(&uri, text);
    let replies = exchange(std::slice::from_ref(&opened));
    let diagnostics = published(&replies, &uri);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let replies = exchange(&[
        opened,
        code_action(1, &uri, ((1, 0), (2, 20)), &diagnostics),
    ]);
    let found = actions(&replies, 1, &uri);
    let titles: Vec<&str> =
        found.iter().map(|(title, ..)| title.as_str()).collect();
    assert_eq!(titles, ["Import `text.wrap`", "Import `shapes.twice`"]);
    assert_eq!(found[0].1, "quickfix");
    assert_eq!(found[0].2, [(1, 0, 1, 0, String::from("use text.wrap\n"))]);
    // Applied, the document checks and a signature can be written.
    let fixed = "use fs\nuse text.wrap\nuse shapes.twice\nx = wrap 10 \"a\"\ny = twice 2\n";
    let replies = exchange(&[
        open(&uri, fixed),
        code_action(2, &uri, ((4, 0), (4, 1)), &[]),
    ]);
    assert!(
        published(&replies, &uri).is_empty(),
        "{:?}",
        published(&replies, &uri)
    );
    let found = actions(&replies, 2, &uri);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "Annotate `y: Integer`");
    assert_eq!(found[0].1, "refactor.rewrite");
    assert_eq!(found[0].2, [(4, 1, 4, 1, String::from(": Integer"))]);
}

#[test]
fn the_crates_suggestions_become_quick_fixes() {
    // A call whose argument reads both as a tuple and as a list.
    let text = "g = x (y = 0) => x\ng (1, 2)\n";
    let path = scratch("tuple.fipe", text);
    let uri = uri(&path);
    let replies = exchange(&[
        open(&uri, text),
        code_action(1, &uri, ((1, 0), (1, 8)), &[]),
    ]);
    let found = actions(&replies, 1, &uri);
    let doubled: Vec<_> = found
        .iter()
        .filter(|(title, ..)| title.contains("double"))
        .collect();
    assert_eq!(doubled.len(), 1, "{found:?}");
    assert_eq!(
        doubled[0].2,
        [
            (1, 2, 1, 3, String::from("((")),
            (1, 7, 1, 8, String::from("))"))
        ]
    );
    // An outcome into a text place: pipe its good side or payload.
    let text = "use text\nm: Maybe<String> = Just \"a\"\nm | text.trim\n";
    let path = scratch("outcome.fipe", text);
    let uri = fipe_lsp::uri::of(&path).as_str().to_string();
    let opened = open(&uri, text);
    let replies = exchange(std::slice::from_ref(&opened));
    let diagnostics = published(&replies, &uri);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("cannot enter a text place"),
        "{diagnostics:?}"
    );
    let replies = exchange(&[
        opened,
        code_action(1, &uri, ((2, 0), (2, 13)), &diagnostics),
    ]);
    let found = actions(&replies, 1, &uri);
    let titles: Vec<&str> =
        found.iter().map(|(title, ..)| title.as_str()).collect();
    assert_eq!(
        titles,
        ["Pipe the good side with `||`", "Pipe the payload with `|:`"],
        "{diagnostics:?}"
    );
    assert_eq!(found[1].2, [(2, 2, 2, 3, String::from("|:"))]);
}
