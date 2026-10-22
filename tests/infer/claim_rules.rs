use super::{errors, module, ty};
use fipe::{
    decl::Declarations,
    infer::{infer, Reason},
    resolve::resolve,
};
use std::fmt::Write as _;

#[test]
fn supplied_receiver_matches_the_claim() {
    for source in [
        "pub type Bad\nimpl Bad: Zero { zero: (self: Self) -> Self }",
        "pub type Bad\nimpl Bad: Len { len: Natural }",
    ] {
        assert!(Declarations::new().commands(source).is_err(), "{source}");
    }
}

#[test]
fn opaque_claims_supply_every_member() {
    for claim in ["Eq", "Ord", "Show", "Default"] {
        assert!(
            Declarations::new()
                .commands(&format!("pub type T\nimpl T: {claim} {{}}"))
                .is_err(),
            "{claim}"
        );
    }
}

/// Whether a source fails with a limit error whose message holds `text`.
fn limit_error(source: &str, text: &str) -> bool {
    super::run(source).1.is_err_and(|errors| {
        errors.iter().any(|e| {
            matches!(&e.reason, Reason::Limit { message } if message.contains(text))
        })
    })
}

#[test]
fn member_limits_are_met_only_when_proved() {
    // A member parameter narrower than the claim's is not proved; one
    // wider, or the same, is.
    for annotation in ["Vec<U8>", "Maybe<U8>", "(Integer -> U8)"] {
        let required = annotation.replace("U8", "Integer");
        let source = format!("type C: Add<{required}> {{ x: Integer }}\nimpl C: Add<{required}> {{ add: {annotation} -> Self -> Self = k self => self }}");
        assert!(limit_error(&source, "unproved function entry"), "{source}");
        let source = format!("type C: Add<{annotation}> {{ x: Integer }}\nimpl C: Add<{annotation}> {{ add: {required} -> Self -> Self = k self => self }}");
        let typed = super::run(&source).1;
        assert!(typed.is_ok_and(|t| t.checks.is_empty()), "{source}");
    }
    // `len` is refuted by its body: -1 is not Natural.
    assert!(limit_error(
        "type A: Len { x: F64 }\nimpl A: Len { len = self => -1 }",
        "refuted entry"
    ));
    let typed = super::run(
        "type A: Len { n: Natural }\nimpl A: Len { len = self => self.n }",
    )
    .1
    .unwrap();
    assert_eq!(typed.checks, []);
}

#[test]
fn written_combination_is_met_by_its_parts() {
    assert_eq!(
        ty("type Key: Eq & Show\nkf: Key a => a -> Bool = p => p == p\nkf 1"),
        "Bool"
    );
}

#[test]
fn packing_preserves_outer_literal_variables() {
    let source = "g: Show -> String = s => s.show\nh = n => g (n + 1)";
    assert_eq!(ty(&format!("{source}\nh 2.5")), "String");
    assert!(ty(source).contains("-> String"));
}

fn with_files(
    source: &str,
    files: &[(&str, &str)],
) -> Result<fipe::typed::Typed, Vec<fipe::infer::Error>> {
    let files: Vec<_> = files
        .iter()
        .map(|(name, text)| ((*name).into(), module(text)))
        .collect();
    let parsed = module(source);
    let declarations = Declarations::new();
    let resolved = resolve(&parsed, &declarations, &files).unwrap();
    infer(&parsed, &resolved, &declarations, &files)
}

#[test]
fn and_10_claim_members_share_the_types_visibility() {
    let library = "pub type V: Add & From<Integer> & From<String> { pub x: Integer }\nimpl V: Add { add = o self => self }\nimpl V: From<Integer> { pub from = n => Self n }\nimpl V: From<String> { from = s => Self 0 }\npub v = V 1";
    for expression in ["lib.v + lib.v", "lib.v.add lib.v", "\"a\" | to<lib.V>"]
    {
        let source = format!("use lib\n{expression}");
        assert!(with_files(&source, &[("lib", library)]).is_ok(), "{source}");
    }
}

#[test]
fn cyclic_base_is_rejected() {
    assert_ne!(errors("type A: A"), []);
    assert_ne!(
        errors(
            "type A: B where self.a: Integer\ntype B: A where self.b: Integer"
        ),
        []
    );
    assert_ne!(errors("type A: B\ntype B: A"), []);
}

#[test]
fn extra_claim_arguments_are_rejected() {
    assert_ne!(errors("type E: Eq<E, E> { x: F64 }"), []);
    assert_ne!(errors("type A: Add<A, A, A> { x: F64 }\nimpl A: Add<A, A, A> { add = o self => self }"), []);
}

#[test]
fn prefix_names_a_limit_and_an_occurring_variable() {
    for source in [
        "fi: Integer a => a -> a = p => p",
        "fc: Eq a => b -> b = p => p",
    ] {
        assert_ne!(errors(source), [], "{source}");
    }
}

#[test]
fn listed_member_conflicts_fail_at_the_type() {
    let found = with_files(
        "use za zb\ntype V: za.Zero & zb.Zero { x: F64 }",
        &[
            ("za", "pub type Zero where Self.zero: Self"),
            ("zb", "pub type Zero where Self.zero: Integer"),
        ],
    )
    .unwrap_err();
    assert!(found.iter().any(|e| matches!(&e.reason, Reason::ClaimBlock { problem, .. } if problem.contains("conflict"))), "{found:?}");
}

#[test]
fn supplied_target_must_exist() {
    assert!(Declarations::new()
        .commands("impl Nope: Show { show: (self: Self) -> String }")
        .is_err());
}

#[test]
fn defaults_follow_the_table() {
    for source in [
        "type D: Default { s: String }",
        "type D: Default { p: ParseError }",
    ] {
        assert_ne!(errors(source), [], "{source}");
    }
}

#[test]
fn member_limit_chain_does_not_expand_twice() {
    let mut source = String::from("type C0 where self.m0: Integer\n");
    for i in 1..=24 {
        writeln!(source, "type C{i}: C{} where self.m{i}: Integer", i - 1)
            .unwrap();
    }
    source.push_str("f: C24 a => a -> a = p => p");
    assert!(ty(&source).contains("C24"));
}

#[test]
fn imported_field_keeps_its_type() {
    let parsed = module("use shape\nshape.b.p");
    let files = [
        ("geo", "pub type Pt { pub x: I64 }"),
        (
            "shape",
            "use geo\npub type Box { pub p: geo.Pt }\npub b = Box (geo.Pt 1)",
        ),
    ];
    let typed = with_files("use shape\nshape.b.p", &files).unwrap();
    let fipe::ast::Entry::Line(id) = parsed.entries.last().unwrap() else {
        panic!()
    };
    assert_eq!(typed.expr(*id).unwrap().to_string(), "Pt");
}

#[test]
fn inferred_type_depth_is_bounded() {
    let mut source = String::from("x0 = Just 1\n");
    for i in 1..=100 {
        writeln!(source, "x{i} = Just x{}", i - 1).unwrap();
    }
    assert!(super::run(&source).1.is_err());
}

#[test]
fn implied_literal_is_not_printed_again() {
    assert_eq!(
        ty("k: Number a => a -> a = p => p + 1\nf = k"),
        "Number a => a -> a"
    );
}
