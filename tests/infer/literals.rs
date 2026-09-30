use super::{errors, ty};

#[test]
fn forms_choose_together_and_widths_keep_their_names() {
    assert_eq!(ty("x = 18446744073709551616"), "Integer");
    assert_eq!(ty("f: F64 = 2"), "F64");
    assert_eq!(ty("a: U8 = 2\nb: U16 = 3\na + b"), "Integer");
    assert_eq!(ty("a: U8 = 2\nb: U16 = a"), "U16");
    for (a, b) in [("2", "2.5"), ("2.5", "2")] {
        assert_eq!(
            ty(&format!("c = True\nx = if c {{ {a} }} else {{ {b} }}")),
            "F64"
        );
    }
    assert_ne!(
        errors("c = True\nn: Integer = 2\nx = if c { n } else { 2.5 }"),
        []
    );
    assert_ne!(errors("c = True\nx = if c { 2 } else { \"a\" }"), []);
    let source = "c = True\nx = if c { 2 } else { \"a\" }";
    let found = errors(source);
    assert!(
        matches!(found.as_slice(), [fipe::infer::Error { span, reason: fipe::infer::Reason::NoCommonMeaning { forms } }]
        if &source[span.start..span.end] == "if c { 2 } else { \"a\" }" && forms == &["Whole", "Text"])
    );
}

#[test]
fn a_program_literal_member_accepts_a_form_without_evaluating_it() {
    let mask = "type Mask { bits: U8 }\nimpl Mask { literal = f => Self 0 }\n";
    assert_eq!(ty(&format!("{mask}m: Mask = 0b0000_1111")), "Mask");
}

#[test]
fn arithmetic_right_operands_and_literal_ties() {
    assert_eq!(ty("double = n => n + n"), "Add a => a -> a");
    assert_eq!(ty("scale = v k => v * k"), "Mul<b> a => a -> b -> a");
    assert_eq!(ty("inc = n => n + 1"), "Add a, Literal a => a -> a");
    assert_eq!(
        ty("double: Add a => a -> a = n => n + n"),
        "Add a => a -> a"
    );
    let vector = "type V { x: F64 }\nimpl V { mul: F64 -> Self -> Self = k self => Self (self.x * k) }\nv = V 1.0\n";
    assert_eq!(ty(&format!("{vector}k: F64 = 2\nv * k")), "V");
    assert!(matches!(errors(&format!("{vector}v * 2.0")).as_slice(),
        [fipe::infer::Error { reason: fipe::infer::Reason::MissingMember { name, .. }, .. }] if name == "literal"));
}

#[test]
fn generic_literal_forms_survive_instantiation() {
    for source in [
        "half = n => n + 0.5\n",
        "half: Add a, Literal a => a -> a = n => n + 0.5\n",
    ] {
        assert_eq!(ty(&format!("{source}half 2.0")), "F64");
        assert_ne!(errors(&format!("{source}n: Integer = 2\nhalf n")), []);
    }
}

#[test]
fn arithmetic_results_follow_a_known_receivers_member() {
    assert_eq!(ty("diff = p q => p - q"), "Sub<b> a => a -> b -> a");
    assert_eq!(ty("diff = p => p - p"), "Sub a => a -> a");
    let source = "type Vector { x: F64 }\ntype Point { x: F64 }\nimpl Point { sub: Self -> Self -> Vector = q self => Vector (self.x - q.x) }\ndiff: Point -> Point -> Vector = p q => p - q";
    assert_eq!(ty(source), "Point -> Point -> Vector");
}

#[test]
fn whole_forms_at_float_places_must_be_exact() {
    assert_eq!(ty("x: F32 = 16777216"), "F32");
    assert_ne!(errors("x: F32 = 16777217"), []);
    assert_eq!(ty("x: F64 = 18446744073709551616"), "F64");
    assert_ne!(errors("x: F64 = 9007199254740993"), []);
}
