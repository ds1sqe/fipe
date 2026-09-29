//! Member obligations: a dot on a head whose type is not yet known is
//! solved once it is, carried by the scheme until then, and
//! `DotNeedsType` where its variable leaves the type. A `.name`
//! selector is the same dot on its parameter.

use super::{errors, kinds, run, ty};
use fipe::{infer::Reason, token::Source};

const OLDER: &str = "older = days e => e.age > days * 86_400";

#[test]
fn a_scheme_carries_the_member_to_its_use() {
    let scheme = ty(OLDER);
    assert_eq!(scheme, "b.age: a, Num a, Ord a => a -> b -> Bool");
    let text = format!("{OLDER}\nls -l /tmp | filter (older 7)");
    assert_eq!(ty(&text), "Vec<Entry>");
    // Never used: nothing leaves, nothing fails.
    assert!(run("h = e => e.x").1.is_ok());
    assert!(run("k = e => e.name\nk").1.is_ok());
}

#[test]
fn a_member_solves_field_first_then_self_function() {
    let point = "type P { x: i64 }\nimpl P { dist = self q => self.x - q.x }\n";
    assert_eq!(ty(&format!("{point}(P 1).dist (P 2)")), "i64");
    let text = "g = e => e.nope\ng (ls | head)";
    let [error] = errors(text).try_into().unwrap();
    let Reason::NoField { other, .. } = error.reason else {
        panic!("{:?}", error.reason)
    };
    let source = Source::new(text);
    assert_eq!(source.slice(error.span), Some("e.nope"));
    assert_eq!(source.slice(other), Some("g"));
}

#[test]
fn a_member_whose_head_leaves_needs_a_type() {
    let text = "(f => 1) (x => x.name)";
    assert_eq!(kinds(text), [("DotNeedsType", "x.name")]);
    // `Eq` alone does not default where its variable leaves.
    assert_eq!(kinds("[] == []"), [("Ambiguous", "[] == []")]);
}

#[test]
fn a_selector_is_a_member_on_its_parameter() {
    assert_eq!(ty("s = .name"), "a.name: b => a -> b");
    assert_eq!(ty("s = .name\nls | map s"), "Vec<String>");
    assert_eq!(ty("ls | map .name"), "Vec<String>");
    assert_eq!(kinds("(f => 1) .name"), [("DotNeedsType", ".name")]);
}
