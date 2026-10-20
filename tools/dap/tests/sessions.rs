//! Scripted sessions: requests in, responses and events out, through
//! the session the binary runs, and once through the binary over stdio.
use fipe_dap::{
    protocol,
    session::{Control, Session},
};
use serde_json::{json, Value};
use std::{
    io::{BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn substituted(text: &str) -> String {
    text.replace("${ROOT}", root().to_str().unwrap())
}

/// The scripted session `tests/sessions/<name>.json`.
fn script(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/sessions")
        .join(format!("{name}.json"));
    serde_json::from_str(&substituted(&std::fs::read_to_string(path).unwrap()))
        .unwrap()
}

/// Every message the session sends for `requests`, in order.
fn drive(requests: &[Value]) -> Vec<Value> {
    let mut session = Session::default();
    let mut sent = Vec::new();
    for request in requests {
        let control = session.handle(request);
        session.settle();
        sent.extend(session.out.take());
        if control == Control::Exit {
            break;
        }
    }
    sent
}

fn events(sent: &[Value]) -> Vec<&Value> {
    sent.iter().filter(|m| m["type"] == "event").collect()
}

/// The `call` lines of an example's trace fixture.
fn calls(example: &str) -> Vec<String> {
    let path = root().join(format!("tests/fixtures/traces/{example}.txt"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("call "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn launch(arguments: Value) -> Vec<Value> {
    let mut launch =
        json!({ "seq": 2, "type": "request", "command": "launch" });
    launch["arguments"] = arguments;
    drive(&[
        json!({ "seq": 1, "type": "request", "command": "initialize" }),
        launch,
        json!({ "seq": 3, "type": "request", "command": "configurationDone" }),
    ])
}

#[test]
fn an_example_under_its_trace_runs_to_terminated_with_its_calls() {
    let script = script("02-outcomes");
    let sent = drive(script["requests"].as_array().unwrap());
    for message in sent.iter().filter(|m| m["type"] == "response") {
        assert_eq!(message["success"], true, "{message}");
    }
    let events = events(&sent);
    let names: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(names.first(), Some(&"initialized"));
    assert_eq!(names[names.len() - 2..], ["exited", "terminated"]);
    let outputs: Vec<&str> = events
        .iter()
        .filter(|e| e["event"] == "output")
        .map(|e| e["body"]["output"].as_str().unwrap())
        .collect();
    assert_eq!(outputs, calls("02-outcomes"));
    let threads = sent
        .iter()
        .find(|m| m["command"] == "threads")
        .expect("a threads response");
    assert_eq!(threads["body"]["threads"][0]["id"], 1);
}

#[test]
fn every_example_with_a_trace_replays_its_calls() {
    for example in ["01-files", "04-text", "05-modules"] {
        let root = root();
        let sent = launch(json!({
            "program": root.join(format!("Documentation/examples/{example}.fipe")),
            "host": root.join("tests/fixtures/host"),
            "trace": root.join(format!("tests/fixtures/traces/{example}.txt")),
        }));
        let outputs: Vec<&str> = events(&sent)
            .iter()
            .filter(|e| e["event"] == "output")
            .map(|e| e["body"]["output"].as_str().unwrap())
            .collect();
        assert_eq!(outputs, calls(example), "{example}");
        assert_eq!(events(&sent).last().unwrap()["event"], "terminated");
    }
}

#[test]
fn a_fault_stops_with_reason_exception_at_its_location() {
    let root = root();
    // Without a trace every host call fails, so the first one Faults.
    let sent = launch(json!({
        "program": root.join("Documentation/examples/02-outcomes.fipe"),
        "host": root.join("tests/fixtures/host"),
    }));
    let stopped = events(&sent)
        .into_iter()
        .find(|e| e["event"] == "stopped")
        .expect("a stopped event");
    assert_eq!(stopped["body"]["reason"], "exception");
    assert_eq!(stopped["body"]["threadId"], 1);
    let description = stopped["body"]["description"].as_str().unwrap();
    assert!(
        description.starts_with("fault in main at ")
            && description.contains("no recorded call"),
        "{description}"
    );
}

#[test]
fn a_budget_too_small_stops_as_an_exception() {
    let root = root();
    let sent = launch(json!({
        "program": root.join("Documentation/examples/02-outcomes.fipe"),
        "host": root.join("tests/fixtures/host"),
        "trace": root.join("tests/fixtures/traces/02-outcomes.txt"),
        "budget": 5,
    }));
    let stopped = events(&sent)
        .into_iter()
        .find(|e| e["event"] == "stopped")
        .expect("a stopped event");
    assert_eq!(stopped["body"]["reason"], "exception");
    assert!(
        stopped["body"]["text"].as_str().unwrap().contains("budget"),
        "{stopped}"
    );
}

#[test]
fn a_program_that_does_not_check_fails_its_launch() {
    let dir =
        std::env::temp_dir().join(format!("fipe-dap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("bad.fipe");
    std::fs::write(&program, "x = 1 +\n").unwrap();
    let sent = launch(json!({ "program": program }));
    let response = sent
        .iter()
        .find(|m| m["command"] == "launch")
        .expect("a launch response");
    assert_eq!(response["success"], false);
    assert!(
        response["message"]
            .as_str()
            .unwrap()
            .contains("does not check"),
        "{response}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unknown_request_is_refused() {
    let sent =
        drive(&[json!({ "seq": 1, "type": "request", "command": "goto" })]);
    assert_eq!(sent[0]["success"], false);
    assert_eq!(sent[0]["message"], "unsupported: goto");
}

#[test]
fn the_binary_speaks_the_protocol_over_stdio() {
    let script = script("02-outcomes");
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-dap"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in script["requests"].as_array().unwrap() {
        protocol::write(&mut stdin, request).unwrap();
    }
    stdin.flush().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut sent = Vec::new();
    while let Some(message) = protocol::read(&mut output).unwrap() {
        sent.push(message);
    }
    assert!(child.wait().unwrap().success());
    // The binary reads requests while a program runs, so a request may
    // be answered between two of the run's events: compare the messages
    // without their order and sequence numbers.
    let unordered = |messages: Vec<Value>| {
        let mut texts: Vec<String> = messages
            .into_iter()
            .map(|mut message| {
                message.as_object_mut().unwrap().remove("seq");
                message.to_string()
            })
            .collect();
        texts.sort();
        texts
    };
    assert_eq!(
        unordered(sent),
        unordered(drive(script["requests"].as_array().unwrap()))
    );
}
