//! Faults: `fault` ends the run without a value, an `Err` returned is
//! a value, and a Fault names its reason at the node where it happened.
use super::run::{budgeted, program, shown, spanned};
use fipe::value::Reason;

#[test]
fn fault_ends_the_run_with_its_message() {
    assert_eq!(shown("fault \"boom\""), "fault: boom");
    assert_eq!(shown("x = 1\nfault \"stop\"\nx"), "fault: stop");
    let f = "f: Integer -> Integer = n => if n > 0 { n } else { fault \"negative\" }\n";
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
