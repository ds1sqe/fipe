use super::{errors, run, ty};
use fipe::infer::Reason;
use std::fmt::Write;

#[test]
fn plain_members_do_not_grant_claims() {
    let plain = "type V { x: F64 }\nimpl V { add = other self => Self (self.x + other.x) }\n";
    assert_eq!(ty(&format!("{plain}(V 1).add (V 2)")), "V");
    assert!(errors(&format!("{plain}(V 1) + (V 2)"))
        .iter()
        .any(|e| matches!(e.reason, Reason::MissingClaim { .. })));
    let claimed = "type V: Add { x: F64 }\nimpl V: Add { add = other self => Self (self.x + other.x) }\n";
    assert_eq!(ty(&format!("{claimed}(V 1) + (V 2)")), "V");
    assert_eq!(ty("add = a b => a + b"), "Add<b> a => a -> b -> a");
    assert_eq!(
        ty("older = d e => e.age > d"),
        "b.age: a, Ord a => a -> b -> Bool"
    );
}

#[test]
fn blocks_must_match_the_listed_claim() {
    for source in [
        "type V: Zero { x: F64 }",
        "type V: Integer { x: F64 }",
        "type V: Show { x: F64 }\nimpl V { show = self => \"own\" }",
        "type V: Zero { x: F64 }\nimpl V: Zero {}",
        "type V { x: F64 }\nimpl V: Zero { zero = Self 0.0 }",
        "type V: Zero { x: F64 }\nimpl V: Zero { extra = Self 0.0 }",
    ] {
        assert!(
            errors(source)
                .iter()
                .any(|e| matches!(e.reason, Reason::ClaimBlock { .. })),
            "{source}"
        );
    }
    assert!(
        run("type V: Zero { x: F64 }\nimpl V: Zero { zero = Self 0.0 }")
            .1
            .is_ok()
    );
}

#[test]
fn packed_places_require_the_named_claim() {
    let plain = "type Shape where self.area: F64\ntype C { x: F64 }\nimpl C { area = self => self.x }\nxs: Vec<Shape> = [C 1]";
    assert!(errors(plain)
        .iter()
        .any(|e| matches!(e.reason, Reason::MissingClaim { .. })));
    let claimed = plain
        .replace("type C {", "type C: Shape {")
        .replace("impl C {", "impl C: Shape {");
    assert_eq!(ty(&claimed), "Vec<Shape>");
}

#[test]
fn claims_control_derived_members_and_defaults() {
    assert_eq!(ty("type V: Default { x: F64 = 0.0 }\nV.default"), "V");
    assert!(errors("type V: Default { x: F64 }")
        .iter()
        .any(|e| matches!(e.reason, Reason::ClaimBlock { .. })));
    assert_eq!(ty("type V: Eq & Show { x: F64 }\nshow (V 1)"), "String");
    assert!(errors("type V { x: F64 }\nshow (V 1)")
        .iter()
        .any(|e| matches!(e.reason, Reason::MissingClaim { .. })));
}

#[test]
fn a_right_operand_selects_a_claim_and_unknown_names_candidates() {
    let source = "type V: Mul<F64> & Mul<V> { x: F64 }\nimpl V: Mul<F64> { mul = k self => Self (self.x * k) }\nimpl V: Mul<V> { mul = other self => Self (self.x * other.x) }\nv = V 1\n";
    assert_eq!(ty(&format!("{source}k: F64 = 2\nv * k")), "V");
    assert_eq!(ty(&format!("{source}v * v")), "V");
    for call in ["v.mul v", "V.mul v v", ".mul v v", "v.mul 2.0"] {
        assert_eq!(ty(&format!("{source}{call}")), "V");
    }
    assert!(errors(&format!("{source}f = v.mul"))
        .iter()
        .any(|e| matches!(e.reason, Reason::Conversion { .. })));

    assert!(errors(&format!("{source}f = k => v * k")).iter().any(|e| matches!(&e.reason,Reason::Conversion { candidates, .. } if candidates.len() == 2)));
}

#[test]
fn supplied_claim_member_types_are_checked_at_load() {
    let mut declarations = fipe::decl::Declarations::new();
    assert!(declarations.commands("pub type Bad\nimpl Bad: Add { add: Self -> (self: Self) -> String }\npub bad: Bad").is_err());
}

#[test]
fn declared_parameter_bounds_require_claims() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.commands("pub type Box<T: Ord>\npub type C\nimpl C { eq: Self -> (self: Self) -> Bool, cmp: Self -> (self: Self) -> Ordering }\npub boxed: Box<C>").unwrap();
    let module = super::module("boxed");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    assert!(fipe::infer::infer(&module, &resolved, &declarations, &[])
        .unwrap_err()
        .iter()
        .any(|e| matches!(e.reason, Reason::MissingClaim { .. })));
}

#[test]
fn combinations_and_duplicate_blocks_are_checked() {
    assert_eq!(
        ty("type Key: Eq & Show\ntype V: Key { x: F64 }\nshow (V 1)"),
        "String"
    );
    assert!(errors("type V: Zero { x: F64 }\nimpl V: Zero { zero = Self 0 }\nimpl V: Zero { zero = Self 1 }").iter().any(|e| matches!(e.reason,Reason::ClaimBlock { .. })));
    assert!(errors("type A where self.m: Integer\ntype B where self.m: String\ntype V: A & B {}\nimpl V: A { m = self => 1 }\nimpl V: B { m = self => \"s\" }").iter().any(|e| matches!(e.reason,Reason::ClaimBlock { .. })));
}

#[test]
fn arrays_derive_from_parts_and_claims_plan_narrow_parameters() {
    assert_eq!(
        ty("f: [Integer; 2] -> Bool = a => a == a"),
        "[Integer; 2] -> Bool"
    );
    assert_eq!(
        ty("f: [Integer; 2] -> String = a => a.show"),
        "[Integer; 2] -> String"
    );
    // A member parameter narrower than the limit's is a check error.
    let errors = super::run("type Limit where self.take: Integer -> Integer\ntype V: Limit {}\nimpl V: Limit { take: U8 -> Self -> Integer = n self => n }").1.unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| matches!(&e.reason, Reason::Limit { message }
            if message.contains("unproved function entry"))),
        "{errors:?}"
    );
}

#[test]
fn imported_claim_names_keep_their_declaration_identity() {
    let declarations = fipe::decl::Declarations::new();
    let files = [
        ("limits".into(),super::module("pub type Shape where self.area: F64")),
        ("shapes".into(),super::module("use limits.Shape\npub type C: Shape { pub x: F64 }\nimpl C: Shape { pub area = self => self.x }")),
    ];
    let module =
        super::module("use shapes.C\nuse limits.Shape\ns: Shape = C 1");
    let resolved =
        fipe::resolve::resolve(&module, &declarations, &files).unwrap();
    let result = fipe::infer::infer(&module, &resolved, &declarations, &files);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn inherited_claim_members_can_be_derived_or_written_in_part_blocks() {
    for claims in ["Ord", "Eq & Ord"] {
        for eq in ["", "impl W: Eq { eq = other self => True }\n"] {
            let source = format!("type W: {claims} {{ x: F64 }}\n{eq}impl W: Ord {{ cmp = other self => Equal }}\n(W 1) == (W 2)");
            assert_eq!(ty(&source), "Bool");
        }
    }
    assert_eq!(ty("type Both: Zero & Show\ntype W: Both { x: F64 }\nimpl W: Zero { zero = Self 0 }\nW.zero.show"), "String");
    assert!(errors("type W: Ord { x: F64 }\nimpl W: Eq { eq = other self => True }\nimpl W: Ord { eq = other self => True, cmp = other self => Equal }").iter().any(|e| matches!(e.reason, Reason::ClaimBlock { .. })));
    let mut number = String::from("type W: Number { x: F64 }\n");
    for (class, member) in [
        ("Add", "add"),
        ("Sub", "sub"),
        ("Mul", "mul"),
        ("Div", "div"),
        ("Rem", "rem"),
    ] {
        writeln!(
            number,
            "impl W: {class} {{ {member} = other self => self }}"
        )
        .unwrap();
    }
    number.push_str("impl W: Literal { literal = form => Self 0.0 }\nimpl W: Number {}\n(W 1) + (W 2)");
    assert_eq!(ty(&number), "W");
    let mut declarations = fipe::decl::Declarations::new();
    declarations.commands("pub type W: Ord { x: F64 }\nimpl W: Ord { cmp: Self -> (self: Self) -> Ordering }").unwrap();
    declarations.commands("pub type W: Ord { x: F64 }\nimpl W: Eq { eq: Self -> (self: Self) -> Bool }\nimpl W: Ord { cmp: Self -> (self: Self) -> Ordering }").unwrap();
}
