//! Breakpoints, stepping and inspection, through the session the
//! binary runs.
use fipe_dap::session::{Control, Session};
use serde_json::{json, Value};
mod common;

use common::root;
use std::path::PathBuf;

/// A session with its requests numbered as they are sent.
struct Client {
    session: Session,
    seq: u64,
    sent: Vec<Value>,
}

impl Client {
    fn new() -> Self {
        Self {
            session: Session::default(),
            seq: 0,
            sent: Vec::new(),
        }
    }

    /// Send a request without letting the program run after it.
    fn send(&mut self, command: &str, arguments: Value) -> Value {
        self.seq += 1;
        let mut request = json!({
            "seq": self.seq, "type": "request", "command": command,
        });
        request["arguments"] = arguments;
        let control = self.session.handle(&request);
        let messages = self.session.out.take();
        let response = messages
            .iter()
            .find(|m| m["type"] == "response" && m["request_seq"] == self.seq)
            .cloned()
            .expect("a response");
        self.sent.extend(messages);
        assert_eq!(control, Control::Continue);
        response
    }

    /// Send a request, then let the program run until it stops.
    fn request(&mut self, command: &str, arguments: Value) -> Value {
        let response = self.send(command, arguments);
        self.session.settle();
        self.sent.extend(self.session.out.take());
        response
    }

    /// The body of the last event named `name`.
    fn last(&self, name: &str) -> Value {
        self.sent
            .iter()
            .rev()
            .find(|m| m["type"] == "event" && m["event"] == name)
            .map_or_else(|| panic!("no {name} event"), |m| m["body"].clone())
    }

    fn launch(&mut self, arguments: Value) {
        self.request("initialize", json!({ "adapterID": "fipe" }));
        let response = self.request("launch", arguments);
        assert_eq!(response["success"], true, "{response}");
    }

    fn top(&mut self) -> Value {
        let response = self.request("stackTrace", json!({ "threadId": 1 }));
        response["body"]["stackFrames"][0].clone()
    }

    /// The variables of a frame's scope, by name and value.
    fn scope(&mut self, frame: u64, name: &str) -> Vec<(String, String, u64)> {
        let scopes = self.request("scopes", json!({ "frameId": frame }));
        let reference = scopes["body"]["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|scope| scope["name"] == name)
            .unwrap_or_else(|| panic!("no {name} scope: {scopes}"))
            ["variablesReference"]
            .clone();
        self.variables(reference.as_u64().unwrap())
    }

    fn variables(&mut self, reference: u64) -> Vec<(String, String, u64)> {
        let response = self
            .request("variables", json!({ "variablesReference": reference }));
        response["body"]["variables"]
            .as_array()
            .unwrap_or_else(|| panic!("{response}"))
            .iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap().to_string(),
                    v["value"].as_str().unwrap().to_string(),
                    v["variablesReference"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    fn pipe(&mut self) -> Vec<(String, String)> {
        self.scope(0, "Pipe")
            .into_iter()
            .map(|(name, value, _)| (name, value))
            .collect()
    }
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
}

fn example(name: &str) -> PathBuf {
    root().join(format!("Documentation/examples/{name}.fipe"))
}

fn outcomes() -> Client {
    let mut client = Client::new();
    client.launch(json!({
        "program": example("02-outcomes"),
        "host": root().join("tests/fixtures/host"),
        "trace": root().join("tests/fixtures/traces/02-outcomes.txt"),
    }));
    client
}

/// A program of its own under a trace of its own, in a fresh directory.
fn scratch(name: &str, program: &str, trace: &str) -> (Client, PathBuf) {
    let dir = std::env::temp_dir()
        .join(format!("fipe-dap-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("program.fipe");
    std::fs::write(&path, program).unwrap();
    std::fs::write(dir.join("trace.txt"), trace).unwrap();
    let mut client = Client::new();
    client.launch(json!({
        "program": path,
        "host": root().join("tests/fixtures/host"),
        "trace": dir.join("trace.txt"),
        "budget": 1_000_000,
    }));
    (client, path)
}

#[test]
fn the_stage_columns_of_a_line_are_its_pipe_marks() {
    let mut client = outcomes();
    let response = client.request(
        "breakpointLocations",
        json!({ "source": { "path": example("02-outcomes") }, "line": 7 }),
    );
    assert_eq!(
        response["body"]["breakpoints"],
        json!([
            { "line": 7, "column": 20 },
            { "line": 7, "column": 27 },
            { "line": 7, "column": 46 },
        ])
    );
}

#[test]
fn a_stage_breakpoint_stops_at_its_mark_and_next_goes_stage_by_stage() {
    let mut client = outcomes();
    let response = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("02-outcomes") },
            "breakpoints": [{ "line": 7, "column": 20 }],
        }),
    );
    assert_eq!(response["body"]["breakpoints"][0]["verified"], true);
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "breakpoint");
    let top = client.top();
    assert_eq!(
        (top["line"].clone(), top["column"].clone()),
        (json!(7), json!(20))
    );
    let entry = "Entry \"config.toml\" \"./config.toml\" 12 60";
    assert_eq!(
        client.pipe(),
        pairs(&[
            ("mark", "|:"),
            ("in", &format!("Just ({entry})")),
            ("side", "good"),
            ("stage", "runs"),
            ("out", "not yet"),
        ])
    );
    client.request("next", json!({ "threadId": 1 }));
    assert_eq!(client.last("stopped")["reason"], "step");
    let top = client.top();
    assert_eq!(
        (top["line"].clone(), top["column"].clone()),
        (json!(7), json!(27))
    );
    let file = "Just \"port = 8080\\n\"";
    assert_eq!(
        client.pipe(),
        pairs(&[
            ("mark", "|!"),
            ("in", file),
            ("side", "good"),
            ("stage", "skipped"),
            ("out", "not yet"),
        ])
    );
    client.request("next", json!({ "threadId": 1 }));
    let top = client.top();
    assert_eq!(
        (top["line"].clone(), top["column"].clone()),
        (json!(7), json!(46))
    );
    let pipe = client.pipe();
    assert_eq!(pipe[0], (String::from("mark"), String::from("||")));
    assert_eq!(pipe[1].1, file);
    assert_eq!(pipe[3].1, "skipped");
    client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
}

#[test]
fn on_a_miss_the_pipe_scope_shows_the_other_side() {
    let program = "use fs\ndefault_config = \"port = 80\"\n\
                   find ./missing.toml |: cat |! log \"no config\" || default_config\n";
    let trace = "value ()\ncall commands.read (\"./missing.toml\") -> Ok \"./missing.toml\"\n\
                 call commands.find (\"./missing.toml\") -> Nothing\n";
    let (mut client, path) = scratch("miss", program, trace);
    client.request(
        "setBreakpoints",
        json!({ "source": { "path": path }, "breakpoints": [{ "line": 3, "column": 21 }] }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "breakpoint");
    assert_eq!(
        client.pipe(),
        pairs(&[
            ("mark", "|:"),
            ("in", "Nothing"),
            ("side", "bad"),
            ("stage", "skipped"),
            ("out", "not yet"),
        ])
    );
    client.request("next", json!({ "threadId": 1 }));
    let pipe = client.pipe();
    assert_eq!(pipe[0].1, "|!");
    assert_eq!(pipe[1].1, "Nothing");
    assert_eq!(pipe[3].1, "runs");
    client.request("next", json!({ "threadId": 1 }));
    let pipe = client.pipe();
    assert_eq!(pipe[0].1, "||");
    assert_eq!(pipe[1].1, "Nothing");
    assert_eq!(pipe[3].1, "runs");
    client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
}

#[test]
fn step_in_at_a_stage_enters_the_function_it_calls() {
    let mut client = outcomes();
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("02-outcomes") },
            "breakpoints": [{ "line": 28, "column": 10 }],
        }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "breakpoint");
    assert_eq!(client.top()["name"], "main");
    client.request("stepIn", json!({ "threadId": 1 }));
    let response = client.request("stackTrace", json!({ "threadId": 1 }));
    let frames = response["body"]["stackFrames"].as_array().unwrap().clone();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[0]["name"], "describe");
    assert_eq!(frames[1]["name"], "main");
    client.request("stepOut", json!({ "threadId": 1 }));
    let response = client.request("stackTrace", json!({ "threadId": 1 }));
    assert_eq!(response["body"]["stackFrames"].as_array().unwrap().len(), 1);
}

#[test]
fn a_line_breakpoint_in_a_function_shows_its_locals_and_evaluates_a_name() {
    let mut client = outcomes();
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("02-outcomes") },
            "breakpoints": [{ "line": 32 }],
        }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "breakpoint");
    let top = client.top();
    assert_eq!(top["name"], "read_port");
    assert_eq!(top["line"], 32);
    let locals = client.scope(0, "Locals");
    assert!(
        locals
            .iter()
            .any(|(name, value, _)| name == "text" && value == "\"8080\""),
        "{locals:?}"
    );
    let found = client
        .request("evaluate", json!({ "expression": "text", "frameId": 0 }));
    assert_eq!(found["body"]["result"], "\"8080\"");
    let expression = client.request(
        "evaluate",
        json!({ "expression": "text + 1", "frameId": 0 }),
    );
    assert_eq!(expression["success"], false);
    let unbound = client
        .request("evaluate", json!({ "expression": "nope", "frameId": 0 }));
    assert_eq!(unbound["message"], "nope is not bound in this frame");
    let effects = client.scope(0, "Effects");
    assert!(effects.len() >= 2, "{effects:?}");
}

#[test]
fn a_host_default_runs_as_part_of_the_line_that_calls_it() {
    // Line 24 calls `fs.list`, whose `long` parameter has a default in
    // the host's fs.fipe.
    let mut client = Client::new();
    client.launch(json!({
        "program": example("05-modules"),
        "host": root().join("tests/fixtures/host"),
        "trace": root().join("tests/fixtures/traces/05-modules.txt"),
    }));
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("05-modules") },
            "breakpoints": [{ "line": 24 }],
        }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.top()["line"], 24);
    client.request("next", json!({ "threadId": 1 }));
    assert_eq!(client.last("stopped")["reason"], "step");
    let top = client.top();
    assert_eq!(
        (top["source"]["name"].clone(), top["line"].clone()),
        (json!("main"), json!(24))
    );
    assert_eq!(top["column"], 27, "the `| first` stage");
}

#[test]
fn a_compound_value_opens_by_constructor_and_field() {
    let mut client = outcomes();
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("02-outcomes") },
            "breakpoints": [{ "line": 7, "column": 20 }],
        }),
    );
    client.request("configurationDone", json!({}));
    let pipe = client.scope(0, "Pipe");
    let (_, _, just) = pipe
        .iter()
        .find(|(name, _, _)| name == "in")
        .unwrap()
        .clone();
    assert!(just > 0);
    let inside = client.variables(just);
    assert_eq!(inside.len(), 1, "{inside:?}");
    let entry = client.variables(inside[0].2);
    let names: Vec<&str> =
        entry.iter().map(|(name, _, _)| name.as_str()).collect();
    assert_eq!(names, ["name", "path", "size", "age"]);
    assert_eq!(entry[0].1, "\"config.toml\"");
}

#[test]
fn a_column_without_a_mark_is_not_verified() {
    let mut client = outcomes();
    let response = client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("02-outcomes") },
            "breakpoints": [{ "line": 7, "column": 2 }, { "line": 4 }],
        }),
    );
    let answers = &response["body"]["breakpoints"];
    assert_eq!(answers[0]["verified"], false);
    assert_eq!(answers[0]["message"], "no pipe mark at 7:2");
    assert_eq!(answers[1]["verified"], false);
}

#[test]
fn pause_stops_a_running_program() {
    let mut client = outcomes();
    client.send("configurationDone", json!({}));
    assert!(client.session.running());
    let response = client.send("pause", json!({ "threadId": 1 }));
    assert_eq!(response["success"], true);
    client.session.settle();
    client.sent.extend(client.session.out.take());
    assert_eq!(client.last("stopped")["reason"], "pause");
    client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
}

#[test]
fn stop_on_entry_stops_before_the_first_node() {
    let mut client = Client::new();
    client.request("initialize", json!({}));
    client.request(
        "launch",
        json!({
            "program": example("02-outcomes"),
            "host": root().join("tests/fixtures/host"),
            "trace": root().join("tests/fixtures/traces/02-outcomes.txt"),
            "stopOnEntry": true,
        }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "entry");
    assert_eq!(client.top()["name"], "main");
}

#[test]
fn a_recursion_without_a_base_case_stops_at_its_call_with_no_budget_left() {
    let (mut client, path) =
        scratch("recursion", "f = n => f (n + 1)\nf 0\n", "value ()\n");
    client.request("configurationDone", json!({}));
    let stopped = client.last("stopped");
    assert_eq!(stopped["reason"], "exception");
    let top = client.top();
    assert_eq!(top["name"], "fault");
    assert_eq!(top["source"]["path"], json!(path));
    // The VM Faults at the node it could not pay, inside the call
    // `f (n + 1)` (columns 10 to 18).
    assert_eq!(top["line"], 1);
    let column = top["column"].as_u64().unwrap();
    assert!((10..=18).contains(&column), "{top}");
    let budget = client.scope(0, "Budget");
    assert_eq!(budget[0], (String::from("remaining"), String::from("0"), 0));
    let info = client.request("exceptionInfo", json!({ "threadId": 1 }));
    assert_eq!(info["body"]["breakMode"], "always");
    assert!(
        info["body"]["description"]
            .as_str()
            .unwrap()
            .starts_with("fault in main at 1:1"),
        "{info}"
    );
}

#[test]
fn a_launch_that_does_not_check_says_where_in_words() {
    let dir = std::env::temp_dir()
        .join(format!("fipe-dap-words-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("bad.fipe");
    std::fs::write(&program, "x = nope 1\n").unwrap();
    let mut client = Client::new();
    client.request("initialize", json!({}));
    let response = client.request("launch", json!({ "program": program }));
    let message = response["message"].as_str().unwrap();
    assert!(
        message.contains(
            "does not check: 1 diagnostic, the first at 1:5: unknown name"
        ),
        "{message}"
    );
    assert!(
        !message.contains('{') && !message.contains("Span"),
        "{message}"
    );
}

#[test]
fn a_closure_frame_shows_what_it_captured() {
    let program =
        "mk = n => (x => x + n)\nadd10: Integer -> Integer = mk 10\nadd10 1\n";
    let (mut client, path) = scratch("captures", program, "value ()\n");
    client.request(
        "setBreakpoints",
        json!({ "source": { "path": path }, "breakpoints": [{ "line": 1 }] }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.top()["name"], "mk");
    client.request("continue", json!({ "threadId": 1 }));
    let top = client.top();
    assert!(
        top["name"].as_str().unwrap().starts_with("arm at 1:"),
        "{top}"
    );
    let captures = client.scope(0, "Captures");
    assert_eq!(captures, [(String::from("n"), String::from("10"), 0)]);
    let locals = client.scope(0, "Locals");
    assert!(
        locals
            .iter()
            .any(|(name, value, _)| name == "x" && value == "1"),
        "{locals:?}"
    );
    client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
}

#[test]
fn an_arm_binding_and_module_values_are_shown_by_name() {
    let program = "k = 6\nm = k * 7\npick = {\n    0 => m,\n    other => other + k,\n}\npick 3\n";
    let (mut client, path) = scratch("globals", program, "value ()\n");
    client.request(
        "setBreakpoints",
        json!({ "source": { "path": path }, "breakpoints": [{ "line": 5 }] }),
    );
    client.request("configurationDone", json!({}));
    assert_eq!(client.last("stopped")["reason"], "breakpoint");
    let locals = client.scope(0, "Locals");
    assert!(
        locals
            .iter()
            .any(|(name, value, _)| name == "other" && value == "3"),
        "{locals:?}"
    );
    let globals = client.scope(0, "Globals");
    assert_eq!(
        globals,
        [
            (String::from("k"), String::from("6"), 0),
            (String::from("m"), String::from("42"), 0),
        ]
    );
    let found =
        client.request("evaluate", json!({ "expression": "m", "frameId": 0 }));
    assert_eq!(found["body"]["result"], "42");
    client.request("continue", json!({ "threadId": 1 }));
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
}

#[test]
fn a_session_stepped_line_by_line_observes_what_a_run_does() {
    for name in [
        "01-files",
        "02-outcomes",
        "03-types",
        "04-text",
        "05-modules",
    ] {
        let program = example(name);
        let lines = std::fs::read_to_string(&program).unwrap().lines().count();
        let mut client = Client::new();
        client.launch(json!({
            "program": program,
            "host": root().join("tests/fixtures/host"),
            "trace": root().join(format!("tests/fixtures/traces/{name}.txt")),
        }));
        let every: Vec<Value> =
            (1..=lines).map(|line| json!({ "line": line })).collect();
        client.request(
            "setBreakpoints",
            json!({ "source": { "path": program }, "breakpoints": every }),
        );
        client.request("configurationDone", json!({}));
        let mut stops = 0;
        let fault = loop {
            let ended =
                client.sent.iter().find_map(|m| match m["event"].as_str() {
                    Some("terminated") => Some(None),
                    Some("stopped") if m["body"]["reason"] == "exception" => {
                        Some(
                            m["body"]["description"].as_str().map(String::from),
                        )
                    }
                    _ => None,
                });
            if let Some(fault) = ended {
                break fault;
            }
            stops += 1;
            assert!(stops < 100_000, "{name}: no end");
            client.request("next", json!({ "threadId": 1 }));
        };
        assert!(stops > 0, "{name}: no breakpoint was hit");
        let output = |category: &str| -> Vec<String> {
            client
                .sent
                .iter()
                .filter(|m| {
                    m["event"] == "output" && m["body"]["category"] == category
                })
                .map(|m| m["body"]["output"].as_str().unwrap().to_string())
                .collect()
        };
        let expected = common::expected(name);
        assert_eq!(output("console"), expected.console, "{name}: the trace");
        assert_eq!(output("stdout").concat(), expected.success, "{name}");
        assert_eq!(output("stderr").concat(), expected.error, "{name}");
        if let Some(description) = fault {
            assert_eq!(
                &format!("{description}\n"),
                expected.console.last().unwrap(),
                "{name}"
            );
        }
    }
}

/// The `(line, column)` of each stop until the run ends, resuming each
/// by `command`.
fn stops(client: &mut Client, command: &str) -> Vec<(u64, u64)> {
    let mut stops = Vec::new();
    while client.sent.last().unwrap()["event"] != "terminated" {
        assert_ne!(client.last("stopped")["reason"], "exception");
        let top = client.top();
        stops.push((
            top["line"].as_u64().unwrap(),
            top["column"].as_u64().unwrap(),
        ));
        assert!(stops.len() < 1000, "no end: {stops:?}");
        client.request(command, json!({ "threadId": 1 }));
    }
    stops
}

fn modules(lines: &[u64]) -> Client {
    let mut client = Client::new();
    client.launch(json!({
        "program": example("05-modules"),
        "host": root().join("tests/fixtures/host"),
        "trace": root().join("tests/fixtures/traces/05-modules.txt"),
    }));
    let breakpoints: Vec<Value> =
        lines.iter().map(|line| json!({ "line": line })).collect();
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": example("05-modules") },
            "breakpoints": breakpoints,
        }),
    );
    client.request("configurationDone", json!({}));
    client
}

#[test]
fn a_line_breakpoint_stops_once_each_time_the_run_arrives_at_its_line() {
    // Line 24's pattern and line 29's `with` are announced again after
    // the code they scope; line 30 calls a closure written on it.
    let mut client = modules(&[24, 29, 30]);
    let lines: Vec<u64> = stops(&mut client, "continue")
        .iter()
        .map(|stop| stop.0)
        .collect();
    assert_eq!(lines, [24, 29, 30]);
}

#[test]
fn next_never_comes_back_to_a_line_it_left() {
    let mut client = modules(&[29]);
    let stops = stops(&mut client, "next");
    assert_eq!(stops.first(), Some(&(29, 1)));
    assert!(stops.windows(2).all(|pair| pair[0] < pair[1]), "{stops:?}");
    let lines: Vec<u64> = stops.iter().map(|stop| stop.0).collect();
    assert!(lines.contains(&30) && lines.contains(&32), "{stops:?}");
}

#[test]
fn a_recursive_call_arrives_at_its_line_again_in_each_frame() {
    let program = "down = n => if n == 0 { 0 } else { 1 + (down (n - 1)) }\n\
                   spin = n => if n == 0 { 0 } else { spin (n - 1) }\n\
                   count = {\n    0 => 0,\n    n => count (n - 1),\n}\n\
                   down 2\nspin 2\ncount 2\n";
    let (mut client, path) = scratch("recursion-lines", program, "value ()\n");
    client.request(
        "setBreakpoints",
        json!({
            "source": { "path": path },
            "breakpoints": [{ "line": 1 }, { "line": 2 }, { "line": 5 }],
        }),
    );
    client.request("configurationDone", json!({}));
    let mut seen = Vec::new();
    while client.sent.last().unwrap()["event"] != "terminated" {
        let frames = client.request("stackTrace", json!({ "threadId": 1 }));
        let frames = frames["body"]["stackFrames"].as_array().unwrap().clone();
        seen.push((frames[0]["line"].as_u64().unwrap(), frames.len()));
        assert!(seen.len() < 100, "{seen:?}");
        client.request("continue", json!({ "threadId": 1 }));
    }
    // `down` 2, 1 and 0 each run line 1, each in a frame of its own;
    // `spin` and `count` tail-call, a new frame in the place of the one
    // that calls: `spin` runs line 2 three times, `count` line 5 twice.
    assert_eq!(
        seen,
        [
            (1, 2),
            (1, 3),
            (1, 4),
            (2, 2),
            (2, 2),
            (2, 2),
            (5, 2),
            (5, 2)
        ]
    );
}

#[test]
fn a_user_method_and_its_returned_capture_remain_visible_through_an_adapter() {
    let program = "type M { n: Integer }\nimpl M {\n    op = a b self => {\n        seed = self.n + a * 10 + b\n        x => seed + x\n    },\n}\ninvoke = value => value.op 3 4\nf: Integer -> Integer = invoke (M 2)\nf 1\n";
    // Prove this source uses an actual adapter for the written user method.
    let module = fipe::parse::parse(fipe::lex::lex(program)).unwrap();
    let unit = fipe::check::check(
        &module,
        &fipe::decl::Declarations::new(),
        &[],
        fipe::check::Options::default(),
    )
    .expect("generic written method returning a closure");
    let lowered = fipe::compile::compile(&unit);
    let tables = lowered.tables.borrow();
    let method = tables
        .definitions
        .iter()
        .position(|definition| definition.label == "op")
        .expect("raw user method");
    assert!(
        lowered
            .receiver_adapters
            .iter()
            .any(|adapter| adapter.target
                == fipe::mir::ReceiverTarget::Definition(method)),
        "the user body must be reached through a receiver adapter"
    );
    let raw = lowered.definitions[method].expect("raw method body is emitted");
    assert!(
        lowered.receiver_contexts[raw].is_none(),
        "user method is not a synthetic adapter"
    );
    drop(tables);
    let (mut client, path) = scratch("method-captures", program, "value ()\n");
    client.request("setBreakpoints", json!({"source": {"path": path}, "breakpoints": [{"line": 4}, {"line": 5}]}));
    client.request("configurationDone", json!({}));
    let mut method_seen = false;
    let mut capture_seen = false;
    for _ in 0..12 {
        if client.sent.last().unwrap()["event"] == "terminated" {
            break;
        }
        let top = client.top();
        let name = top["name"].as_str().unwrap();
        if name == "op" {
            let locals = client.scope(0, "Locals");
            for (name, value) in [("a", "3"), ("b", "4"), ("self", "M 2")] {
                assert!(
                    locals
                        .iter()
                        .any(|(actual, shown, _)| actual == name
                            && shown == value),
                    "raw method local {name}: {locals:?}"
                );
            }
            method_seen = true;
        } else if name.starts_with("arm at 5:") {
            assert_eq!(
                client.scope(0, "Captures"),
                [(String::from("seed"), String::from("36"), 0)]
            );
            assert!(client
                .scope(0, "Locals")
                .iter()
                .any(|(name, value, _)| name == "x" && value == "1"));
            capture_seen = true;
        } else {
            panic!("unexpected visible frame at the method/closure breakpoint: {top}");
        }
        client.request("continue", json!({"threadId": 1}));
    }
    assert_eq!(client.sent.last().unwrap()["event"], "terminated");
    assert!(
        method_seen && capture_seen,
        "both real user frames must remain observable"
    );
}
