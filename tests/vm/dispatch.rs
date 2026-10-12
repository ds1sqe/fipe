use super::run::{session, shown};

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

#[test]
fn an_impl_value_member_is_computed_on_its_first_read() {
    let source =
        "type P { x: Integer }\nimpl P { origin = Self 1 }\nP.origin.x";
    assert_eq!(shown(source), "1");
    let source = "type P { x: Integer }\nimpl P { origin = Self 2 }\n(P.origin.x, P.origin.x + 1)";
    assert_eq!(shown(source), "(2, 3)");
}

#[test]
fn a_member_of_a_packed_value_runs_its_tag_s_row_on_the_payload() {
    let shapes = "type Shape where self.area: F64\n\
                  type C: Shape { r: F64 }\n\
                  impl C: Shape { area = self => self.r * 2.0 }\n";
    assert_eq!(shown(&format!("{shapes}x: Shape = C 2.0\nx.area")), "4.0");
    assert_eq!(
        shown(&format!("{shapes}g: Shape -> F64 = s => s.area\ng (C 1.5)")),
        "3.0"
    );
    // A later line reaches the value through `Shape` alone.
    assert_eq!(
        session(&[
            "type Shape where self.area: F64",
            "type C: Shape { r: F64 }",
            "impl C: Shape { area = self => self.r }",
            "x: Shape = C 2.0",
            "x.area",
            "type Named: Shape where 0.0 < self.area",
            "n: Named = x",
            "n.area",
        ]),
        ["()", "()", "()", "()", "2.0", "()", "()", "2.0"]
    );
}

#[test]
fn packed_values_compare_tags_first_then_values() {
    let keys = "type Key: Eq & Show\n\
                type C: Key { n: Integer }\n\
                type D: Key { m: Integer }\n\
                impl C: Show { show = self => \"c\" ++ self.n.show }\n\
                k: Key = C 1\n\
                j: Key = C 1\n\
                l: Key = C 2\n\
                d: Key = D 1\n";
    assert_eq!(
        shown(&format!("{keys}(k == j, k == l, k == d, d == d, k != d)")),
        "(True, False, False, True, True)"
    );
    assert_eq!(
        shown(&format!("{keys}(k.show, [k, d] | show)")),
        "(\"c1\", \"[c1, D 1]\")"
    );
}
