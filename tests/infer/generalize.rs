//! Schemes, groups, annotations, `pub`, defaulting and the body rules
//! of types.md that inference enforces.

use super::{errors, kinds, run, ty};
use fipe::{infer::Reason, token::Source, types::Obligation};

#[test]
fn bindings_generalize_and_uses_instantiate() {
    assert_eq!(ty("add = a b => a + b"), "Num a => a -> a -> a");
    let add = "add = a b => a + b\n";
    assert_eq!(ty(&format!("{add}add 2")), "i64 -> i64");
    assert_eq!(ty(&format!("{add}add 2\nadd 2.0")), "f64 -> f64");
    // The literal's variable keeps `Num` from `add`: it stays open.
    assert_eq!(ty(&format!("{add}x = add 2")), "Num a => a -> a");
    // A block binding generalizes; one that captures a parameter keeps
    // that parameter's variable.
    let id = "f = x => {\n    id = y => y\n    _ = id 1\n    id \"a\"\n}";
    assert_eq!(ty(id), "a -> String");
    assert_eq!(ty("g = x => {\n    h = y => x\n    h 1\n}"), "a -> a");
    // VAR brings the obligations to the use.
    assert_eq!(
        kinds(&format!("{add}add \"a\" \"b\"")),
        [("NotNumeric", "add")]
    );
}

#[test]
fn groups_are_monomorphic_inside() {
    let parity = "even = n => if n == 0 { True } else { odd (n - 1) }\n\
        odd = n => if n == 0 { False } else { even (n - 1) }\n";
    assert_eq!(ty(parity.trim_end()), "Num a, Eq a => a -> Bool");
    // `a` leaves at the line with numeric classes only: it defaults.
    assert_eq!(ty(&format!("{parity}even 4")), "Bool");
    let annotated =
        "even: i64 -> Bool = n => if n == 0 { True } else { odd (n - 1) }\n\
        odd = n => if n == 0 { False } else { even (n - 1) }\n";
    assert_eq!(ty(&format!("{annotated}odd")), "i64 -> Bool");
    // Used before its line: the group is typed first.
    assert_eq!(ty("y = f 1\nf = x => x\ny"), "i64");
    // No polymorphic recursion: the second use names the first.
    let text = "f = x => {\n    _ = f \"a\"\n    f True\n}";
    let [error] = errors(text).try_into().unwrap();
    let Reason::Mismatch { other, .. } = error.reason else {
        panic!("{:?}", error.reason)
    };
    assert_eq!((error.span.start, other.start), (29, 19));
}

#[test]
fn annotations_are_rigid_and_pub_is_closed() {
    let area = "pub area: f64 -> f64 -> f64 = w h => w * h";
    assert_eq!(ty(area), "f64 -> f64 -> f64");
    assert_eq!(kinds("pub f = x => x"), [("NotComplete", "f")]);
    assert_eq!(ty("f: a -> a = x => x"), "a -> a");
    assert_eq!(kinds("f: a -> a = x => x + x"), [("NotNumeric", "+")]);
    assert_eq!(kinds("f: a -> a = x => 1"), [("Mismatch", "1")]);
    let member = "type P { x: i64 }\nimpl P { pub f = a => a }";
    assert_eq!(kinds(member), [("NotComplete", "f")]);
}

#[test]
fn literals_default_and_the_rest_is_ambiguous() {
    assert_eq!(ty("x = 1"), "i64");
    assert_eq!(ty("xs = [1, 2]"), "Vec<i64>");
    // A literal with other obligations stays polymorphic at its binding.
    let inc = "inc = x => x + 1";
    assert_eq!(ty(inc), "Num a => a -> a");
    assert_eq!(ty(&format!("{inc}\ninc 2.0")), "f64");
    assert_eq!(ty(&format!("{inc}\ninc 2")), "i64");
    assert_eq!(ty(&format!("{inc}\ny = inc 2")), "Num a => a");
    let check = "check = n => if n < 0 { fault \"negative\" } else { Ok n }";
    assert_eq!(ty(check), "Num a, Ord a => a -> Result<a, b>");
    let result = ty(&format!("{check}\ncheck 5"));
    assert!(result.starts_with("Result<i64, ?"), "{result}");
    // 07's cast row: `round` keeps `f64`.
    assert_eq!(kinds("n: i64 = round 2.5"), [("Mismatch", "2.5")]);
    let text = "f = s => {\n    _ = read s\n    ()\n}";
    let [error] = errors(text).try_into().unwrap();
    assert_eq!(
        error.reason,
        Reason::Ambiguous {
            class: "Read".into()
        }
    );
    assert_eq!(Source::new(text).slice(error.span), Some("f"));
}

#[test]
fn bodies_records_and_literal_ranges() {
    let body = "f = x => {\n    x + 1\n    x\n}";
    assert_eq!(kinds(body), [("Discarded", "x + 1")]);
    assert_eq!(ty("f = x => {\n    _ = x + 1\n    x\n}"), "Num a => a -> a");
    assert_eq!(ty("g = {\n    put out b\"x\"\n    1\n}"), "i64");
    let point = "type P { x: i64, y: i64 = 0 }\n";
    assert_eq!(ty(&format!("{point}P {{ x: 1 }}")), "P");
    let missing = format!("{point}P {{ y: 1 }}");
    assert_eq!(kinds(&missing), [("MissingField", "P { y: 1 }")]);
    assert_eq!(kinds("x: u8 = 300"), [("OutOfRange", "300")]);
    assert_eq!(kinds("x: i8 = 128"), [("OutOfRange", "128")]);
    assert_eq!(ty("x: i8 = -128"), "i8");
    assert_eq!(kinds("f = x => x\ny: u8 = f 256"), [("OutOfRange", "256")]);
}

#[test]
fn declared_defaults_and_self() {
    let (_, typed) = run("ls | sort");
    assert_eq!(typed.unwrap().unsolved, Vec::<Obligation>::new());
    assert_eq!(ty("ls | sort"), "Vec<Entry>");
    let point = "type Point { x: i64, y: i64 = 0 }\n\
        impl Point { dist = self q => self.x - q }\n";
    assert_eq!(ty(&format!("{point}p = Point 1\np.dist 2")), "i64");
}
