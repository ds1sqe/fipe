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
    run_options(input, &Options::default())
}

fn run_options(input: Vec<u8>, options: &Options) -> (bool, Vec<Value>) {
    let mut output = Vec::new();
    let clean = serve(&mut Cursor::new(input), &mut output, options).unwrap();
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

#[test]
fn invalid_host_options_leave_initialization_waiting_and_unmodified() {
    for host in [
        json!(null),
        json!(false),
        json!(1),
        json!([]),
        json!({}),
        json!(""),
    ] {
        let bad = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"initializationOptions":{"host":host,"inlayHints":false},"capabilities":{"general":{"positionEncodings":["utf-8"]}}}});
        let good =
            json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{}});
        let (_, replies) = run(framed(&[
            bad,
            request(2, "workspace/symbol"),
            good,
            shutdown_and_exit()[0].clone(),
            shutdown_and_exit()[1].clone(),
        ]));
        assert_eq!(code(&replies, 1), -32602);
        assert_eq!(code(&replies, 2), -32002);
        let capabilities =
            &replies.iter().find(|reply| reply["id"] == 3).unwrap()["result"]
                ["capabilities"];
        assert_eq!(capabilities["positionEncoding"], "utf-16");
        assert_eq!(capabilities["inlayHintProvider"], true);
    }
}

#[test]
fn valid_override_replaces_bad_options_and_base_errors_precede_interfaces() {
    let missing = root().join("missing-selected-host");
    let mut bad = initialize();
    bad["id"] = json!(1);
    bad["params"]["initializationOptions"]["host"] = json!(missing);
    bad["params"]["initializationOptions"]["interfaces"] =
        json!([{"module":"missing","path":"/missing.fipi"}]);
    let mut good = initialize();
    good["id"] = json!(2);
    let document = uri(&root().join("host-retry.fipe"));
    let (_, replies) = run_options(
        framed(&[
            bad,
            good,
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":document,"languageId":"fipe","version":1,"text":"use config\nconfig."}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{"textDocument":{"uri":document},"position":{"line":1,"character":7}}}),
            shutdown_and_exit()[0].clone(),
            shutdown_and_exit()[1].clone(),
        ]),
        &Options {
            host: Some(missing.clone()),
        },
    );
    let error = replies.iter().find(|reply| reply["id"] == 1).unwrap();
    assert_eq!(error["error"]["code"], -32602);
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(missing.to_str().unwrap())
            && message.contains("cannot enumerate"),
        "{message}"
    );
    assert!(!message.contains("interface"), "{message}");
    assert!(replies
        .iter()
        .find(|reply| reply["id"] == 2)
        .unwrap()
        .get("result")
        .is_some());
    let completion =
        &replies.iter().find(|reply| reply["id"] == 3).unwrap()["result"];
    assert!(
        completion
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "max_port"),
        "{completion}"
    );
}

// File-backed output cannot block the child while its input is being rejected.
struct HeaderChild(std::process::Child);
impl Drop for HeaderChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn header_process(
    input: Vec<u8>,
    case: usize,
) -> (std::process::ExitStatus, Vec<u8>, String) {
    use std::{
        fs,
        io::Write as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("lsp-headers-{}-{case}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let stdout = directory.join("stdout");
    let stderr = directory.join("stderr");
    let mut child = HeaderChild(
        Command::new(env!("CARGO_BIN_EXE_fipe-lsp"))
            .stdin(Stdio::piped())
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let mut stdin = child.0.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let _ = writer.join().unwrap();
            panic!(
                "header child exceeded 10 seconds; output at {}",
                directory.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Reap explicitly even though try_wait has already collected termination.
    assert_eq!(child.0.wait().unwrap(), status);
    if let Err(error) = writer.join().unwrap() {
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionReset
        ));
    }
    assert!(fs::metadata(&stdout).unwrap().len() <= 65536);
    assert!(fs::metadata(&stderr).unwrap().len() <= 65536);
    (
        status,
        fs::read(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap(),
    )
}

#[test]
fn header_limits_end_the_real_server_before_later_requests() {
    let healthy = framed(&[
        initialize(),
        shutdown_and_exit()[0].clone(),
        shutdown_and_exit()[1].clone(),
    ]);
    let (status, output, error) = header_process(healthy, 0);
    assert_eq!(status.code(), Some(0));
    assert!(error.is_empty());
    let mut read = Cursor::new(output);
    let mut ids = Vec::new();
    while let Some(message) = transport::read(&mut read).unwrap() {
        if let Some(id) = message["id"].as_u64() {
            ids.push(id);
        }
    }
    assert_eq!(ids, [0, 999]);
    let mut line = vec![b'x'; transport::MAX_HEADER_LINE];
    line.push(b'\n');
    let many = b"X: x\n".repeat(transport::MAX_HEADER_BYTES / 5 + 1);
    for (case, mut input) in [line, many].into_iter().enumerate() {
        input.extend(framed(&[initialize()]));
        let (status, output, error) = header_process(input, case + 1);
        assert_eq!(status.code(), Some(1));
        assert!(output.is_empty(), "later initialize must not execute");
        assert_eq!(
            error,
            if case == 0 {
                "fipe-lsp: header line exceeds 8192 bytes\n"
            } else {
                "fipe-lsp: headers exceed 32768 bytes\n"
            }
        );
    }
}
