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
    assert_eq!(
        scheme,
        "b.age: a, Mul a, Ord a, Literal a => a -> b -> Bool"
    );
    let text = format!("{OLDER}\nls -l /tmp | filter (older 7)");
    assert_eq!(ty(&text), "Vec<Entry>");
    // Never used: nothing leaves, nothing fails.
    assert!(run("h = e => e.x").1.is_ok());
    assert_eq!(kinds("k = e => e.name\nk"), [("Ambiguous", "e.name")]);
}

#[test]
fn a_member_solves_field_first_then_self_function() {
    let point = "type P { x: I64 }\nimpl P { dist = self q => self.x - q.x }\n";
    assert_eq!(ty(&format!("{point}(P 1).dist (P 2)")), "Integer");
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
    assert_eq!(kinds(text), [("Ambiguous", "x.name")]);
    // `Eq` alone does not default where its variable leaves.
    assert_eq!(kinds("[] == []"), [("Ambiguous", "==")]);
}

#[test]
fn a_selector_is_a_member_on_its_parameter() {
    assert_eq!(ty("s = .name"), "a.name: b => a -> b");
    assert_eq!(ty("s = .name\nls | map s"), "Vec<String>");
    assert_eq!(ty("ls | map .name"), "Vec<String>");
    assert_eq!(kinds("(f => 1) .name"), [("Ambiguous", ".name")]);
}

#[test]
fn named_member_types_fold_from_their_declarations() {
    assert_eq!(ty("type Named where self.name: String\nf: Named a => a -> String = x => x.name"), "Named a => a -> String");
    assert_eq!(
        ty("type Named where self.name: String\nf = x => x.name"),
        "a.name: b => a -> b"
    );
}

#[test]
fn packed_places_check_members_and_keep_only_receiver_operations() {
    let shape = "type Shape where self.area: F64\ntype Circle { radius: F64 }\nimpl Circle { area = self => self.radius * self.radius }\n";
    assert_eq!(ty(&format!("{shape}s: Shape = Circle 2\ns.area")), "F64");
    assert_eq!(
        ty(&format!("{shape}xs: Vec<Shape> = [Circle 2, Circle 3]")),
        "Vec<Shape>"
    );
    assert_ne!(errors(&format!("{shape}s: Shape = True")), []);
    assert_eq!(ty("x: Integer | String = 2\nf: (Integer | String) -> String = Integer n => show n, String s => s\nf x"), "String");
}

#[test]
fn member_words_dispatch_without_a_higher_kind() {
    assert_eq!(ty("(Just 1).map (x => x + 1)"), "Maybe<Integer>");
    assert_eq!(ty("[1, 2] | map (x => x + 1)"), "Vec<Integer>");
    assert_eq!(ty("\"a\" | concat \"b\""), "String");
}

#[test]
fn derived_members_follow_parts_and_recursive_shapes() {
    assert_eq!(ty("type Tree { Leaf(value: Integer), Branch(children: Vec<Tree>) }\n(Leaf 1) == (Leaf 2)"), "Bool");
    assert_eq!(ty("type P { x: Integer }\nshow (P 2)"), "String");
    assert_ne!(
        errors("type P { f: Integer -> Integer }\nshow (P (x => x))"),
        []
    );
    let shape = "type Shape where self.area: F64\ntype C { r: F64 }\nimpl C { area = self => self.r }\nf: Shape -> F64 = x => x.area\nC 2 | f";
    assert_eq!(ty(shape), "F64");
}

#[test]
fn array_concat_and_same_meaning_unions_remain_unchecked() {
    assert_eq!(
        kinds("f: [Integer; 1] -> [Integer; 2] = a => a ++ a"),
        [("NotYetChecked", "++")]
    );
    assert_eq!(kinds("x: I8 | U8 = 1"), [("NotYetChecked", "I8 | U8")]);
}

#[test]
fn supplied_members_use_the_last_spot_and_never_a_bare_name() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.commands("pub type Meter\nimpl Meter { scale: F64 -> Self -> F64 }\npub meter: Meter").unwrap();
    let module = super::module("meter.scale 2.0");
    let resolved = fipe::resolve::resolve(&module, &declarations, &[]).unwrap();
    let typed =
        fipe::infer::infer(&module, &resolved, &declarations, &[]).unwrap();
    let fipe::ast::Entry::Line(id) = module.entries[0] else {
        panic!()
    };
    assert_eq!(typed.expr(id).unwrap().to_string(), "F64");
    assert!(declarations
        .commands
        .as_ref()
        .unwrap()
        .value("scale")
        .is_none());
    assert!(fipe::resolve::resolve(
        &super::module("scale"),
        &declarations,
        &[]
    )
    .is_err());
}
