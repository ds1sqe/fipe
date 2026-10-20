//! Breakpoints, stepping and inspection, through the session the
//! binary runs.
use fipe_dap::session::{Control, Session};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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
