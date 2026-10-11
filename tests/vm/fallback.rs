//! `||` by its recorded mode: `Bool`, the same family, or the good
//! payload.
use super::run::shown;

#[test]
fn a_payload_fallback_unwraps_the_good_side() {
    assert_eq!(shown("Nothing || 4"), "4");
    assert_eq!(shown("Just 3 || 4"), "3");
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    let ok = "r: Result<Integer, String> = Ok 2\n";
    assert_eq!(shown(&format!("{err}r || 0")), "0");
    assert_eq!(shown(&format!("{ok}r || 0")), "2");
}

#[test]
fn a_family_fallback_keeps_the_original_left() {
    assert_eq!(shown("Nothing || Just 4"), "Just 4");
    assert_eq!(shown("Just 3 || Just 4"), "Just 3");
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    assert_eq!(shown(&format!("{err}r || Ok 9")), "Ok 9");
}

#[test]
fn bool_fallback_short_circuits() {
    assert_eq!(shown("(False || True, False || False)"), "(True, False)");
    assert_eq!(shown("True || fault \"x\""), "True");
}

#[test]
fn an_unneeded_right_side_is_not_evaluated() {
    // `fault` fits any type, so the same family is the reading.
    assert_eq!(shown("Just 3 || fault \"unreadable\""), "Just 3");
    assert_eq!(shown("Just 3 || 0 + (fault \"unreadable\")"), "3");
    assert_eq!(
        shown("Nothing || fault \"unreadable\""),
        "fault: unreadable"
    );
}
