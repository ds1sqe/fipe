//! The conformance host's sink policy: writes to `out` are the success
//! output, writes to any other sink are files a later `cat` reads,
//! each expression line's value goes to the success output and an `Err`
//! or a Fault to the error output.
use super::{observe, program};
use fipe::{embed::Request, value::Value};

fn run(source: &str) -> fipe::vm::Observation {
    observe(&program(source, &[]), None, source).0
}

#[test]
fn a_write_to_out_is_the_success_output() {
    let observation = run("put out \"hello\\n\"");
    assert_eq!(observation.result, Ok(Value::Unit));
    assert_eq!(observation.success, b"hello\n");
    assert_eq!(observation.error, b"");
}

#[test]
fn a_file_put_then_appended_is_read_back_by_cat() {
    let observation = run("put ./f \"ab\"\nappend ./f \"cd\"\ncat ./f");
    assert_eq!(observation.result, Ok(Value::text("abcd")));
    assert_eq!(observation.success, b"\"abcd\"\n");
    // A later put replaces the file.
    let observation = run("put ./f \"ab\"\nput ./f \"x\"\ncat ./f");
    assert_eq!(observation.result, Ok(Value::text("x")));
}

#[test]
fn the_runs_value_is_written_and_an_err_goes_to_the_error_output() {
    let observation = run("1 + 2");
    assert_eq!(observation.success, b"3\n");
    let observation = run("x: Result<Integer, String> = Err \"bad\"\nx");
    assert_eq!(observation.success, b"");
    assert_eq!(observation.error, b"\"bad\"\n");
    let observation = run("x: Maybe<Integer> = Just 4\nx");
    assert_eq!(observation.success, b"4\n");
    let observation = run("x: Maybe<Integer> = Nothing\nx");
    assert_eq!(observation.success, b"");
}

#[test]
fn each_expression_line_is_written_in_order_between_writes_to_out() {
    let observation =
        run("1 + 1\nput out \"mid\\n\"\nx = 5\nx * 2\nNothing\n()");
    assert_eq!(observation.success, b"2\nmid\n10\n");
    assert_eq!(observation.error, b"");
}

#[test]
fn a_fault_is_written_located_to_the_error_output() {
    let observation = run("a = 1\n1.0 / 0.0");
    assert!(observation.result.is_err());
    assert_eq!(observation.success, b"");
    assert_eq!(
        String::from_utf8_lossy(&observation.error),
        "fault in main at 2:1: the result is not finite\n"
    );
}

#[test]
fn the_files_example_writes_its_big_logs_by_put() {
    let source = super::examples::EXAMPLES[0].1;
    let program = program(source, &[("geometry", super::examples::GEOMETRY)]);
    let (observation, _) = observe(&program, None, source);
    let tables = program.tables.borrow();
    let written: Vec<_> = observation
        .trace
        .iter()
        .filter_map(|(request, _)| {
            let Request::Call {
                declaration,
                arguments,
                ..
            } = request
            else {
                return None;
            };
            (tables.declarations[*declaration].label == "put").then(|| {
                let Value::Seq(bytes) = &arguments[1] else {
                    panic!("put takes bytes")
                };
                bytes
                    .iter()
                    .map(|byte| match byte {
                        Value::Int(byte) => u8::try_from(*byte).unwrap(),
                        other => panic!("{other:?}"),
                    })
                    .collect::<Vec<u8>>()
            })
        })
        .collect();
    assert_eq!(written, [b"[\"syslog\", \"kern.log\"]".to_vec()]);
    // Each expression line's value, in order: the largest logs' names,
    // the configuration paths and the two counts of the notes; the lines
    // ending in `put` and `append` give `()`, which writes nothing.
    assert_eq!(
        observation.success,
        b"[\"syslog\", \"kern.log\"]\n[\"/etc/hosts\", \"/etc/fipe.conf\"]\n3\n3\n"
    );
}
