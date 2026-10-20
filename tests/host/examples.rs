//! Each documentation example, checked as one unit, runs under the
//! conformance host; its trace is the fixture
//! `tests/fixtures/traces/<example>.txt`. `FIPE_BLESS=1` rewrites the
//! fixtures instead of comparing.
use super::{observe, program, trace};
use fipe::{embed::Request, value::Reason};

pub const GEOMETRY: &str =
    include_str!("../../Documentation/examples/geometry.fipe");

pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "01-files",
        include_str!("../../Documentation/examples/01-files.fipe"),
    ),
    (
        "02-outcomes",
        include_str!("../../Documentation/examples/02-outcomes.fipe"),
    ),
    (
        "03-types",
        include_str!("../../Documentation/examples/03-types.fipe"),
    ),
    (
        "04-text",
        include_str!("../../Documentation/examples/04-text.fipe"),
    ),
    (
        "05-modules",
        include_str!("../../Documentation/examples/05-modules.fipe"),
    ),
    ("geometry", GEOMETRY),
];

/// An example's trace, run once, with no budget.
fn traced(source: &str) -> String {
    let program = program(source, &[("geometry", GEOMETRY)]);
    let (observation, _) = observe(&program, None, source);
    let sources = trace::Sources::of(source);
    let tables = program.tables.borrow();
    trace::write(&observation, &tables, &sources)
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/traces")
        .join(format!("{name}.txt"))
}

#[test]
fn each_example_replays_its_trace_byte_for_byte() {
    let bless = std::env::var_os("FIPE_BLESS").is_some();
    for (name, source) in EXAMPLES {
        let first = traced(source);
        if bless {
            std::fs::write(fixture(name), &first).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(fixture(name))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(first, expected, "{name}: the first run");
        assert_eq!(traced(source), expected, "{name}: the replay");
    }
}

#[test]
fn a_smaller_budget_cuts_the_trace_at_the_unpaid_action() {
    let source = EXAMPLES[0].1;
    let program = program(source, &[("geometry", GEOMETRY)]);
    let (full, charged) = observe(&program, None, source);
    assert!(full.result.is_ok(), "{:?}", full.result);
    assert!(full.trace.len() > 2);
    // Half the units: the run stops partway, at a Budget Fault located
    // on the action that could not be paid.
    let (cut, _) = observe(&program, Some(charged / 2), source);
    let fault = cut.result.as_ref().expect_err("the budget stops it");
    assert_eq!(fault.reason, Reason::Budget);
    assert!(cut.trace.len() < full.trace.len());
    // Every call before the stop is the full run's, in order, with the
    // same reply: the budget changes the trace only where it stops.
    assert_eq!(cut.trace[..], full.trace[..cut.trace.len()]);
    // One unit short of the whole run: every call happened, then the
    // run stops before its end.
    let (short, _) = observe(&program, Some(charged - 1), source);
    assert_eq!(short.result.as_ref().unwrap_err().reason, Reason::Budget);
    assert!(short.trace.iter().zip(&full.trace).all(|(a, b)| a == b));
    assert!(matches!(full.trace[0].0, Request::Call { .. }));
}
