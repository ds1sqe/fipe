use super::run::shown;

#[test]
fn a_written_eq_runs_as_a_body_by_the_value_tag() {
    let source = "type Mask: Eq { bits: Integer }\n\
                  impl Mask: Eq { eq = self other => (other.bits % 10) == (self.bits % 10) }\n\
                  ((Mask 1) == (Mask 11), (Mask 1) == (Mask 2))";
    assert_eq!(shown(source), "(True, False)");
}

#[test]
fn show_is_structural_or_the_written_member() {
    assert_eq!(
        shown("type Point: Show { x: Integer, y: Integer }\nshow (Point 1 2)"),
        "\"Point 1 2\""
    );
    // A written member runs its body, spelled as a member or reached
    // through the prelude's word, alone or inside a container.
    let written = "type Point: Show { x: Integer, y: Integer }\nimpl Point: Show { show = self => show (self.x * 10) }\np = Point 1 2\n";
    assert_eq!(shown(&format!("{written}p.show")), "\"10\"");
    assert_eq!(shown(&format!("{written}show p")), "\"10\"");
    assert_eq!(shown(&format!("{written}show [p, p]")), "\"[10, 10]\"");
    assert_eq!(shown("show (Just 2)"), "\"Just 2\"");
    assert_eq!(shown("show 1.5"), "\"1.5\"");
    assert_eq!(shown("show \"a\""), "\"\\\"a\\\"\"");
}

#[test]
fn conversions_check_their_limits() {
    assert_eq!(shown("1 | to<U8>"), "1");
    assert_eq!(shown("300 | try<U8>"), "Err LimitError");
    assert_eq!(shown("3 | try<U8>"), "Ok 3");
    assert_eq!(
        shown("f: Integer -> U8 = n => n | to<U8>\nf 300"),
        "fault: the cast failed"
    );
    assert_eq!(
        shown("f: Integer -> Result<U8, LimitError> = n => n | try<U8>\nf 300"),
        "Err LimitError"
    );
}

#[test]
fn a_derived_comparison_orders_by_constructor_then_fields() {
    assert_eq!(shown("(Just 1) < (Just 2)"), "True");
    assert_eq!(shown("(Just 1) < Nothing"), "True");
    assert_eq!(shown("(1, \"b\") < (1, \"a\")"), "False");
}
