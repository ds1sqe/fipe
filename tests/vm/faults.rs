//! `fault` ends the run without a value; an `Err` returned is a value.
use super::run::shown;

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
