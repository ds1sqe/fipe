//! A session line whose run Faults is withdrawn: its names, types and
//! claims are as if it had not been given, and its effects stand.
use super::{host, run::BUDGET, structural, Declarations, Machine, Outcome};
use fipe::{
    compile,
    embed::{Reply, Request},
    session::{Report, Session},
    value::Value,
};

/// A host over one session and one machine: it answers file requests
/// from `files`, host calls from `calls` in order, records each call,
/// and withdraws a line whose run Faults.
struct Host {
    session: Session,
    machine: Machine,
    files: Vec<(&'static str, &'static str)>,
    calls: Vec<Result<Value, String>>,
    called: Vec<String>,
    requested: Vec<String>,
}

impl Host {
    const fn new(declarations: Declarations) -> Self {
        Self {
            session: Session::new(declarations),
            machine: Machine::new(Some(BUDGET)),
            files: Vec::new(),
            calls: Vec::new(),
            called: Vec::new(),
            requested: Vec::new(),
        }
    }

    /// The line's report, every file request answered.
    fn check(&mut self, text: &str) -> Report {
        let mut outcome = self.session.line(text);
        loop {
            match outcome {
                Outcome::Request(Request::File { stem }) => {
                    self.requested.push(stem.clone());
                    let text = self
                        .files
                        .iter()
                        .find(|(name, _)| *name == stem)
                        .map(|(_, text)| String::from(*text));
                    outcome = self.session.reply(Reply::File(text));
                }
                Outcome::Request(request) => panic!("{text}: {request:?}"),
                Outcome::Done(report) => return report,
            }
        }
    }

    /// The line's value, its Fault, or its diagnostics when refused.
    fn line(&mut self, text: &str) -> String {
        let report = self.check(text);
        self.run(&report)
    }

    /// Run a checked line; withdraw it when its run Faults.
    fn run(&mut self, report: &Report) -> String {
        let Some(unit) = &report.unit else {
            return format!("refused: {:?}", report.diagnostics);
        };
        let program = compile::compile(unit);
        let mut outcome = self.machine.run(&program);
        while let Outcome::Request(request) = outcome {
            self.called.push(host::key(&program, &request));
            let reply = self.calls.remove(0);
            outcome = self.machine.reply(&program, Reply::Value(reply));
        }
        match outcome {
            Outcome::Done(Ok(value)) => {
                structural::show(&value, &program.tables.borrow())
            }
            Outcome::Done(Err(fault)) => {
                let line = report.accepted.expect("an accepted line");
                let withdrawn = self.session.withdraw(line).expect("the last");
                self.machine.withdraw(&withdrawn);
                format!("fault: {}", fault.reason)
            }
            Outcome::Request(_) => unreachable!("every request was answered"),
        }
    }
}

/// A line refused for naming what no accepted line defines.
fn unknown(shown: &str) -> bool {
    shown.starts_with("refused: ")
        && (shown.contains("UnknownName") || shown.contains("UnknownType"))
}

#[test]
fn a_binding_whose_run_faults_is_unknown_until_given_again() {
    let mut host = Host::new(Declarations::new());
    assert_eq!(
        host.line("x = 1.0 / 0.0"),
        "fault: the result is not finite"
    );
    assert!(unknown(&host.line("x + 1.0")));
    assert_eq!(host.line("x = 1.0"), "()");
    assert_eq!(host.line("x + 1.0"), "2.0");
}

#[test]
fn a_type_from_an_earlier_line_stays_when_a_construction_faults() {
    let mut host = Host::new(Declarations::new());
    assert_eq!(host.line("type T { n: F64 = (1.0 / 0.0) }"), "()");
    assert_eq!(host.line("t = T"), "fault: the result is not finite");
    assert!(unknown(&host.line("t")));
    assert_eq!(host.line("T { n: 2.0 }"), "T 2.0");
}

#[test]
fn a_withdrawn_unit_takes_its_type_and_claim_with_it() {
    let mut host = Host::new(Declarations::new());
    let unit = "type P: Show { n: F64 }\n\
                impl P: Show { show = self => \"p\" }\n\
                p = P { n: 1.0 / 0.0 }";
    assert_eq!(host.line(unit), "fault: the result is not finite");
    let shown = host.line("P { n: 1.0 }");
    assert!(unknown(&shown), "P is gone: {shown}");
    assert!(unknown(&host.line("p")), "p is gone");
    let fixed = "type P: Show { n: F64 }\n\
                 impl P: Show { show = self => \"p\" }\n\
                 p = P { n: 1.0 }";
    assert_eq!(host.line(fixed), "()");
    assert_eq!(host.line("p.show"), "\"p\"");
}

#[test]
fn a_module_whose_init_faults_runs_again_when_used_again() {
    let mut host = Host::new(host::declarations());
    host.files = vec![("m", "use cli\npub a = cli.port\npub b = cli.port")];
    host.calls = vec![
        Ok(Value::Int(1)),
        Err(String::from("down")),
        Ok(Value::Int(1)),
        Ok(Value::Int(2)),
    ];
    assert_eq!(host.line("use m"), "fault: host: down");
    assert!(unknown(&host.line("m.a")), "the use is gone");
    assert_eq!(host.line("use m"), "()");
    assert_eq!(host.line("m.b"), "2");
    assert_eq!(host.called, ["cli.port"; 4], "the init ran twice");
}

#[test]
fn a_module_a_withdrawn_line_initialized_runs_again() {
    let mut host = Host::new(host::declarations());
    host.files = vec![("n", "use cli\npub c = cli.port")];
    host.calls = vec![Ok(Value::Int(1)), Ok(Value::Int(2))];
    let line = "use n\nx = 1.0 / 0.0";
    assert_eq!(host.line(line), "fault: the result is not finite");
    assert_eq!(host.line("use n\nn.c"), "2");
    assert_eq!(host.called, ["cli.port"; 2], "the init ran twice");
}

#[test]
fn a_native_a_withdrawn_line_reached_is_not_called_for_another() {
    let mut host = Host::new(Declarations::new());
    assert_eq!(
        host.line("x = (floor 2.5) + 1.0 / 0.0"),
        "fault: the result is not finite"
    );
    // `ceil` and `sub` take the declaration numbers `floor` and `div`
    // had, and the tables reach the same length again.
    assert_eq!(host.line("(ceil 2.5) + 1.0 - 0.5"), "3.5");
}

#[test]
fn a_prelude_type_a_withdrawn_line_reached_is_found_again() {
    let mut host = Host::new(Declarations::new());
    assert_eq!(host.line("y = 1"), "()");
    assert_eq!(
        host.line("x = (1.0 > 0.5, 1.0 / 0.0)"),
        "fault: the result is not finite"
    );
    // As many meanings again, `Bool` numbered one later.
    assert_eq!(host.line("(Just 1.0) == Nothing"), "False");
}

#[test]
fn a_file_a_withdrawn_line_loaded_is_requested_and_typed_again() {
    let mut host = Host::new(Declarations::new());
    host.files = vec![("m", "pub a = 1\npub b = 1.0 / 0.0")];
    assert_eq!(host.line("use m"), "fault: the result is not finite");
    host.files = vec![("m", "pub a = 2.5")];
    assert_eq!(host.line("use m\nm.a"), "2.5");
    assert_eq!(host.requested, ["m", "m"]);
}

#[test]
fn a_file_type_a_withdrawn_line_typed_is_typed_again() {
    let mut host = Host::new(Declarations::new());
    host.files =
        vec![("m", "pub type R { pub a: Integer = 1 }\npub b = 1.0 / 0.0")];
    assert_eq!(
        host.line("use m\nr = m.R\nr.a"),
        "fault: the result is not finite"
    );
    host.files = vec![("m", "pub type R { pub a: F64 = 2.5 }")];
    assert_eq!(host.line("use m\nr = m.R\nr.a"), "2.5");
}

#[test]
fn a_query_before_withdrawal_leaves_nothing_of_the_line() {
    let mut host = Host::new(Declarations::new());
    let report = host.check("type P: Integer where self > 0\nx = 1.0 / 0.0");
    // The query types `P`'s test against the accepted line.
    let Outcome::Done(query) = host.session.query("p: P = 1") else {
        panic!("a query asks for nothing here")
    };
    assert_eq!(query.diagnostics, []);
    assert_eq!(host.run(&report), "fault: the result is not finite");
    // The next line takes the withdrawn line's number; the one after
    // reads its test.
    assert_eq!(host.line("type P: Integer where self < 0"), "()");
    assert_eq!(host.line("p: P = -1\np"), "-1");
    assert!(host.line("q: P = 1").starts_with("refused: "));
}

#[test]
fn a_stem_answered_absent_stays_absent_after_a_withdrawn_retry() {
    let mut host = Host::new(Declarations::new());
    host.files = vec![("f", "use m\npub k = 1")];
    host.check("use f");
    assert_eq!(host.requested, ["f", "m"]);
    host.files.push(("m", "pub a = 1.0 / 0.0"));
    assert_eq!(host.line("use m"), "fault: the result is not finite");
    // `f`'s `use m` keeps the remembered absent answer.
    host.check("y = 1");
    assert_eq!(host.requested, ["f", "m", "m"]);
}

#[test]
fn a_withdrawn_line_takes_its_settled_entries_with_it() {
    let mut host = Host::new(Declarations::new());
    for line in [
        "trial = n d => if d * d > n { True } else { if n % d == 0 { False } else { trial n (d + 1) } }",
        "is_prime = n => n > 1 && trial n 2",
        "type Prime: Integer where is_prime self",
    ] {
        assert_eq!(host.line(line), "()");
    }
    let settled = host.session.settled(None).len();
    // `p` stands after the end of the next line, so an entry of `p`
    // cannot pass for one of `q`.
    let line = "x = 1.0 / 0.0\np: Prime = 7";
    assert_eq!(host.line(line), "fault: the result is not finite");
    assert_eq!(host.session.settled(None).len(), settled);
    assert_eq!(host.line("q: Prime = 11"), "()");
    assert_eq!(host.session.settled(Some("q")).len(), 1);
}

#[test]
fn a_weak_variable_a_withdrawn_line_solved_is_open_again() {
    let mut host = Host::new(Declarations::new());
    // A pattern-bound binding stays monomorphic: its first use fixes it.
    assert_eq!(host.line("[f] = [(x => x)]"), "()");
    let line = "a = (1.0 / 0.0, f 1)";
    assert_eq!(host.line(line), "fault: the result is not finite");
    // Checked only: calling `f` at run time is outside this test.
    assert_eq!(host.check("b = f True").diagnostics, []);
}

#[test]
fn a_value_a_withdrawn_run_stored_is_not_read_by_a_later_line() {
    let mut declarations = host::declarations();
    declarations
        .module("config", include_str!("../fixtures/host/config.fipe"))
        .unwrap();
    let mut host = Host::new(declarations);
    host.files = vec![("m", "use cli\npub a = cli.port\npub b = cli.port")];
    host.calls = vec![Ok(Value::Int(1)), Err(String::from("down"))];
    assert_eq!(host.line("use m"), "fault: host: down");
    // The definition the withdrawn run stored is numbered again here.
    assert_eq!(host.line("use config\nconfig.max_port"), "65535");
}

#[test]
fn a_withdrawn_line_leaves_the_session_as_if_never_given() {
    let lines = ["f = n => n + 1", "type R { a: Integer }", "f 41"];
    let mut fresh = Host::new(Declarations::new());
    let mut withdrawn = Host::new(Declarations::new());
    assert_eq!(fresh.line(lines[0]), withdrawn.line(lines[0]));
    let faulting = "type Q { b: F64 }\nimpl Q { c = self => self.b }\n\
                    q = Q { b: 0.0 / 0.0 }";
    assert_eq!(withdrawn.line(faulting), "fault: the result is not finite");
    for line in &lines[1..] {
        let mut expected = fresh.check(line);
        let mut given = withdrawn.check(line);
        assert_eq!(fresh.session.source(), withdrawn.session.source());
        expected.accepted = None;
        given.accepted = None;
        assert_eq!(given, expected, "{line}");
    }
    assert_eq!(fresh.line("f 1"), "2");
    assert_eq!(withdrawn.line("f 1"), "2");
}
