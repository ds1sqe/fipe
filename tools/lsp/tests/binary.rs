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
