//! Explicit metadata selection is offline, atomic and scoped to initialization.
use fipe_lsp::{
    loader::{self, Host, Interface, InterfaceProblem},
    serve, transport, Options,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
};

fn directory(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("lsp-interfaces-{name}"));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}
fn input(module: &str, path: &Path) -> Interface {
    Interface {
        module: module.to_owned(),
        path: path.to_owned(),
    }
}
fn uri(path: &Path) -> String {
    fipe_lsp::uri::of(path).as_str().to_owned()
}
fn initialize(id: u64, options: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{
        "capabilities":{"general":{"positionEncodings":["utf-8"]}},"initializationOptions":options}})
}
fn open(path: &Path, text: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri(path),"languageId":"fipe","version":1,"text":text}}})
}
fn at(id: u64, method: &str, path: &Path, line: u32, character: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":{"textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}}})
}
fn response(messages: &[Value], id: u64) -> &Value {
    messages.iter().find(|m| m["id"] == id).unwrap()
}
fn exchange(messages: &[Value], options: &Options, writer: &mut impl Write) {
    let mut framed = Vec::new();
    for message in messages {
        transport::write(&mut framed, message).unwrap();
    }
    transport::write(
        &mut framed,
        &json!({"jsonrpc":"2.0","id":999,"method":"shutdown"}),
    )
    .unwrap();
    transport::write(&mut framed, &json!({"jsonrpc":"2.0","method":"exit"}))
        .unwrap();
    assert!(serve(&mut Cursor::new(framed), writer, options).unwrap());
}
fn decode(bytes: Vec<u8>) -> Vec<Value> {
    let mut cursor = Cursor::new(bytes);
    let mut messages = Vec::new();
    while let Some(message) = transport::read(&mut cursor).unwrap() {
        messages.push(message);
    }
    messages
}
fn run(messages: &[Value]) -> Vec<Value> {
    let mut bytes = Vec::new();
    exchange(messages, &Options::default(), &mut bytes);
    decode(bytes)
}
fn clean(messages: &[Value]) {
    let diagnostics: Vec<_> = messages
        .iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics")
        .collect();
    assert!(!diagnostics.is_empty(), "{messages:?}");
    assert!(
        diagnostics
            .iter()
            .all(|m| m["params"]["diagnostics"] == json!([])),
        "{messages:?}"
    );
}

#[test]
fn offline_metadata_drives_editor_actions_and_preserves_exact_sources() {
    let dir = directory("actions");
    let path = dir.join("arithmetic.fipi");
    let text="## 한글 metadata.\r\npub base = 32\r\npub width: Integer\r\npub f: (n: Integer = width) -> Integer\r\n";
    fs::write(&path, text).unwrap();
    let host = Host::default()
        .with_interfaces(&[input("demo.math", &path)])
        .unwrap();
    assert_eq!(host.source("demo.math"), Some((path.as_path(), text)));
    let client = dir.join("client.fipe");
    let messages = [
        initialize(
            0,
            &json!({"interfaces":[{"module":"demo.math","path":path}]}),
        ),
        open(&client, "use demo.math\n(math.base, math.f)"),
        at(1, "textDocument/hover", &client, 1, 7),
        at(2, "textDocument/definition", &client, 1, 7),
        at(4, "textDocument/hover", &client, 1, 17),
        open(&dir.join("completion.fipe"), "use demo.math\nmath."),
        at(
            3,
            "textDocument/completion",
            &dir.join("completion.fipe"),
            1,
            5,
        ),
    ];
    let replies = run(&messages);
    assert_eq!(
        replies
            .iter()
            .find(|m| m["method"] == "textDocument/publishDiagnostics")
            .unwrap()["params"]["diagnostics"],
        json!([]),
        "{replies:?}"
    );
    assert!(response(&replies, 1)["result"]
        .to_string()
        .contains("pub base = 32"));
    assert!(response(&replies, 4)["result"]
        .to_string()
        .contains("Integer"));
    let definition = &response(&replies, 2)["result"][0];
    assert_eq!(definition["uri"], uri(&path));
    assert_eq!(
        definition["range"],
        json!({"start":{"line":1,"character":4},"end":{"line":1,"character":8}})
    );
    assert!(response(&replies, 3)["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["label"] == "base"));
    // Inert provider/build/generator names cannot enter this text-only path.
    for name in ["provider.so", "build.rs", "generator"] {
        fs::write(dir.join(name), "not executable metadata").unwrap();
    }
    assert_eq!(run(&messages), replies);
}

#[test]
fn selected_names_exclude_invalid_siblings_before_parsing() {
    let dir = directory("precedence");
    for (module, source) in [
        ("foo", "use foo\nfoo.answer"),
        ("demo.math", "use demo.math\nmath.answer"),
        ("demo.math", "use demo\ndemo.math.answer"),
    ] {
        let path = dir.join("different-name.fipi");
        fs::write(&path, "pub answer: Integer").unwrap();
        fs::write(
            dir.join(format!("{}.fipe", module.split('.').next().unwrap())),
            "= invalid (",
        )
        .unwrap();
        clean(&run(&[
            initialize(
                0,
                &json!({"interfaces":[{"module":module,"path":path}]}),
            ),
            open(&dir.join("client.fipe"), source),
        ]));
    }
}

#[test]
fn duplicate_and_failed_batches_preserve_the_entire_prior_host() {
    let dir = directory("atomic");
    fs::write(dir.join("foo.fipe"), "pub old: Integer").unwrap();
    let host = loader::host(Some(&dir));
    let before = host.declarations.clone();
    let sources = host.sources.clone();
    let first = dir.join("first.fipi");
    let second = dir.join("second.fipi");
    fs::write(&first, "pub answer: Integer").unwrap();
    fs::write(&second, "pub answer: Integer").unwrap();
    for inputs in [
        vec![input("foo", &first)],
        vec![input("new", &first), input("new", &second)],
        vec![input("new", &first), input("new", &first)],
    ] {
        let error = host.with_interfaces(&inputs).unwrap_err();
        let InterfaceProblem::Duplicate { first: prior } = &error.reason else {
            panic!("{error}")
        };
        assert!(error.to_string().contains(&prior.display().to_string()));
        assert!(error
            .to_string()
            .contains(&error.input.path.display().to_string()));
        assert_eq!(host.declarations, before);
        assert_eq!(host.sources, sources);
    }
    let missing = dir.join("missing.fipi");
    assert!(host
        .with_interfaces(&[input("new", &first), input("missing", &missing)])
        .is_err());
    fs::write(&second, "pub broken: (").unwrap();
    assert!(host
        .with_interfaces(&[input("new", &first), input("broken", &second)])
        .is_err());
    assert_eq!(host.declarations, before);
    assert_eq!(host.sources, sources);
    assert!(host.declarations.get_module("new").is_none());
    let nested = host
        .with_interfaces(&[input("demo.math", &first), input("demo", &first)])
        .unwrap();
    assert_eq!(nested.sources.len(), 3);
}

#[test]
fn invalid_selection_stays_waiting_until_corrected_without_fallback() {
    let dir = directory("failures");
    let valid = dir.join("ok.fipi");
    fs::write(&valid, "pub answer: Integer").unwrap();
    let syntax = dir.join("syntax.fipi");
    fs::write(&syntax, "## é\r\npub broken: (").unwrap();
    let binary = dir.join("binary.fipi");
    fs::write(&binary, [0xff]).unwrap();
    fs::write(dir.join("foo.fipe"), "pub answer = 42").unwrap();
    let client = dir.join("client.fipe");
    for selected in [
        Value::Null,
        json!({}),
        json!([{}]),
        json!([{"module":"foo","path":4}]),
        json!([{"module":"foo","path":"relative.fipi"}]),
        json!([{"module":"commands","path":valid}]),
        json!([{"module":"BAD","path":valid}]),
        json!([{"module":"foo","path":dir.join("missing.fipi")}]),
        json!([{"module":"foo","path":binary}]),
        json!([{"module":"foo","path":syntax}]),
        json!([{"module":"foo","path":dir.join("foo.fipe")}]),
    ] {
        let replies = run(&[
            initialize(0, &json!({"interfaces":selected})),
            open(&client, "1 + 2"),
            at(1, "textDocument/hover", &client, 0, 0),
            initialize(
                2,
                &json!({"interfaces":[{"module":"foo","path":valid}]}),
            ),
            open(&client, "use foo\nfoo.answer"),
        ]);
        assert_eq!(
            response(&replies, 0)["error"]["code"],
            -32602,
            "{replies:?}"
        );
        assert_eq!(response(&replies, 1)["error"]["code"], -32002);
        assert!(response(&replies, 2).get("result").is_some());
        clean(&replies);
        assert_eq!(
            replies
                .iter()
                .filter(|m| m["method"] == "textDocument/publishDiagnostics")
                .count(),
            1
        );
    }
    let error = Host::default()
        .with_interfaces(&[input("foo", &syntax)])
        .unwrap_err();
    assert!(error.to_string().contains(&syntax.display().to_string()));
    assert!(error.to_string().contains("2:"), "{error}");
    let InterfaceProblem::Declarations { text, errors } = error.reason else {
        panic!("declaration error")
    };
    assert_eq!(text, "## é\r\npub broken: (");
    assert!(!errors.is_empty());
    assert!(errors[0].span.start >= 7);
    for options in [json!({}), json!({"interfaces":[]})] {
        clean(&run(&[initialize(0, &options), open(&client, "1 + 2")]));
    }
}

#[test]
fn legacy_state_survives_failed_host_override_and_client_default_errors_stay_local(
) {
    let dir = directory("legacy");
    fs::write(dir.join("base.fipe"), "pub old: Integer").unwrap();
    let override_dir = dir.join("override");
    fs::create_dir(&override_dir).unwrap();
    fs::write(override_dir.join("new.fipe"), "pub other: Integer").unwrap();
    let mut output = Vec::new();
    let client = dir.join("client.fipe");
    exchange(
        &[
            initialize(
                0,
                &json!({"host":override_dir,"interfaces":[{"module":"foo","path":dir.join("missing.fipi")}]}),
            ),
            initialize(1, &json!({})),
            open(&client, "use base\nbase.old"),
        ],
        &Options {
            host: Some(dir.clone()),
        },
        &mut output,
    );
    let replies = decode(output);
    assert_eq!(response(&replies, 0)["error"]["code"], -32602);
    clean(&replies);
    let path = dir.join("default.fipi");
    fs::write(&path, "pub f: (n: Integer = True) -> Integer").unwrap();
    let replies = run(&[
        initialize(0, &json!({"interfaces":[{"module":"config","path":path}]})),
        open(&client, "use config\nconfig.f"),
    ]);
    let diagnostics = &replies
        .iter()
        .find(|m| m["method"] == "textDocument/publishDiagnostics")
        .unwrap()["params"];
    assert_eq!(diagnostics["uri"], uri(&client));
    assert_eq!(diagnostics["diagnostics"][0]["range"]["start"]["line"], 1);
}

enum DiskEdit {
    Write(PathBuf, Vec<u8>),
    Remove(PathBuf),
    #[cfg(unix)]
    Relink(PathBuf, PathBuf),
}

struct ScriptedOutput {
    bytes: Vec<u8>,
    edits: Vec<(u64, Vec<DiskEdit>)>,
}
impl Write for ScriptedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        if let Ok(message) = serde_json::from_slice::<Value>(bytes) {
            if let Some(index) =
                self.edits.iter().position(|(id, _)| message["id"] == *id)
            {
                for edit in self.edits.remove(index).1 {
                    match edit {
                        DiskEdit::Write(path, bytes) => fs::write(path, bytes)?,
                        DiskEdit::Remove(path) => fs::remove_file(path)?,
                        #[cfg(unix)]
                        DiskEdit::Relink(path, target) => {
                            fs::remove_file(&path)?;
                            std::os::unix::fs::symlink(target, path)?;
                        }
                    }
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn scripted(
    messages: &[Value],
    edits: Vec<(u64, Vec<DiskEdit>)>,
    options: &Options,
) -> Vec<Value> {
    let mut writer = ScriptedOutput {
        bytes: Vec::new(),
        edits,
    };
    exchange(messages, options, &mut writer);
    assert!(writer.edits.is_empty());
    decode(writer.bytes)
}
fn changed(path: &Path, text: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri(path),"version":2},"contentChanges":[{"text":text}]}})
}
fn close(path: &Path) -> Value {
    json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri(path)}}})
}
fn watched(uri: &str, kind: u32) -> Value {
    json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":uri,"type":kind}]}})
}
fn diagnostics_before<'a>(
    messages: &'a [Value],
    id: u64,
    path: &Path,
) -> &'a Value {
    let end = messages.iter().position(|m| m["id"] == id).unwrap();
    &messages[..end]
        .iter()
        .rev()
        .find(|m| {
            m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == uri(path)
        })
        .unwrap()["params"]["diagnostics"]
}
fn no_location(messages: &[Value], id: u64) {
    let value = &response(messages, id)["result"];
    assert!(value.is_null() || value == &json!([]), "{id}: {value}");
}
fn stale(messages: &[Value], id: u64, client: &Path) {
    let diagnostics = diagnostics_before(messages, id, client);
    assert!(
        diagnostics.as_array().unwrap().iter().any(|d| d["message"]
            .as_str()
            .unwrap()
            .contains("using last accepted interface metadata")),
        "{diagnostics}"
    );
}

#[test]
fn watched_batches_commit_together_and_navigation_never_uses_unseen_disk_text()
{
    let dir = directory("reload-batch");
    let first = dir.join("first.fipi");
    let second = dir.join("second.fipi");
    fs::write(&first, "pub item: Integer").unwrap();
    fs::write(&second, "pub item: Integer").unwrap();
    fs::write(dir.join("demo.fipe"), "= malformed (").unwrap();
    let legacy_dir = dir.join("legacy");
    fs::create_dir(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("legacy.fipe");
    fs::write(&legacy_path, "pub value: Integer").unwrap();
    let client = dir.join("client.fipe");
    let legacy = dir.join("legacy-client.fipe");
    let replies = scripted(
        &[
            initialize(
                0,
                &json!({"interfaces":[{"module":"demo.math","path":first},{"module":"other","path":second}]}),
            ),
            open(&client, "use demo.math\nuse other\n(math.item, other.item)"),
            open(&legacy, "use legacy\nlegacy.value"),
            at(10, "textDocument/definition", &client, 2, 7),
            watched(&uri(&first), 2),
            at(1, "textDocument/hover", &client, 2, 7),
            at(2, "textDocument/definition", &client, 2, 7),
            at(6, "textDocument/definition", &legacy, 1, 8),
            at(3, "textDocument/definition", &client, 2, 19),
            watched(&uri(&second), 2),
            at(4, "textDocument/hover", &client, 2, 7),
            at(5, "textDocument/definition", &client, 2, 7),
            at(7, "textDocument/hover", &legacy, 1, 8),
        ],
        vec![
            (
                0,
                vec![
                    DiskEdit::Write(
                        first.clone(),
                        b"## updated\npub item: String".to_vec(),
                    ),
                    DiskEdit::Write(second.clone(), b"pub item: (".to_vec()),
                    DiskEdit::Write(
                        legacy_path,
                        b"pub replacement: String".to_vec(),
                    ),
                ],
            ),
            (
                3,
                vec![DiskEdit::Write(
                    second.clone(),
                    b"pub item: String".to_vec(),
                )],
            ),
        ],
        &Options {
            host: Some(legacy_dir),
        },
    );
    no_location(&replies, 10);
    stale(&replies, 1, &client);
    assert!(response(&replies, 1)["result"]
        .to_string()
        .contains("Integer"));
    no_location(&replies, 2);
    no_location(&replies, 3);
    assert!(!response(&replies, 6)["result"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!diagnostics_before(&replies, 1, &second)
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(diagnostics_before(&replies, 4, &client), &json!([]));
    assert_eq!(diagnostics_before(&replies, 4, &second), &json!([]));
    assert!(response(&replies, 4)["result"]
        .to_string()
        .contains("String"));
    assert_eq!(
        response(&replies, 5)["result"][0]["range"]["start"]["line"],
        1
    );
    assert!(response(&replies, 7)["result"]
        .to_string()
        .contains("Integer"));
}

#[test]
fn unsaved_interfaces_override_disk_and_recover_after_invalid_edits_and_deletion(
) {
    for utf8 in [true, false] {
        let dir = directory(if utf8 {
            "buffers-utf8"
        } else {
            "buffers-utf16"
        });
        let path = dir.join("api.fipi");
        fs::write(&path, "pub item: Integer").unwrap();
        let client = dir.join("client.fipe");
        let mut init = initialize(
            0,
            &json!({"interfaces":[{"module":"api","path":path}]}),
        );
        if !utf8 {
            init["params"]["capabilities"]["general"]["positionEncodings"] =
                json!([]);
        }
        let invalid = "## Ω\r\npub item: (name: String = \"🚀\") -> (";
        let incremental = json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri(&path),"version":3},"contentChanges":[{"range":{"start":{"line":1,"character":10},"end":{"line":1,"character":13}},"text":"Integer"}]}});
        let replies = scripted(
            &[
                init,
                open(&client, "use api\napi.item"),
                open(&path, "## Ω\r\npub item: String\r\n"),
                at(1, "textDocument/hover", &client, 1, 5),
                watched(&uri(&path), 3),
                at(2, "textDocument/definition", &client, 1, 5),
                at(20, "textDocument/hover", &path, 1, 5),
                changed(&path, "## Ω\r\npub item: F64\r\n"),
                at(3, "textDocument/hover", &client, 1, 5),
                incremental,
                at(4, "textDocument/hover", &client, 1, 5),
                changed(&path, invalid),
                at(5, "textDocument/definition", &client, 1, 5),
                changed(&path, "## Ω\r\npub item: String\r\n"),
                at(6, "textDocument/definition", &client, 1, 5),
                close(&path),
                at(7, "textDocument/definition", &client, 1, 5),
                watched(&uri(&path), 1),
                at(8, "textDocument/hover", &client, 1, 5),
                at(9, "textDocument/definition", &client, 1, 5),
            ],
            vec![
                (1, vec![DiskEdit::Remove(path.clone())]),
                (
                    7,
                    vec![DiskEdit::Write(
                        path.clone(),
                        b"pub item: F64".to_vec(),
                    )],
                ),
            ],
            &Options::default(),
        );
        assert!(response(&replies, 1)["result"]
            .to_string()
            .contains("String"));
        assert_eq!(response(&replies, 2)["result"][0]["uri"], uri(&path));
        assert_eq!(
            response(&replies, 2)["result"][0]["range"]["start"]["line"],
            1
        );
        assert!(response(&replies, 20)["result"].is_null());
        assert!(response(&replies, 3)["result"].to_string().contains("F64"));
        assert!(response(&replies, 4)["result"]
            .to_string()
            .contains("Integer"));
        no_location(&replies, 5);
        stale(&replies, 5, &client);
        let errors = diagnostics_before(&replies, 5, &path);
        assert!(!errors.as_array().unwrap().is_empty());
        let line = invalid.lines().last().unwrap();
        let expected = if utf8 {
            line.len()
        } else {
            line.encode_utf16().count()
        };
        assert_eq!(
            errors[0]["range"]["start"]["character"], expected,
            "{errors}"
        );
        assert_eq!(diagnostics_before(&replies, 6, &client), &json!([]));
        assert_eq!(
            response(&replies, 6)["result"][0]["range"]["start"]["line"],
            1
        );
        no_location(&replies, 7);
        stale(&replies, 7, &client);
        assert_eq!(diagnostics_before(&replies, 8, &client), &json!([]));
        assert!(response(&replies, 8)["result"].to_string().contains("F64"));
        assert_eq!(
            response(&replies, 9)["result"][0]["range"]["start"]["line"],
            0
        );
    }
}

#[test]
fn invalid_bytes_deleted_paths_and_unselected_events_never_discover_or_fall_back(
) {
    let dir = directory("reload-errors");
    let path = dir.join("api.fipi");
    fs::write(&path, "pub item: Integer").unwrap();
    fs::write(dir.join("api.fipe"), "pub item = 42").unwrap();
    let unselected = dir.join("extra.fipi");
    fs::write(&unselected, "pub new: Integer").unwrap();
    let client = dir.join("client.fipe");
    let encoded = uri(&path).replace("api.fipi", "%61pi.fipi");
    let replies = scripted(
        &[
            initialize(
                0,
                &json!({"interfaces":[{"module":"api","path":path}]}),
            ),
            open(&client, "use api\napi.item"),
            watched(&uri(&path), 2),
            at(1, "textDocument/definition", &client, 1, 5),
            watched(&uri(&path), 3),
            at(2, "textDocument/definition", &client, 1, 5),
            watched(&uri(&unselected), 1),
            json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[null,{}, {"uri":4}, {"uri":uri(&path)}, {"uri":uri(&path),"type":"bad"}, {"uri":uri(&path),"type":4}]}}),
            at(3, "textDocument/definition", &client, 1, 5),
            watched(&encoded, 1),
            at(4, "textDocument/definition", &client, 1, 5),
            open(&dir.join("other.fipe"), "use extra\nextra.new"),
            at(5, "textDocument/hover", &client, 1, 5),
        ],
        vec![
            (0, vec![DiskEdit::Write(path.clone(), vec![0xff])]),
            (1, vec![DiskEdit::Remove(path.clone())]),
            (
                2,
                vec![DiskEdit::Write(
                    path.clone(),
                    b"pub item: String".to_vec(),
                )],
            ),
        ],
        &Options::default(),
    );
    for id in [1, 2, 3] {
        no_location(&replies, id);
        stale(&replies, id, &client);
    }
    assert_eq!(
        diagnostics_before(&replies, 1, &path)[0]["range"],
        json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})
    );
    assert_eq!(diagnostics_before(&replies, 4, &client), &json!([]));
    assert!(!response(&replies, 4)["result"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!diagnostics_before(&replies, 5, &dir.join("other.fipe"))
        .as_array()
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[test]
fn original_canonical_aliases_share_selection_and_retargeting_is_explicit() {
    let dir = directory("aliases");
    let target = dir.join("target-a.fipi");
    let alternate = dir.join("target-b.fipi");
    let path = dir.join("selected.fipi");
    fs::write(&target, "pub item: Integer").unwrap();
    fs::write(&alternate, "pub item: F64").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let client = dir.join("client.fipe");
    let canonical = uri(&target).replace("target-a", "target-%61");
    let replies = scripted(
        &[
            initialize(
                0,
                &json!({"interfaces":[{"module":"demo.math","path":path},{"module":"demo.other","path":path}]}),
            ),
            open(&client, "use demo.math\n(math.item, demo.other.item)"),
            open(&path, "pub item: Integer"),
            open(&target, "pub item: String"),
            at(1, "textDocument/definition", &client, 1, 7),
            close(&target),
            at(2, "textDocument/definition", &client, 1, 7),
            changed(&path, "pub item: String"),
            open(&target, "pub item: String"),
            at(3, "textDocument/hover", &client, 1, 23),
            watched(&canonical, 3),
            at(4, "textDocument/definition", &client, 1, 7),
            watched(&uri(&path), 2),
            at(5, "textDocument/definition", &client, 1, 7),
        ],
        vec![
            (3, vec![DiskEdit::Remove(target.clone())]),
            (4, vec![DiskEdit::Relink(path.clone(), alternate)]),
        ],
        &Options::default(),
    );
    no_location(&replies, 1);
    stale(&replies, 1, &client);
    let error = diagnostics_before(&replies, 1, &path)[0]["message"]
        .as_str()
        .unwrap();
    assert!(
        error.contains(&uri(&path)) && error.contains(&uri(&target)),
        "{error}"
    );
    assert_eq!(response(&replies, 2)["result"][0]["uri"], uri(&path));
    assert_eq!(diagnostics_before(&replies, 3, &client), &json!([]));
    assert!(response(&replies, 3)["result"]
        .to_string()
        .contains("String"));
    assert_eq!(response(&replies, 4)["result"][0]["uri"], uri(&path));
    no_location(&replies, 5);
    stale(&replies, 5, &client);
    assert!(diagnostics_before(&replies, 5, &path)[0]["message"]
        .as_str()
        .unwrap()
        .contains("selected target changed"));
}

#[test]
fn reloaded_exports_and_docs_reach_completion_without_interface_script_features(
) {
    let dir = directory("reload-completion");
    let path = dir.join("api.fipi");
    let client = dir.join("client.fipe");
    fs::write(&path, "pub old: Integer").unwrap();
    let updated = "## Fresh after reload.\r\npub fresh: String\r\n";
    let replies = scripted(
        &[
            initialize(
                0,
                &json!({"interfaces":[{"module":"api","path":path}]}),
            ),
            open(&client, "use api\napi."),
            at(1, "textDocument/completion", &client, 1, 4),
            watched(&uri(&path), 2),
            at(2, "textDocument/completion", &client, 1, 4),
            changed(&client, "use api\napi.fresh"),
            at(3, "textDocument/hover", &client, 1, 6),
            at(4, "textDocument/definition", &client, 1, 6),
            open(&path, updated),
            json!({"jsonrpc":"2.0","id":5,"method":"textDocument/formatting","params":{"textDocument":{"uri":uri(&path)},"options":{"tabSize":4,"insertSpaces":true}}}),
        ],
        vec![(
            0,
            vec![DiskEdit::Write(path.clone(), updated.as_bytes().to_vec())],
        )],
        &Options::default(),
    );
    for (id, expected, absent) in [(1, "old", "fresh"), (2, "fresh", "old")] {
        let items = response(&replies, id)["result"].as_array().unwrap();
        assert!(items.iter().any(|item| item["label"] == expected));
        assert!(!items.iter().any(|item| item["label"] == absent));
    }
    assert!(response(&replies, 3)["result"]
        .to_string()
        .contains("Fresh after reload."));
    assert_eq!(
        response(&replies, 4)["result"][0]["range"]["start"]["line"],
        1
    );
    assert_eq!(response(&replies, 5)["error"]["code"], -32803);
    assert!(response(&replies, 5)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("interface"));
    assert_eq!(diagnostics_before(&replies, 5, &path), &json!([]));
}

const RESOURCE_TOOL_HEADER: &str = "## 한글 offline resource.\r\npub type resource Table\r\npub type Packet\r\npub acquire: () -> Table\r\n";
const RESOURCE_TOOL_CLIENT: &str = "use api\nx: api.Table = b\"x\"";

fn resource_tool_expected(host: &Host) -> Vec<(Value, String)> {
    let module =
        fipe::parse::parse(fipe::lex::lex(RESOURCE_TOOL_CLIENT)).unwrap();
    fipe::check::check(
        &module,
        &host.declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap_err()
    .iter()
    .map(|error| {
        let fipe_lsp::words::Placed::Here(span, message) =
            fipe_lsp::words::check(error)
        else {
            panic!("client-owned resource diagnostic: {error:?}")
        };
        (
            serde_json::to_value(fipe_lsp::position::range(
                RESOURCE_TOOL_CLIENT,
                span,
                fipe_lsp::position::Encoding::Utf8,
            ))
            .unwrap(),
            message,
        )
    })
    .collect()
}

fn resource_tool_diagnostic(
    messages: &[Value],
    id: u64,
    client: &Path,
    expected: &[(Value, String)],
) {
    let actual = diagnostics_before(messages, id, client).as_array().unwrap();
    assert!(!expected.is_empty());
    for (range, message) in expected {
        assert!(
            actual.iter().any(|d| &d["range"] == range
                && d["message"].as_str() == Some(message.as_str())),
            "{id}: {actual:?}"
        );
    }
}

#[test]
fn resource_tool_selected_declarations_and_codec_classification_are_real() {
    let dir = directory("resource-tool-classification");
    let path = dir.join("api.fipi");
    fs::write(&path, RESOURCE_TOOL_HEADER).unwrap();
    let host = Host::default()
        .with_interfaces(&[input("api", &path)])
        .unwrap();
    assert_eq!(
        host.source("api"),
        Some((path.as_path(), RESOURCE_TOOL_HEADER))
    );
    let namespace = host.declarations.get_module("api").unwrap();
    for (name, marked) in [("Table", true), ("Packet", false)] {
        let declared =
            namespace.types.iter().find(|ty| ty.name == name).unwrap();
        assert!(
            matches!(declared.kind, fipe::decl::Kind::Type { resource, .. } if resource == marked)
        );
    }
    let module =
        fipe::parse::parse(fipe::lex::lex("use api\napi.acquire ()")).unwrap();
    let unit = fipe::check::check(
        &module,
        &host.declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    assert!(unit
        .tables()
        .meanings
        .iter()
        .any(|m| m.label == "Table" && m.resource));
    assert!(!resource_tool_expected(&host).is_empty());
    let ordinary =
        fipe::parse::parse(fipe::lex::lex("use api\nx: api.Packet = b\"x\""))
            .unwrap();
    let ordinary_unit = fipe::check::check(
        &ordinary,
        &host.declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    assert!(ordinary_unit.tables().nodes.iter().any(|node| matches!(
        node.kind,
        fipe::ir::Kind::Cast {
            row: fipe::ir::CastRow::Codec,
            ..
        }
    )));
}

#[test]
fn resource_tool_reload_retains_marked_semantics_without_provider_fallback() {
    let dir = directory("resource-tool-reload");
    let path = dir.join("api.fipi");
    fs::write(&path, RESOURCE_TOOL_HEADER).unwrap();
    fs::write(dir.join("api.fipe"), "= malformed provider fallback (").unwrap();
    let client = dir.join("client.fipe");
    let navigation = dir.join("navigation.fipe");
    let ordinary = dir.join("ordinary.fipe");
    let host = Host::default()
        .with_interfaces(&[input("api", &path)])
        .unwrap();
    let expected = resource_tool_expected(&host);
    let edited = RESOURCE_TOOL_HEADER
        .replace("offline resource", "edited offline resource")
        + "pub ready: Integer\n";
    let replies = scripted(
        &[
            initialize(
                0,
                &json!({"interfaces":[{"module":"api","path":path}]}),
            ),
            open(&client, RESOURCE_TOOL_CLIENT),
            open(&ordinary, "use api\nx: api.Packet = b\"x\""),
            open(&navigation, "use api\napi.acquire ()"),
            at(1, "textDocument/hover", &navigation, 1, 5),
            open(&path, &edited),
            open(&dir.join("completion.fipe"), "use api\napi."),
            at(
                20,
                "textDocument/completion",
                &dir.join("completion.fipe"),
                1,
                4,
            ),
            at(2, "textDocument/definition", &navigation, 1, 5),
            changed(&path, "pub type resource Table\npub acquire: ("),
            at(3, "textDocument/definition", &navigation, 1, 5),
            changed(&path, RESOURCE_TOOL_HEADER),
            at(4, "textDocument/definition", &navigation, 1, 5),
            close(&path),
            at(5, "textDocument/definition", &navigation, 1, 5),
            watched(&uri(&path), 2),
            at(6, "textDocument/definition", &navigation, 1, 5),
            watched(&uri(&path), 3),
            at(7, "textDocument/definition", &navigation, 1, 5),
        ],
        vec![
            (
                5,
                vec![DiskEdit::Write(path.clone(), edited.as_bytes().to_vec())],
            ),
            (6, vec![DiskEdit::Remove(path.clone())]),
        ],
        &Options::default(),
    );
    for id in 1..=7 {
        resource_tool_diagnostic(&replies, id, &client, &expected);
    }
    for id in [1, 2, 4, 5, 6] {
        assert_eq!(diagnostics_before(&replies, id, &ordinary), &json!([]));
        if id != 1 {
            assert_eq!(response(&replies, id)["result"][0]["uri"], uri(&path));
            assert_eq!(
                response(&replies, id)["result"][0]["range"],
                json!({"start":{"line":3,"character":4},"end":{"line":3,"character":11}})
            );
        }
    }
    for id in [3, 7] {
        no_location(&replies, id);
        stale(&replies, id, &navigation);
        assert!(!diagnostics_before(&replies, id, &path)
            .as_array()
            .unwrap()
            .is_empty());
    }
    assert!(response(&replies, 20)["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["label"] == "ready"));
    assert!(response(&replies, 1)["result"]
        .to_string()
        .contains("Table"));
}
