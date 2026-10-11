use super::run::{run, session, shown};
use fipe::value::Reason;

#[test]
fn a_known_body_is_entered_with_its_spots() {
    assert_eq!(shown("add2 = x => x + 2\nadd2 4"), "6");
    assert_eq!(shown("f = (w = 80) (c = (w / 8)) => c\nf 16"), "2");
    assert_eq!(shown("f = (w = 80) (c = (w / 8)) => c\nf 16 3"), "3");
}

#[test]
fn a_session_keeps_each_line_value_and_a_closure_keeps_what_it_captured() {
    assert_eq!(
        session(&["x = 1", "f = () => x", "x = 2", "f ()"]),
        ["()", "()", "()", "1"]
    );
    assert_eq!(session(&["add2 = x => x + 2", "add2 4"]), ["()", "6"]);
}

#[test]
fn eval_apply_fills_enters_and_applies_the_rest() {
    assert_eq!(shown("app = f => (f 1) 2\napp (a b => a + b)"), "3");
    assert_eq!(
        shown("app = f => f 1 2\napp (a => (b => (a, b)))"),
        "(1, 2)"
    );
    assert_eq!(shown("app = f => f 1\n(app (a b => a + b)) 4"), "5");
    assert_eq!(
        shown("pair = a b => (x => (a, b, x))\npair 1 2 3"),
        "(1, 2, 3)"
    );
    assert_eq!(shown("k = (f => f 1) (x => x + 1)\nk"), "2");
    assert_eq!(
        shown("twice = f => (x => f (f x))\ntwice (n => n * 2) 3"),
        "12"
    );
}

#[test]
fn an_open_spot_makes_a_function_of_the_rest() {
    assert_eq!(shown("pair = a b => (a, b)\n(pair --b 2) 1"), "(1, 2)");
}

#[test]
fn conditionals_blocks_and_binds_run_in_order() {
    assert_eq!(shown("if 1 == 1 { 2 } else { 3 }"), "2");
    assert_eq!(shown("if 1 == 2 { 2 } else { 3 }"), "3");
    assert_eq!(shown("{\n    x = 1\n    y = x + 1\n    y * 2\n}"), "4");
    assert_eq!(shown("with cwd /tmp { 1 }"), "1");
}

#[test]
fn a_recursive_definition_grows_the_frames_not_the_stack() {
    let source =
        "count = n => if n == 0 { 0 } else { 1 + (count (n - 1)) }\ncount 2000";
    assert_eq!(shown(source), "2000");
}

#[test]
fn a_host_call_is_a_request_and_a_failed_reply_faults() {
    let module = fipe::parse::parse(fipe::lex::lex("\"a\" | put ./f")).unwrap();
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .commands(include_str!("../fixtures/host/commands.fipe"))
        .unwrap();
    let unit = fipe::check::check(
        &module,
        &declarations,
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let program = fipe::compile::compile(&unit);
    let mut machine = fipe::vm::Machine::new(super::run::BUDGET);
    let fipe::embed::Outcome::Request(fipe::embed::Request::Call { .. }) =
        machine.run(&program)
    else {
        panic!("the sink literal's reader is a host call")
    };
    let reply = fipe::embed::Reply::Value(Err("no host".into()));
    match machine.reply(&program, reply) {
        fipe::embed::Outcome::Done(Err(fault)) => {
            assert_eq!(fault.reason, Reason::Host("no host".into()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_run_charges_the_machine() {
    let module = fipe::parse::parse(fipe::lex::lex("1 + 2")).unwrap();
    let unit = fipe::check::check(
        &module,
        &fipe::decl::Declarations::new(),
        &[],
        fipe::check::Options::default(),
    )
    .unwrap();
    let program = fipe::compile::compile(&unit);
    let mut machine = fipe::vm::Machine::new(super::run::BUDGET);
    let _ = machine.run(&program);
    assert!(machine.charged() > 0);
    assert_eq!(machine.budget(), super::run::BUDGET);
    assert!(machine.frames().is_empty());
    let _ = run("1");
}
