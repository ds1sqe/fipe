//! The host suspension: a host call is a request with its arguments in
//! spot order, the run resumes on the reply, and the observation is the
//! value or Fault and the trace; the charges are not part of it (E1, H1).
use super::host::{key, observe, observe_charged, program, Script};
use super::Value;
use fipe::{
    embed::{Reply, Request},
    mir::Program,
    value::{Prelude, Reason, Record},
};
use std::rc::Rc;

fn keys(program: &Program, trace: &[(Request, Reply)]) -> Vec<String> {
    trace
        .iter()
        .map(|(request, _)| key(program, request))
        .collect()
}

fn arguments(request: &Request) -> &[Value] {
    let Request::Call { arguments, .. } = request else {
        panic!("{request:?}")
    };
    arguments
}

fn prelude(program: &Program) -> Prelude {
    Prelude::new(&program.tables.borrow())
}

/// An `Entry` of the conformance host.
fn entry(program: &Program, name: &str) -> Value {
    let tables = program.tables.borrow();
    let meaning = tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == "Entry")
        .expect("the host's Entry");
    Value::Record(Rc::new(Record {
        meaning,
        constructor: 0,
        fields: vec![
            Value::text(name),
            Value::text(&format!("/var/log/{name}")),
            Value::Int(2_000_000),
            Value::Int(9),
        ],
    }))
}

/// A path literal read by the host's `Src` reader: the text, accepted.
fn read(program: &Program, script: Script, path: &str) -> Script {
    script.reply("commands.read", Ok(prelude(program).ok(Value::text(path))))
}

#[test]
fn a_host_call_suspends_with_its_arguments_in_spot_order() {
    let source = "use fs\nls -l /var/log | take 1";
    let program = program(source);
    let script = read(&program, Script::default(), "/var/log").reply(
        "commands.ls",
        Ok(Value::seq(vec![
            entry(&program, "a.log"),
            entry(&program, "b.log"),
        ])),
    );
    let observation = observe(&program, Some(1 << 20), script);
    // The path literal's reader is the host's own, a host call at run
    // time; then the listing, with every spot in order.
    assert_eq!(
        keys(&program, &observation.trace),
        ["commands.read", "commands.ls"]
    );
    let prelude = prelude(&program);
    assert_eq!(
        arguments(&observation.trace[1].0),
        [
            Value::text("/var/log"),
            prelude.bool(true),
            prelude.bool(false),
            Value::Int(1),
            Value::text("*"),
        ]
    );
    assert_eq!(
        observation.result,
        Ok(Value::seq(vec![entry(&program, "a.log")]))
    );
}

#[test]
fn named_arguments_evaluate_once_in_source_order() {
    let source = "use fs\nfs.list -path ./etc (Rd 7)";
    let program = program(source);
    let script = read(&program, Script::default(), "./etc")
        .reply("commands.Rd", Ok(Value::text("rd 7")))
        .reply("fs.list", Ok(Value::seq(Vec::new())));
    let observation = observe(&program, Some(1 << 20), script);
    assert_eq!(
        keys(&program, &observation.trace),
        ["commands.read", "commands.Rd", "fs.list"]
    );
    assert_eq!(
        arguments(&observation.trace[2].0),
        [
            Value::text("rd 7"),
            Value::text("./etc"),
            prelude(&program).bool(false)
        ]
    );
}

#[test]
fn a_skipped_side_has_no_host_effects() {
    let source = "find ./x |: cat || \"none\"";
    let program = program(source);
    let missed = read(&program, Script::default(), "./x")
        .reply("commands.find", Ok(prelude(&program).nothing()));
    let observation = observe(&program, Some(1 << 20), missed);
    assert_eq!(
        keys(&program, &observation.trace),
        ["commands.read", "commands.find"]
    );
    assert_eq!(observation.result, Ok(Value::text("none")));
    let found = read(&program, Script::default(), "./x")
        .reply(
            "commands.find",
            Ok(prelude(&program).just(entry(&program, "x"))),
        )
        .reply("commands.cat", Ok(Value::text("text of x")));
    let observation = observe(&program, Some(1 << 20), found);
    assert_eq!(
        keys(&program, &observation.trace),
        ["commands.read", "commands.find", "commands.cat"]
    );
    assert_eq!(observation.result, Ok(Value::text("text of x")));
}

#[test]
fn a_fault_leaves_the_prior_effects() {
    let source = "x = find ./a\nfault \"stop\"";
    let program = program(source);
    let script = read(&program, Script::default(), "./a")
        .reply("commands.find", Ok(prelude(&program).nothing()));
    let observation = observe(&program, Some(1 << 20), script);
    assert_eq!(
        keys(&program, &observation.trace),
        ["commands.read", "commands.find"]
    );
    assert_eq!(
        observation.result.unwrap_err().reason,
        Reason::Message("stop".into())
    );
}

#[test]
fn a_host_failure_faults_at_the_call() {
    let source = "find ./a";
    let program = program(source);
    let script = read(&program, Script::default(), "./a")
        .reply("commands.find", Err("disk on fire".into()));
    let observation = observe(&program, Some(1 << 20), script);
    let fault = observation.result.unwrap_err();
    assert_eq!(fault.reason, Reason::Host("disk on fire".into()));
    assert_eq!(&source[fault.span.start..fault.span.end], "find ./a");
    assert_eq!(observation.trace.len(), 2);
}

#[test]
fn replaying_one_script_twice_gives_equal_observations() {
    let source = "find ./x |: cat || \"none\"";
    let program = program(source);
    let script = read(&program, Script::default(), "./x")
        .reply("commands.find", Ok(prelude(&program).nothing()));
    let first = observe(&program, Some(1 << 20), script.clone());
    assert_eq!(first, observe(&program, Some(1 << 20), script.clone()));
    assert_eq!(first.output, Vec::<u8>::new());
    // Charges are the VM's metering, not an observation: an unbounded
    // run observes the same.
    assert_eq!(first, observe(&program, None, script));
}

#[test]
fn a_host_call_the_budget_cannot_pay_makes_no_request() {
    let source = "find ./x";
    let program = program(source);
    let script = || {
        read(&program, Script::default(), "./x")
            .reply("commands.find", Ok(prelude(&program).nothing()))
    };
    let (full, units) = observe_charged(&program, None, script());
    let full = keys(&program, &full.trace);
    assert_eq!(full, ["commands.read", "commands.find"]);
    let mut before = 0;
    let mut seen = 0;
    for budget in 0..units {
        let observation = observe(&program, Some(budget), script());
        let fault = observation.result.unwrap_err();
        assert_eq!(fault.reason, Reason::Budget);
        let at = &source[fault.span.start..fault.span.end];
        // The effects that stand are exactly the full run's first ones,
        // in order, one more each time a call becomes payable: the find
        // is never asked under a budget that stops the run.
        let asked = keys(&program, &observation.trace);
        assert_eq!(asked, full[..asked.len()], "{budget}: {at}");
        assert!(asked.len() == before || asked.len() == before + 1);
        assert!(asked.len() < 2, "{budget}: {at}");
        before = asked.len();
        if at == "find ./x" {
            assert_eq!(asked, ["commands.read"], "{budget}");
            seen += 1;
        }
    }
    assert!(seen > 0, "some budget stops at the find");
    assert_eq!(before, 1, "the read stands once its call is paid");
}

#[test]
fn with_puts_its_key_in_the_environment_of_a_host_call() {
    let source = "with lang \"C\" { find ./x }\nfind ./y";
    let program = program(source);
    let script = read(&program, Script::default(), "./x")
        .reply("commands.find", Ok(prelude(&program).nothing()));
    let script = read(&program, script, "./y")
        .reply("commands.find", Ok(prelude(&program).nothing()));
    let observation = observe(&program, Some(1 << 20), script);
    let environments: Vec<_> = observation
        .trace
        .iter()
        .map(|(request, _)| {
            let Request::Call { environment, .. } = request else {
                unreachable!()
            };
            let tables = program.tables.borrow();
            environment
                .iter()
                .map(|(key, value)| (tables.keys[*key].clone(), value.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    let c = vec![(String::from("lang"), Value::text("C"))];
    assert_eq!(environments, [c.clone(), c, vec![], vec![]]);
}

#[test]
fn a_shown_value_at_a_byte_spot_reaches_the_host_as_bytes() {
    let source = "type P: Show { n: I64 }\n(P 1) | put ./out";
    let program = program(source);
    let script = read(&program, Script::default(), "./out")
        .reply("commands.put", Ok(Value::Unit));
    let observation = observe(&program, Some(1 << 20), script);
    assert_eq!(observation.result, Ok(Value::Unit));
    let (put, _) = &observation.trace[1];
    let bytes = "P 1".bytes().map(|b| Value::Int(i64::from(b))).collect();
    assert_eq!(arguments(put)[1], Value::seq(bytes));
}

#[test]
fn a_call_in_tail_position_keeps_one_frame_for_its_recursion() {
    let source = "down = n => if n == 0 { find ./x } else { down (n - 1) }\n\
                  down 1000";
    let program = program(source);
    let mut machine = fipe::vm::Machine::new(None);
    // The innermost call asks for its path; every caller is gone.
    let fipe::embed::Outcome::Request(_) = machine.run(&program) else {
        panic!("the innermost call reads its path")
    };
    assert!(machine.frames().len() <= 2, "{}", machine.frames().len());
}

#[test]
fn a_call_not_in_tail_position_keeps_its_caller() {
    let source = "count = n => if n == 0 { find ./x } else { (count (n - 1)) || Nothing }\n\
                  count 10";
    let program = program(source);
    let mut machine = fipe::vm::Machine::new(None);
    let fipe::embed::Outcome::Request(_) = machine.run(&program) else {
        panic!("the innermost call reads its path")
    };
    assert!(machine.frames().len() > 10, "{}", machine.frames().len());
}

/// The keys a source's run asks, with `cli.port` answered `port` each
/// time, and its result.
fn port_run(source: &str, port: i64, asked: usize) -> (Vec<String>, String) {
    let program = program(source);
    let mut script = Script::default();
    for _ in 0..asked {
        script = script.reply("cli.port", Ok(Value::Int(port)));
    }
    let observation = observe(&program, Some(1 << 20), script);
    let shown = match &observation.result {
        Ok(value) => {
            fipe::value::structural::show(value, &program.tables.borrow())
        }
        Err(fault) => format!("fault: {}", fault.reason),
    };
    (keys(&program, &observation.trace), shown)
}

#[test]
fn a_skipped_fallback_conjunction_or_error_side_asks_the_host_nothing() {
    let none: [&str; 0] = [];
    for (source, result) in [
        ("use cli\nJust 3 || cli.port", "3"),
        ("use cli\nFalse && (cli.port > 0)", "False"),
        ("use cli\nTrue || (cli.port > 0)", "True"),
        (
            "use cli\nr: Result<Integer, Integer> = Ok 1\n\
             r |! (_ => Err cli.port)",
            "Ok 1",
        ),
    ] {
        assert_eq!(
            port_run(source, 7, 0),
            (none.map(String::from).to_vec(), result.into()),
            "{source}"
        );
    }
    // The same sides, taken, ask once.
    for (source, result) in [
        ("use cli\nm: Maybe<Integer> = Nothing\nm || cli.port", "7"),
        ("use cli\nTrue && (cli.port > 0)", "True"),
        ("use cli\nFalse || (cli.port > 0)", "True"),
        (
            "use cli\nr: Result<Integer, Integer> = Err 1\n\
             r |! (_ => Err cli.port)",
            "Err 7",
        ),
    ] {
        assert_eq!(
            port_run(source, 7, 1),
            (vec!["cli.port".into()], result.into()),
            "{source}"
        );
    }
}
