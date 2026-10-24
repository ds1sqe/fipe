//! The `fipe-lsp` program over real stdio.

use std::io::{BufReader, Write};
use std::process::{Command, Stdio};

use fipe_lsp::transport;
use serde_json::json;

#[test]
fn the_program_answers_initialize_and_exits_on_exit() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"capabilities": {}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ];
    for message in &messages {
        transport::write(&mut stdin, message).unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let first = transport::read(&mut stdout).unwrap().unwrap();
    assert_eq!(first["id"], 1);
    assert_eq!(first["result"]["serverInfo"]["name"], "fipe-lsp");
    let second = transport::read(&mut stdout).unwrap().unwrap();
    assert_eq!(second["id"], 2);
    assert!(child.wait().unwrap().success());
}

#[test]
fn the_program_exits_with_failure_on_exit_without_shutdown() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ];
    for message in &messages {
        transport::write(&mut stdin, message).unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    assert_eq!(child.wait().unwrap().code(), Some(1));
}

#[test]
fn command_discovery_finishes_before_reading_protocol_input() {
    for (args, code, version) in [
        (vec!["--version"], 0, true),
        (vec!["--help"], 0, false),
        (vec!["-h"], 0, false),
        (vec!["--version", "extra"], 1, false),
        (vec!["--help", "extra"], 1, false),
        (vec!["--unknown"], 1, false),
        (vec!["--host"], 1, false),
    ] {
        let output = discovery(&args, true);
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        if version {
            assert_eq!(
                output.stdout,
                format!("fipe-lsp {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
            );
            assert!(output.stderr.is_empty());
        } else if code == 0 {
            assert!(output.stdout.starts_with(b"usage: fipe-lsp"));
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(output.stderr.starts_with(b"usage: fipe-lsp"));
        }
    }
}

fn discovery(args: &[&str], keep_open: bool) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-lsp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _input = if keep_open {
        child.stdin.take()
    } else {
        drop(child.stdin.take());
        None
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("discovery waited for protocol input: {args:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn no_arguments_and_eof_still_require_shutdown() {
    let output = discovery(&[], false);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn the_program_reports_bad_host_and_accepts_a_corrected_initialize() {
    let host = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/host");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("lsp-binary-host-options");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("file");
    std::fs::write(&file, "not a directory").unwrap();
    let invalid = dir.join("invalid");
    std::fs::create_dir(&invalid).unwrap();
    std::fs::write(invalid.join("bad.fipe"), [255]).unwrap();
    for (path, reason) in [
        (dir.join("missing"), "cannot enumerate"),
        (file, "cannot enumerate"),
        (invalid, "cannot read"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-lsp"))
            .arg("--host")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for message in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"initializationOptions":{"host":host}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"shutdown"}),
            json!({"jsonrpc":"2.0","method":"exit"}),
        ] {
            transport::write(&mut input, &message).unwrap();
        }
        drop(input);
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let error = transport::read(&mut output).unwrap().unwrap();
        assert_eq!(error["error"]["code"], -32602);
        let message = error["error"]["message"].as_str().unwrap();
        assert!(
            message.contains(reason)
                && message.contains(path.to_str().unwrap()),
            "{message}"
        );
        let success = transport::read(&mut output).unwrap().unwrap();
        assert_eq!(success["id"], 2);
        assert!(success.get("result").is_some());
        assert_eq!(transport::read(&mut output).unwrap().unwrap()["id"], 3);
        assert!(child.wait().unwrap().success());
    }
}
