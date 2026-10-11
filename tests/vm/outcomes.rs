//! Marked pipes and `&&` at run time: the five rows of the marked-pipe
//! rule on `Maybe`, `Result` and `Either`, and skipped sides.
use super::run::{charged, shown};

#[test]
fn a_same_family_return_is_used_directly_not_nested() {
    assert_eq!(shown("Just 1 |: (_ => Just 2)"), "Just 2");
    assert_eq!(shown("Just 1 |: (n => n + 1)"), "Just 2");
}

#[test]
fn the_inactive_side_keeps_its_value_and_the_stage_is_not_called() {
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    let ok = "r: Result<Integer, String> = Ok 1\n";
    assert_eq!(shown(&format!("{err}r |: (n => n + 1)")), "Err \"e\"");
    assert_eq!(shown(&format!("{ok}r |! (e => e ++ \"!\")")), "Ok 1");
    assert_eq!(shown(&format!("{err}r |: (_ => fault \"x\")")), "Err \"e\"");
    assert_eq!(
        shown("m: Maybe<Integer> = Nothing\nm |: (n => n + 1)"),
        "Nothing"
    );
}

#[test]
fn the_marked_side_is_rebuilt_from_the_stage() {
    let err = "r: Result<Integer, String> = Err \"e\"\n";
    let ok = "r: Result<Integer, String> = Ok 1\n";
    assert_eq!(shown(&format!("{ok}r |: (n => n + 1)")), "Ok 2");
    assert_eq!(shown(&format!("{err}r |! (e => e ++ \"!\")")), "Err \"e!\"");
    assert_eq!(
        shown(&format!("{err}r |! (e => Err (e ++ \"!\"))")),
        "Err \"e!\""
    );
    assert_eq!(shown(&format!("{err}r |! (_ => Ok 0)")), "Ok 0");
    assert_eq!(shown("Err \"e\" |! (e => e ++ \"!\")"), "Err \"e!\"");
}

#[test]
fn maybes_bad_side_takes_a_value_and_discards_a_plain_one() {
    let nothing = "m: Maybe<Integer> = Nothing\n";
    assert_eq!(shown(&format!("{nothing}m |! Just 5")), "Just 5");
    assert_eq!(shown(&format!("{nothing}m |! 5")), "Nothing");
    assert_eq!(shown("Just 1 |! Just 5"), "Just 1");
}

#[test]
fn either_has_left_bad_and_right_good() {
    let left = "e: Either<Integer, String> = Left 1\n";
    let right = "e: Either<Integer, String> = Right \"a\"\n";
    assert_eq!(shown(&format!("{left}e |! (n => n + 1)")), "Left 2");
    assert_eq!(shown(&format!("{left}e |: (s => s ++ \"!\")")), "Left 1");
    assert_eq!(
        shown(&format!("{right}e |: (s => s ++ \"!\")")),
        "Right \"a!\""
    );
    assert_eq!(shown(&format!("{right}e |! (n => n + 1)")), "Right \"a\"");
    assert_eq!(
        shown(&format!("{left}e |! (_ => Right \"b\")")),
        "Right \"b\""
    );
}

#[test]
fn and_is_a_good_side_stage_that_ignores_the_payload() {
    assert_eq!(shown("Just 1 && Just 5"), "Just 5");
    assert_eq!(shown("Just 1 && 5"), "Just 5");
    assert_eq!(shown("m: Maybe<Integer> = Nothing\nm && Just 5"), "Nothing");
    assert_eq!(
        shown("r: Result<Integer, String> = Err \"e\"\nr && Ok 2"),
        "Err \"e\""
    );
    assert_eq!(shown("(True && False, True && True)"), "(False, True)");
    assert_eq!(shown("False && fault \"x\""), "False");
}

#[test]
fn a_skipped_side_charges_nothing() {
    let nothing = "m: Maybe<Integer> = Nothing\n";
    assert_eq!(
        charged(&format!("{nothing}m |: (n => n + 1 + 2 + 3 + 4)")),
        charged(&format!("{nothing}m |: (n => n)"))
    );
    assert_eq!(
        charged("False && (1 + 2 + 3 + 4 == 10)"),
        charged("False && True")
    );
    assert_eq!(charged("Just 1 || 1 + 2 + 3 + 4"), charged("Just 1 || 1"));
    assert!(charged("True && (1 + 2 + 3 + 4 == 10)") > charged("True && True"));
    assert!(
        charged("Just 1 |: (n => n + 1 + 2)") > charged("Just 1 |: (n => n)")
    );
}
