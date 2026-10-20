//! Faults: `fault` ends the run without a value, an `Err` returned is
//! a value, and a Fault names its reason at the node where it happened.
use super::run::{budgeted, program, session, shown, spanned};
use fipe::value::Reason;

#[test]
fn fault_ends_the_run_with_its_message() {
    assert_eq!(shown("fault \"boom\""), "fault: boom");
    assert_eq!(shown("x = 1\nfault \"stop\"\nx"), "fault: stop");
    let f = "f: Integer -> Integer = \
             n => if n > 0 { n } else { fault \"negative\" }\n";
    assert_eq!(shown(&format!("{f}f 2")), "2");
    assert_eq!(shown(&format!("{f}f -1")), "fault: negative");
}

#[test]
fn an_err_value_is_never_a_fault_by_being_returned() {
    assert_eq!(
        shown("r: Result<Integer, String> = Err \"e\"\nr"),
        "Err \"e\""
    );
}

#[test]
fn a_fault_in_a_skipped_branch_never_runs() {
    assert_eq!(shown("if True { 1 } else { fault \"no\" }"), "1");
    assert_eq!(
        shown("m: Maybe<Integer> = Nothing\nm |: (_ => fault \"no\")"),
        "Nothing"
    );
}

fn fault(source: &str) -> (Reason, String) {
    let fault = budgeted(&program(source), 1 << 20).0.unwrap_err();
    (fault.reason.clone(), spanned(source, &fault).to_owned())
}

#[test]
fn a_zero_divisor_fails_the_division_test_at_the_divisor() {
    let source = "n = [0] | head\n1 / n";
    let program = program(source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "0");
    assert_eq!(
        program.tables.borrow().entries[*entry].label,
        "nonzero divisor"
    );
    assert_eq!(spanned(source, &fault), "n");
}

#[test]
fn a_float_without_a_value_faults() {
    for source in ["1.0 / 0.0", "0.0 / 0.0", "1e308 * 10.0", "sqrt -1.0"] {
        assert_eq!(fault(source).0, Reason::NotFinite, "{source}");
    }
}

#[test]
fn fault_gives_its_message_as_the_reason() {
    assert_eq!(
        fault("fault \"boom\"").0,
        Reason::Message(String::from("boom"))
    );
}

#[test]
fn a_line_whose_run_faults_is_withdrawn_and_its_name_given_again() {
    let lines = session(&[
        "stop = n => fault \"stop\"",
        "x: Integer = stop 1",
        "y = 2",
        "x = 3",
        "x + 1",
        "y",
    ]);
    assert_eq!(lines[1], "fault: stop");
    assert_eq!(lines[4], "4");
    assert_eq!(lines[5], "2");
}

#[test]
fn a_divisor_known_only_to_a_range_with_zero_is_tested_at_run() {
    let source = "type Digit: Integer where 0 <= self, self <= 5\n\
                  div: Integer -> Digit -> Integer = x y => x / y\n\
                  (div 10 2, div 10 0)";
    let program = program(source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    let Reason::Failed { entry, value } = &fault.reason else {
        panic!("{fault:?}")
    };
    assert_eq!(value, "0");
    assert_eq!(
        program.tables.borrow().entries[*entry].label,
        "nonzero divisor"
    );
    assert_eq!(spanned(source, &fault), "y");
    let source = "type Digit: Integer where 0 <= self, self <= 5\n\
                  div: Integer -> Digit -> Integer = x y => x / y\n\
                  div 10 2";
    assert_eq!(shown(source), "5");
}

#[test]
fn a_float_division_by_zero_is_not_finite_on_every_path() {
    for source in [
        "1.0 / 0.0",
        "f = n => n / 0.0\nf 1.0",
        "f: F64 -> F64 = n => n / 0.0\nf 1.0",
    ] {
        assert_eq!(
            shown(source),
            "fault: the result is not finite",
            "{source}"
        );
    }
    // A generic divisor that is an integer keeps its test.
    let source = "f = n => 1 / n\nf 0";
    let program = program(source);
    let fault = budgeted(&program, 1 << 20).0.unwrap_err();
    assert!(matches!(fault.reason, Reason::Failed { .. }), "{fault:?}");
}

#[test]
fn an_inexact_number_names_the_value_and_the_target() {
    assert_eq!(
        shown("x: F64 = 2.5\nn: U64 = x\nn"),
        "fault: 2.5 is not an exact Integer"
    );
    assert_eq!(
        shown("big = _ => 9007199254740993\nf: Integer -> F64 = n => n\nf (big ())"),
        "fault: 9007199254740993 is not an exact F64"
    );
}
