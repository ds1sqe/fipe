use super::{errors, kinds, ty};
use fipe::{infer::Reason, token::Source};

#[test]
fn unannotated_conversion_uses_claim_signature() {
    assert_eq!(ty("type Polar { r: F64, t: F64 }\ntype Point: From<Polar> { x: F64, y: F64 }\nimpl Point: From<Polar> { from = p => Self p.r p.t }\nPoint.from (Polar 1.0 2.0)"), "Point");
}

#[test]
fn claim_members_have_the_required_receiver_and_type() {
    let prefix =
        "type M: Add { c: Integer }\nimpl M: Add { add = o self => o }\n";
    assert_eq!(ty(&format!("{prefix}f = M.add")), "M -> M -> M");
    assert_ne!(errors(&format!("{prefix}(M 1).add \"s\"")), []);
    for source in [
        "type M: Add { c: Integer }\nimpl M: Add { add = o => o }",
        "type Z: Zero { c: Integer }\nimpl Z: Zero { zero = self => self }",
    ] {
        assert!(super::run(source).1.is_err(), "{source}");
    }
}

const OVERLOADED: &str = "type W { n: F64 }\ntype V: Mul<F64> & Mul<W> { x: F64 }\nimpl V: Mul<F64> { mul = k self => Self (self.x * k) }\nimpl V: Mul<W> { mul = w self => Self (self.x * w.n) }\nv = V 1.0\nk = 2.0\n";

#[test]
fn overloads_wait_for_the_argument() {
    for expression in [
        "scale = a b => a * b\nscale v k",
        "(v | .mul) k",
        "g = a k => a.mul k\ng v k",
    ] {
        assert_eq!(ty(&format!("{OVERLOADED}{expression}")), "V");
    }
}

#[test]
fn combinations_are_structural() {
    assert_eq!(ty("type Key: Eq & Show\nshow_key: Key -> String = k => k.show\nshow_key 1"), "String");
}

#[test]
fn packs_claim_their_eligible_limits() {
    assert_eq!(
        ty("same: Eq -> Eq -> Bool = a b => a == b"),
        "Eq -> Eq -> Bool"
    );
    assert_eq!(
        ty("xs: Vec<Show> = [1, \"a\"]\nxs | map show"),
        "Vec<String>"
    );
}

#[test]
fn member_failure_names_member_at_argument() {
    let text = "big = s => s.area > 10.0\nbig 1";
    let found = errors(text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(Source::new(text).slice(found[0].span), Some("1"));
    assert!(
        matches!(&found[0].reason, Reason::MissingMember { ty, name } if name == "area" && ty.to_string() == "Integer")
    );
}

#[test]
fn one_diagnostic_per_cause() {
    let text = "type V { x: F64 }\nv = V 1.0\ninc = n => n + 1\ninc v";
    let found = errors(text);
    let literals = found.iter().filter(|e| matches!(&e.reason, Reason::MissingClaim { claim, .. } if claim.to_string()=="Literal")).count();
    assert_eq!(literals, 1, "{found:?}");
    let text = "type V: Zero {}\nimpl V: Zero { zero = Self {} }\nimpl V: Zero { zero = Self {} }";
    assert_eq!(kinds(text).len(), 1);
    let repeated = errors("gen = s => s.area + s.area\ngen 1");
    assert_eq!(repeated.iter().filter(|e| matches!(&e.reason, Reason::MissingMember { ty, name } if name == "area" && ty.to_string() == "Integer")).count(), 1);
}

#[test]
fn single_base_without_limit_is_not_an_alias() {
    assert_ne!(errors("type N: Integer"), []);
}

#[test]
fn conversion_errors_keep_known_source_and_unknown_target() {
    let found = errors("1 | to<F64>");
    assert!(found.iter().any(|e| matches!(&e.reason, Reason::Conversion { source: Some(source), target, .. } if source.to_string()=="Integer" && target.to_string()=="F64")));
    let found = errors("to 1");
    assert!(found.iter().any(|e| matches!(
        &e.reason,
        Reason::Conversion {
            target: fipe::types::Type::Var(_),
            ..
        }
    )));
}

#[test]
fn missing_operator_member_is_named() {
    assert!(errors("1 ++ 2").iter().any(|e| matches!(&e.reason, Reason::MissingMember { ty, name } if name == "concat" && ty.to_string() == "Integer")));
}
