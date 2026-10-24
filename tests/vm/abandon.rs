//! Abandonment stops execution; the driver still owns reply admission/withdrawal.
use super::{host, Machine, Outcome, Value};
use fipe::{
    compile,
    embed::{Reply, Request},
    mir::Program,
    session::{Report, Session},
    value::Reason,
    vm::{Fault, Stop},
};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn checked(session: &mut Session, source: &str) -> Report {
    let Outcome::Done(report) = session.line(source) else {
        panic!("no source request expected")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    report
}

fn program(report: &Report) -> Program {
    compile::compile(report.unit.as_ref().expect("checked execution unit"))
}

fn host_stop(machine: &mut Machine) {
    loop {
        match machine.next_stop() {
            Stop::Request(Request::Call { .. }) => return,
            Stop::Before(_) | Stop::After(..) => {}
            other => panic!("expected host suspension: {other:?}"),
        }
    }
}

fn terminal(machine: &mut Machine) -> Result<Value, Fault> {
    loop {
        match machine.next_stop() {
            Stop::Done(result) => return result,
            Stop::Before(_) | Stop::After(..) => {}
            other @ Stop::Request(_) => panic!("expected terminal: {other:?}"),
        }
    }
}

#[test]
fn inactive_and_finished_runs_keep_observations_unchanged() {
    let mut machine = Machine::new(None);
    assert!(!machine.abandon());
    for source in ["1 + 2", "fault \"stop\""] {
        let mut machine = Machine::new(None);
        let program = host::program(source);
        assert!(matches!(machine.run(&program), Outcome::Done(_)));
        let lines = machine.lines().to_vec();
        let charged = machine.charged();
        assert!(!machine.abandon());
        assert!(!machine.abandon());
        assert_eq!(machine.lines(), lines);
        assert_eq!(machine.charged(), charged);
    }
}

#[test]
fn a_waiting_direct_run_stops_without_erasing_observations() {
    let program = host::program("1 + 2\ncli.port");
    let mut machine = Machine::new(Some(10_000));
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    let lines = machine.lines().to_vec();
    let charged = machine.charged();
    assert!(!lines.is_empty());
    assert!(machine.abandon());
    assert!(machine.frames().is_empty());
    assert_eq!(machine.lines(), lines);
    assert_eq!(machine.charged(), charged);
    assert_eq!(machine.budget(), Some(10_000));
    assert!(machine.program().is_some());
    assert!(!machine.abandon());
}

#[test]
#[should_panic(expected = "a host request is owed")]
fn an_abandoned_call_has_no_reply_destination() {
    let program = host::program("cli.port");
    let mut machine = Machine::new(None);
    assert!(matches!(machine.run(&program), Outcome::Request(_)));
    assert!(machine.abandon());
    let _ = machine.reply(&program, Reply::Value(Ok(Value::Int(9))));
}

#[test]
fn stepped_abandonment_clears_unstarted_and_queued_active_stops() {
    let program = host::program("cli.port + 1");
    let mut machine = Machine::new(None);
    machine.begin(&program);
    assert!(machine.abandon());
    assert!(catch_unwind(AssertUnwindSafe(|| machine.next_stop())).is_err());
    machine.begin(&program);
    host_stop(&mut machine);
    // Delivery queues an After stop while arithmetic still remains to execute.
    machine.answer(Reply::Value(Ok(Value::Int(7))));
    assert!(machine.abandon());
    assert!(machine.frames().is_empty());
    assert!(catch_unwind(AssertUnwindSafe(|| machine.next_stop())).is_err());
    machine.begin(&program);
    host_stop(&mut machine);
    machine.answer(Reply::Value(Ok(Value::Int(8))));
    assert_eq!(terminal(&mut machine), Ok(Value::Int(9)));
}

#[test]
fn finished_success_and_queued_fault_are_not_abandonment() {
    let program = host::program("text.trim");
    for reply in [Ok(Value::text("trimmed")), Err(String::from("denied"))] {
        let mut machine = Machine::new(None);
        machine
            .begin_with_input(&program, Value::text("  input  "))
            .unwrap();
        host_stop(&mut machine);
        machine.answer(Reply::Value(reply.clone()));
        // A failed reply queues Done immediately; success still runs its wrapper.
        if reply.is_err() {
            assert!(!machine.abandon());
        }
        let result = terminal(&mut machine);
        assert!(!machine.abandon());
        match reply {
            Ok(value) => assert_eq!(result, Ok(value)),
            Err(message) => {
                assert_eq!(result.unwrap_err().reason, Reason::Host(message));
            }
        }
        assert!(catch_unwind(AssertUnwindSafe(|| machine.next_stop())).is_err());
    }
}

#[test]
fn input_misfit_terminal_cannot_restart_main_after_delivery() {
    let program = host::program("cli.port\nadd = a b => a + b\nadd 2");
    let mut machine = Machine::new(None);
    machine
        .begin_with_input(&program, Value::text("wrong shape"))
        .unwrap();
    assert!(!machine.abandon());
    let Stop::Done(Err(fault)) = machine.next_stop() else {
        panic!("misfit is terminal before cli.port can run")
    };
    assert!(matches!(fault.reason, Reason::Host(_)));
    assert_eq!(machine.charged(), 0);
    assert!(catch_unwind(AssertUnwindSafe(|| machine.next_stop())).is_err());
    assert_eq!(machine.charged(), 0);
}

#[test]
fn withdrawal_preserves_prior_closures_and_driver_rejects_old_reply_epochs() {
    let mut session = Session::new(host::declarations());
    let mut machine = Machine::new(None);
    let first =
        program(&checked(&mut session, "offset = 40\nf = n => n + offset"));
    assert_eq!(machine.run(&first), Outcome::Done(Ok(Value::Unit)));
    let abandoned = checked(&mut session, "provisional = f 1\ncli.port");
    let pending = program(&abandoned);
    assert!(matches!(machine.run(&pending), Outcome::Request(_)));
    let mut active_epoch = Some(1_u64);
    let late_reply = (
        active_epoch.take().unwrap(),
        Reply::Value(Ok(Value::Int(900))),
    );
    assert!(
        active_epoch.is_none(),
        "invalidate caller association before abandon"
    );
    assert!(machine.abandon());
    let withdrawn = session.withdraw(abandoned.accepted.unwrap()).unwrap();
    machine.withdraw(&withdrawn);
    let Outcome::Done(query) = session.query("provisional") else {
        panic!("query needs no source")
    };
    assert!(!query.diagnostics.is_empty());
    let replacement = program(&checked(&mut session, "f cli.port"));
    active_epoch = Some(2);
    assert!(matches!(machine.run(&replacement), Outcome::Request(_)));
    let charged = machine.charged();
    // The direct VM has no reply IDs: this filter belongs to the driver.
    let admitted =
        Some(late_reply).filter(|(epoch, _)| Some(*epoch) == active_epoch);
    assert!(admitted.is_none());
    assert_eq!(machine.charged(), charged);
    assert_eq!(
        machine.reply(&replacement, Reply::Value(Ok(Value::Int(2)))),
        Outcome::Done(Ok(Value::Int(42)))
    );
}

fn loaded(session: &mut Session, name: &str) -> Report {
    let Outcome::Request(Request::File { stem }) =
        session.line(&format!("use {name}"))
    else {
        panic!("source requested")
    };
    assert_eq!(stem, name);
    let Outcome::Done(report) = session.reply(Reply::File(Some(String::from(
        "use cli\npub value = cli.port",
    )))) else {
        panic!("complete module")
    };
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    report
}

#[test]
fn withdrawal_after_abandoned_module_init_keeps_prior_initialization() {
    let mut session = Session::new(host::declarations());
    let mut machine = Machine::new(None);
    let first = program(&loaded(&mut session, "prior"));
    assert!(matches!(machine.run(&first), Outcome::Request(_)));
    assert_eq!(
        machine.reply(&first, Reply::Value(Ok(Value::Int(10)))),
        Outcome::Done(Ok(Value::Unit))
    );
    let accepted = loaded(&mut session, "later");
    let second = program(&accepted);
    assert!(matches!(machine.run(&second), Outcome::Request(_)));
    assert!(machine.abandon());
    machine.withdraw(&session.withdraw(accepted.accepted.unwrap()).unwrap());
    let third = program(&checked(&mut session, "use prior\nprior.value"));
    assert_eq!(machine.run(&third), Outcome::Done(Ok(Value::Int(10))));
}
