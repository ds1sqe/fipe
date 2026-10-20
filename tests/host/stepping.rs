//! A stepped run: stops before each statement and after each write,
//! with the same observation and charges as an unstepped run.
use super::{observe, observe_stepped, program};
use fipe::{
    ir::Kind,
    value::{structural, Reason},
    vm::Stop,
};

const OUTCOMES: &str = "use fs
default_config = \"port = 80\"
find PATH |: cat |! log \"no config\" || default_config
";

/// The value after each marked pipe and `||` of the line, in order,
/// with the mark: `|:`, `|!` or `||`.
fn pipes(path: &str) -> Vec<(&'static str, String)> {
    let source = OUTCOMES.replace("PATH", path);
    let program = program(&source, &[]);
    let (_, _, stops) = observe_stepped(&program, None);
    let tables = program.tables.borrow();
    stops
        .iter()
        .filter_map(|stop| {
            let Stop::After(node, value) = stop else {
                return None;
            };
            let mark = match tables.nodes[*node].kind {
                Kind::Marked { good: true, .. } => "|:",
                Kind::Marked { good: false, .. } => "|!",
                Kind::Or { .. } => "||",
                _ => return None,
            };
            Some((mark, structural::show(value, &tables)))
        })
        .collect()
}

#[test]
fn a_stepped_run_stops_after_each_marked_pipe_on_a_hit() {
    let file = "\"port = 8080\\n\"";
    assert_eq!(
        pipes("./config.toml"),
        [
            ("|:", format!("Just {file}")),
            ("|!", format!("Just {file}")),
            ("||", String::from(file)),
        ]
    );
}

#[test]
fn a_stepped_run_stops_after_each_marked_pipe_on_a_miss() {
    assert_eq!(
        pipes("./missing.toml"),
        [
            ("|:", String::from("Nothing")),
            ("|!", String::from("Nothing")),
            ("||", String::from("\"port = 80\"")),
        ]
    );
}

#[test]
fn a_stepped_run_stops_before_each_node_it_runs_and_after_its_writes() {
    let program = program("f = n => n + 1\nf 41", &[]);
    let (observation, _, stops) = observe_stepped(&program, None);
    assert_eq!(
        observation.result.map(|value| format!("{value:?}")).ok(),
        Some(String::from("Int(42)"))
    );
    let tables = program.tables.borrow();
    let befores = stops
        .iter()
        .filter(|stop| matches!(stop, Stop::Before(_)))
        .count();
    assert!(befores > 2, "{stops:?}");
    // The call's result lands in the main body after `f 41`.
    let call = stops.iter().rev().find_map(|stop| match stop {
        Stop::After(node, value)
            if matches!(tables.nodes[*node].kind, Kind::Call { .. }) =>
        {
            Some(structural::show(value, &tables))
        }
        _ => None,
    });
    assert_eq!(call.as_deref(), Some("42"));
    assert!(matches!(stops.last(), Some(Stop::Done(Ok(_)))));
}

#[test]
fn a_stepped_run_observes_and_charges_what_a_run_does() {
    for (name, source) in super::examples::EXAMPLES {
        let program =
            program(source, &[("geometry", super::examples::GEOMETRY)]);
        let (run, charged) = observe(&program, None);
        let (stepped, stepped_charged, _) = observe_stepped(&program, None);
        assert_eq!(stepped, run, "{name}");
        assert_eq!(stepped_charged, charged, "{name}");
    }
}

#[test]
fn a_stepped_run_under_a_budget_faults_where_a_run_does() {
    let source = super::examples::EXAMPLES[0].1;
    let program = program(source, &[("geometry", super::examples::GEOMETRY)]);
    let (_, charged) = observe(&program, None);
    let (run, _) = observe(&program, Some(charged / 2));
    let (stepped, _, _) = observe_stepped(&program, Some(charged / 2));
    assert_eq!(stepped, run);
    assert_eq!(
        stepped.result.expect_err("the budget stops it").reason,
        Reason::Budget
    );
}
