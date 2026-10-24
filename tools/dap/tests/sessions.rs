//! Scripted sessions: requests in, responses and events out, through
//! the session the binary runs, and once through the binary over stdio.
use fipe_dap::{
    protocol::{self, Incoming},
    session::{Control, Session},
};
use serde_json::{json, Value};
use std::{
    io::{BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

mod common;

use common::{expected, root};

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

/// The text of the output events of `category`: `console` for the
/// trace's lines, `stdout` and `stderr` for the program's output.
fn outputs<'a>(sent: &'a [Value], category: &str) -> Vec<&'a str> {
    events(sent)
        .into_iter()
        .filter(|e| e["event"] == "output" && e["body"]["category"] == category)
        .map(|e| e["body"]["output"].as_str().unwrap())
        .collect()
}

/// A session's outputs against its example's trace fixture.
fn assert_observes(sent: &[Value], example: &str) {
    let expected = expected(example);
    assert_eq!(outputs(sent, "console"), expected.console, "{example}");
    assert_eq!(
        outputs(sent, "stdout").concat(),
        expected.success,
        "{example}"
    );
    assert_eq!(
        outputs(sent, "stderr").concat(),
        expected.error,
        "{example}"
    );
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
    assert_observes(&sent, "02-outcomes");
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
        assert_observes(&sent, example);
        assert_eq!(events(&sent).last().unwrap()["event"], "terminated");
    }
}

#[test]
fn a_fault_stops_with_reason_exception_at_its_location() {
    // A trace that records no call: the first host call Faults.
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let root = root();
    let sent = launch(json!({
        "program": root.join("Documentation/examples/02-outcomes.fipe"),
        "host": root.join("tests/fixtures/host"),
        "trace": fixtures.join("fault.txt"),
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
fn a_launch_without_a_trace_runs_live_on_the_std_host() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("live");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("live.fipe");
    std::fs::write(
        &program,
        "use cli\n\"first\"\n\"written\\n\" | put ./live.txt\ncli.args | join \" \"\n\
         s => Err (s | text.trim)\n",
    )
    .unwrap();
    let sent = launch(json!({
        "program": program,
        "cwd": dir,
        "args": ["a", "b"],
        "input": " in \n",
    }));
    for message in sent.iter().filter(|m| m["type"] == "response") {
        assert_eq!(message["success"], true, "{message}");
    }
    assert_eq!(outputs(&sent, "stdout").concat(), "first\na b\n");
    assert_eq!(outputs(&sent, "stderr").concat(), "in\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("live.txt")).unwrap(),
        "written\n"
    );
    let exited = events(&sent)
        .into_iter()
        .find(|e| e["event"] == "exited")
        .expect("an exited event");
    assert_eq!(exited["body"]["exitCode"], 1);
    let console = outputs(&sent, "console");
    assert!(console[0].starts_with("call commands.read"), "{console:?}");
    assert_eq!(console.last().copied(), Some("value Err \"in\"\n"));
    // A write outside the roots is refused; input the program cannot
    // take fails the launch before any effect.
    std::fs::write(&program, "\"x\" | put ../escape.txt\n").unwrap();
    let sent = launch(json!({ "program": program, "cwd": dir }));
    let stopped = events(&sent)
        .into_iter()
        .find(|e| e["event"] == "stopped")
        .expect("a stopped event");
    assert_eq!(stopped["body"]["reason"], "exception");
    assert!(
        stopped["body"]["description"]
            .as_str()
            .unwrap()
            .ends_with("host: ../escape.txt is outside the write roots"),
        "{stopped}"
    );
    assert!(!dir.join("../escape.txt").exists());
    std::fs::write(&program, "n => n + 1\n").unwrap();
    let sent = launch(json!({ "program": program, "cwd": dir, "input": "x" }));
    let response = sent
        .iter()
        .find(|m| m["command"] == "launch")
        .expect("a launch response");
    assert_eq!(response["success"], false);
    assert!(
        response["message"]
            .as_str()
            .unwrap()
            .contains("host input enters only a String or Vec<U8> spot"),
        "{response}"
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
    while let Some(incoming) = protocol::read(&mut output).unwrap() {
        let Incoming::Message(message) = incoming else {
            panic!("the adapter sent a malformed frame: {incoming:?}")
        };
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

#[test]
fn every_scripted_session_ends_as_its_trace_fixture_says() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/sessions");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
        .collect();
    names.sort();
    assert!(names.len() >= 6, "{names:?}");
    for name in names {
        let script = script(&name);
        let example = script["example"].as_str().unwrap();
        let sent = drive(script["requests"].as_array().unwrap());
        for message in sent.iter().filter(|m| m["type"] == "response") {
            assert_eq!(message["success"], true, "{name}: {message}");
        }
        assert_observes(&sent, example);
        let stopped =
            events(&sent).into_iter().find(|e| e["event"] == "stopped");
        if script["ends"] == "exception" {
            let stopped = stopped.expect("a stopped event");
            assert_eq!(stopped["body"]["reason"], "exception", "{name}");
            let description = format!(
                "{}\n",
                stopped["body"]["description"].as_str().unwrap()
            );
            assert_eq!(
                &description,
                expected(example).console.last().unwrap(),
                "{name}"
            );
        } else {
            assert!(stopped.is_none(), "{name}: {stopped:?}");
            assert_eq!(events(&sent).last().unwrap()["event"], "terminated");
        }
    }
}

/// The messages the binary sends for raw `input`, until it exits.
fn over_stdio(input: &[u8]) -> (Vec<Value>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-dap"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input).unwrap();
    drop(stdin);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut sent = Vec::new();
    while let Some(Incoming::Message(message)) =
        protocol::read(&mut output).unwrap()
    {
        sent.push(message);
    }
    (sent, child.wait().unwrap().success())
}

fn framed(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn malformed_frames_are_reported_and_the_session_goes_on() {
    let initialize = r#"{"seq":1,"type":"request","command":"initialize"}"#;
    let disconnect = r#"{"seq":2,"type":"request","command":"disconnect"}"#;
    let mut input = framed("{not json");
    input.extend(b"Content-Type: application/json\r\n\r\n");
    input.extend(framed(initialize));
    input.extend(framed(disconnect));
    let (sent, clean) = over_stdio(&input);
    assert!(clean);
    let important = outputs(&sent, "important");
    assert_eq!(important.len(), 2, "{sent:?}");
    assert!(important[0].contains("not JSON"), "{important:?}");
    assert!(important[1].contains("Content-Length"), "{important:?}");
    let commands: Vec<&Value> = sent
        .iter()
        .filter(|m| m["type"] == "response")
        .map(|m| &m["command"])
        .collect();
    assert_eq!(commands, ["initialize", "disconnect"]);
}

#[test]
fn a_length_too_long_to_read_is_reported_not_allocated() {
    let mut input =
        framed(r#"{"seq":1,"type":"request","command":"initialize"}"#);
    input.extend(b"Content-Length: 18446744073709551615\r\n\r\n{}");
    let (sent, clean) = over_stdio(&input);
    assert!(clean);
    assert_eq!(sent[0]["command"], "initialize");
    let important = outputs(&sent, "important");
    assert!(
        important.len() == 1 && important[0].contains("longer than"),
        "{sent:?}"
    );
}

#[test]
fn a_message_that_is_not_a_request_is_answered() {
    let sent = drive(&[json!([1, 2]), json!({ "seq": 7, "type": "request" })]);
    assert!(outputs(&sent, "important")[0].contains("request object"));
    let response = sent.iter().find(|m| m["type"] == "response").unwrap();
    assert_eq!(response["request_seq"], 7);
    assert_eq!(response["success"], false);
    assert_eq!(response["message"], "a request needs a command");
}

#[test]
fn continuing_after_a_fault_ends_the_run() {
    let root = root();
    let mut requests = vec![
        json!({ "seq": 1, "type": "request", "command": "initialize" }),
        json!({ "seq": 2, "type": "request", "command": "launch", "arguments": {
            "program": root.join("tools/dap/tests/fixtures/fault.fipe"),
            "host": root.join("tests/fixtures/host"),
            "trace": root.join("tools/dap/tests/fixtures/fault.txt"),
        }}),
        json!({ "seq": 3, "type": "request", "command": "configurationDone" }),
    ];
    requests.push(json!({ "seq": 4, "type": "request", "command": "continue",
        "arguments": { "threadId": 1 } }));
    let sent = drive(&requests);
    let resumed = sent.iter().find(|m| m["command"] == "continue").unwrap();
    assert_eq!(resumed["success"], true, "{resumed}");
    let names: Vec<&str> = events(&sent)
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        names[names.len() - 2..],
        ["exited", "terminated"],
        "{names:?}"
    );
    let exited = events(&sent)
        .into_iter()
        .find(|e| e["event"] == "exited")
        .unwrap();
    assert_eq!(exited["body"]["exitCode"], 1);
}

#[test]
fn a_client_counting_from_zero_gets_its_own_numbers() {
    let root = root();
    let program = root.join("Documentation/examples/02-outcomes.fipe");
    let sent = drive(&[
        json!({ "seq": 1, "type": "request", "command": "initialize",
            "arguments": { "linesStartAt1": false, "columnsStartAt1": false } }),
        json!({ "seq": 2, "type": "request", "command": "launch", "arguments": {
            "program": program,
            "host": root.join("tests/fixtures/host"),
            "trace": root.join("tests/fixtures/traces/02-outcomes.txt"),
        }}),
        // The `|:` at 1-based 7:20.
        json!({ "seq": 3, "type": "request", "command": "setBreakpoints",
            "arguments": { "source": { "path": program },
                "breakpoints": [{ "line": 6, "column": 19 }] } }),
        json!({ "seq": 4, "type": "request", "command": "breakpointLocations",
            "arguments": { "source": { "path": program }, "line": 6 } }),
        json!({ "seq": 5, "type": "request", "command": "configurationDone" }),
        json!({ "seq": 6, "type": "request", "command": "stackTrace",
            "arguments": { "threadId": 1 } }),
    ]);
    let answer = |command: &str| {
        sent.iter().find(|m| m["command"] == command).unwrap()["body"].clone()
    };
    let breakpoint = &answer("setBreakpoints")["breakpoints"][0];
    assert_eq!(breakpoint["verified"], true, "{breakpoint}");
    assert_eq!(
        (&breakpoint["line"], &breakpoint["column"]),
        (&json!(6), &json!(19))
    );
    let locations = answer("breakpointLocations");
    assert!(
        locations["breakpoints"]
            .as_array()
            .unwrap()
            .contains(&json!({ "line": 6, "column": 19 })),
        "{locations}"
    );
    let top = &answer("stackTrace")["stackFrames"][0];
    assert_eq!(
        (&top["line"], &top["column"]),
        (&json!(6), &json!(19)),
        "{top}"
    );
}

#[test]
fn a_frame_that_does_not_exist_is_refused() {
    let root = root();
    let program = root.join("Documentation/examples/02-outcomes.fipe");
    let sent = drive(&[
        json!({ "seq": 1, "type": "request", "command": "initialize" }),
        json!({ "seq": 2, "type": "request", "command": "launch", "arguments": {
            "program": program,
            "host": root.join("tests/fixtures/host"),
            "trace": root.join("tests/fixtures/traces/02-outcomes.txt"),
            "stopOnEntry": true,
        }}),
        json!({ "seq": 3, "type": "request", "command": "configurationDone" }),
        json!({ "seq": 4, "type": "request", "command": "scopes",
            "arguments": { "frameId": 424_242 } }),
        json!({ "seq": 5, "type": "request", "command": "evaluate",
            "arguments": { "expression": "x", "frameId": 424_242 } }),
        json!({ "seq": 6, "type": "request", "command": "scopes",
            "arguments": { "frameId": 0 } }),
    ]);
    let response =
        |seq: u64| sent.iter().find(|m| m["request_seq"] == seq).unwrap();
    for seq in [4, 5] {
        assert_eq!(response(seq)["success"], false);
        assert_eq!(response(seq)["message"], "no frame 424242");
    }
    assert_eq!(response(6)["success"], true);
}

#[test]
fn a_budget_that_is_not_a_natural_number_fails_the_launch() {
    let root = root();
    for budget in [json!(-5), json!("x"), json!(1.5)] {
        let sent = launch(json!({
            "program": root.join("Documentation/examples/02-outcomes.fipe"),
            "host": root.join("tests/fixtures/host"),
            "budget": budget,
        }));
        let response = sent.iter().find(|m| m["command"] == "launch").unwrap();
        assert_eq!(response["success"], false, "{budget}");
        assert!(
            response["message"]
                .as_str()
                .unwrap()
                .contains("natural number"),
            "{response}"
        );
    }
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
        (vec!["file.fipe"], 1, false),
    ] {
        let output = discovery(&args, true);
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        if version {
            assert_eq!(
                output.stdout,
                format!("fipe-dap {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
            );
            assert!(output.stderr.is_empty());
        } else if code == 0 {
            assert!(output.stdout.starts_with(b"usage: fipe-dap"));
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(output.stderr.starts_with(b"usage: fipe-dap"));
        }
    }
}

fn discovery(args: &[&str], keep_open: bool) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fipe-dap"))
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
fn no_arguments_and_eof_still_exit_cleanly() {
    let output = discovery(&[], false);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
