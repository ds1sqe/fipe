//! Schemes, groups, annotations, `pub`, defaulting and the body rules
//! of types.md that inference enforces.

use super::{errors, kinds, run, ty};
use fipe::{infer::Reason, token::Source, types::Obligation};

#[test]
fn bindings_generalize_and_uses_instantiate() {
    assert_eq!(ty("add = a b => a + b"), "Add<b> a => a -> b -> a");
    let add = "add = a b => a + b\n";
    assert_eq!(ty(&format!("{add}add 2")), "Integer -> Integer");
    assert_eq!(ty(&format!("{add}add 2\nadd 2.0")), "F64 -> F64");
    // The literal's variable keeps `Add` from `add`: it stays open.
    assert_eq!(
        ty(&format!("{add}x = add 2")),
        "Add<a> b, Literal b => a -> b"
    );
    // A block binding generalizes; one that captures a parameter keeps
    // that parameter's variable.
    let id = "f = x => {\n    id = y => y\n    _ = id 1\n    id \"a\"\n}";
    assert_eq!(ty(id), "a -> String");
    assert_eq!(ty("g = x => {\n    h = y => x\n    h 1\n}"), "a -> a");
    // VAR brings the obligations to the use.
    assert_eq!(kinds(&format!("{add}add \"a\" \"b\"")), [("NoField", "+")]);
}

#[test]
fn groups_are_monomorphic_inside() {
    // The members of one impl call each other in any order: one group,
    // generalized at its boundary.
    let parity = "type P {}\nimpl P {\n    \
        even = n => if n == 0 { True } else { Self.odd (n - 1) },\n    \
        odd = n => if n == 0 { False } else { Self.even (n - 1) },\n}\n";
    assert_eq!(
        ty(&format!("{parity}e = P.even")),
        "Eq a, Sub a, Literal a => a -> Bool"
    );
    // `a` leaves at the line with numeric classes only: it defaults.
    assert_eq!(ty(&format!("{parity}P.even 4")), "Bool");
    assert_eq!(ty(&format!("{parity}P.odd 4.0")), "Bool");
    let annotated = "type P {}\nimpl P {\n    \
        even: I64 -> Bool = n => if n == 0 { True } else { Self.odd (n - 1) },\n    \
        odd = n => if n == 0 { False } else { Self.even (n - 1) },\n}\n";
    assert_eq!(ty(&format!("{annotated}o = P.odd")), "I64 -> Bool");
    // Used before its impl: the group is typed first.
    assert_eq!(
        ty("type P {}\ny = P.id 1\nimpl P { id = x => x }\ny"),
        "Integer"
    );
    // An arm binding calls itself, monomorphic inside, generalized after.
    let count = "count = n => if n == 0 { 0 } else { count (n - 1) }";
    // Its result is held only by the literal `0`: `I64`.
    assert_eq!(ty(count), "Eq a, Sub a, Literal a => a -> Integer");
    assert_eq!(ty(&format!("{count}\ncount 2.0")), "Integer");
    // No polymorphic recursion: the first use fixes the parameter, so
    // the second use's argument fails at that use.
    let text = "f = x => {\n    _ = f \"a\"\n    f True\n}";
    let [error] = errors(text).try_into().unwrap();
    let Reason::Mismatch {
        expected,
        actual,
        other,
    } = error.reason
    else {
        panic!("{:?}", error.reason)
    };
    let source = Source::new(text);
    assert_eq!(source.slice(error.span), Some("\"a\""));
    assert_eq!(source.slice(other), Some("\"a\""));
    assert_eq!(
        (expected.to_string(), actual.to_string()),
        ("Bool".into(), "String".into())
    );
    // Inside an impl the second use names the first.
    let text =
        "type P {}\nimpl P { f = x => {\n    _ = Self.f \"a\"\n    Self.f True\n} }";
    let [error] = errors(text).try_into().unwrap();
    let Reason::Mismatch { other, .. } = error.reason else {
        panic!("{:?}", error.reason)
    };
    assert_eq!(Source::new(text).slice(error.span), Some("\"a\""));
    assert_eq!(Source::new(text).slice(other), Some("\"a\""));
}

#[test]
fn annotations_are_rigid_and_pub_is_closed() {
    let area = "pub area: F64 -> F64 -> F64 = w h => w * h";
    assert_eq!(ty(area), "F64 -> F64 -> F64");
    assert_eq!(kinds("pub f = x => x"), [("NotComplete", "f")]);
    assert_eq!(ty("f: a -> a = x => x"), "a -> a");
    assert_eq!(kinds("f: a -> a = x => x + x"), [("NoField", "+")]);
    assert_eq!(kinds("f: a -> a = x => 1"), [("NoField", "1")]);
    let member = "type P { x: I64 }\nimpl P { pub f = a => a }";
    assert_eq!(kinds(member), [("NotComplete", "f")]);
}

#[test]
fn literals_default_and_the_rest_is_ambiguous() {
    assert_eq!(ty("x = 1"), "Integer");
    assert_eq!(ty("xs = [1, 2]"), "Vec<Integer>");
    // A literal with other obligations stays polymorphic at its binding.
    let inc = "inc = x => x + 1";
    assert_eq!(ty(inc), "Add a, Literal a => a -> a");
    assert_eq!(ty(&format!("{inc}\ninc 2.0")), "F64");
    assert_eq!(ty(&format!("{inc}\ninc 2")), "Integer");
    assert_eq!(ty(&format!("{inc}\ny = inc 2")), "Add a, Literal a => a");
    let check = "check = n => if n < 0 { fault \"negative\" } else { Ok n }";
    assert_eq!(ty(check), "Ord a, Literal a => a -> Result<a, b>");
    assert_eq!(ty(&format!("{check}\ncheck 5")), "Result<Integer, a>");
    // 07's cast row: `round` keeps `F64`.
    assert_eq!(
        kinds("n: I64 = round 2.5"),
        [("NoField", "round"), ("Mismatch", "2.5")]
    );
    let text = "f = s => {\n    _ = read s\n    ()\n}";
    let [error] = errors(text).try_into().unwrap();
    assert_eq!(
        error.reason,
        Reason::Ambiguous {
            class: "read".into()
        }
    );
    assert_eq!(Source::new(text).slice(error.span), Some("read"));
}

#[test]
fn bodies_records_and_literal_ranges() {
    let body = "f = x => {\n    x + 1\n    x\n}";
    assert_eq!(kinds(body), [("Discarded", "x + 1")]);
    assert_eq!(
        ty("f = x => {\n    _ = x + 1\n    x\n}"),
        "Add a, Literal a => a -> a"
    );
    assert_eq!(ty("g = {\n    put out b\"x\"\n    1\n}"), "Integer");
    let point = "type P { x: I64, y: I64 = 0 }\n";
    assert_eq!(ty(&format!("{point}P {{ x: 1 }}")), "P");
    let missing = format!("{point}P {{ y: 1 }}");
    assert_eq!(kinds(&missing), [("MissingField", "P { y: 1 }")]);
    assert_eq!(kinds("x: U8 = 300"), [("OutOfRange", "300")]);
    assert_eq!(kinds("x: I8 = 128"), [("OutOfRange", "128")]);
    assert_eq!(ty("x: I8 = -128"), "I8");
    assert_eq!(kinds("f = x => x\ny: U8 = f 256"), [("OutOfRange", "256")]);
}

#[test]
fn declared_defaults_and_self() {
    let (_, typed) = run("ls | sort");
    assert_eq!(typed.unwrap().unsolved, Vec::<Obligation>::new());
    assert_eq!(ty("ls | sort"), "Vec<Entry>");
    let point = "type Point { x: I64, y: I64 = 0 }\n\
        impl Point { dist = self q => self.x - q }\n";
    assert_eq!(ty(&format!("{point}p = Point 1\np.dist 2")), "Integer");
}

#[test]
fn an_inner_binding_does_not_generalize_over_the_recursive_name() {
    // `g = f` inside `f`'s arms sees `f` monomorphic, as `f` itself is:
    // `g` at two types is the polymorphic recursion the direct form
    // already rejects.
    let text = "f = n => {\n  g = f\n  a: I64 = g n\n  b: String = g n\n  a\n}";
    assert_eq!(kinds(text), [("Mismatch", "g n")]);
}

#[test]
fn an_early_use_of_an_impl_sees_the_bindings_its_members_name() {
    // `id` names `k`, typed after the use: the group waits for its
    // entry, and the use takes its scheme there.
    let text = "type P {}\ny = P.id 1\nk = \"s\"\nimpl P { id = x => k }\ny";
    assert_eq!(ty(text), "String");
    // Held monomorphic until then: `f` at two types fails, at the
    // second literal: no meaning reads both forms.
    let text = "type P {}\nf = z => P.id z\na = f 1\nb = f \"s\"\n\
        k = \"s\"\nimpl P { id = x => k }";
    assert_eq!(kinds(text), [("NoCommonMeaning", "\"s\"")]);
    // With `k` typed first, the early use sees the scheme at once.
    let text = "type P {}\nk = \"s\"\nf = z => P.id z\na = f 1\n\
        b = f \"s\"\nimpl P { id = x => k }\nb";
    assert_eq!(ty(text), "String");
}
