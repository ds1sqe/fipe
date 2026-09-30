//! The forms 06.1 adds: type signs, numbers in types, the `type` item's
//! base and limits, tuples, gate patterns, members with no value,
//! member constraints, `++`, and `trait` and `where` as names.

use super::{errors, integer, module, name, node, only};
use fipe::{
    ast::{
        ArgumentKind, BinaryOp, CallStyle, ConstraintKind, Entry, ExprKind,
        ImplMember, Item, ItemKind, Limit, Literal, Module, PatternKind,
        TypeId, TypeKind,
    },
    parse::Reason,
    token::Source,
};

/// A type as a prefix tree: `(-> (* A B) C)`, `(! A)`, `Bits<8>`.
fn shape(parsed: &Module, id: TypeId) -> String {
    let all = |ids: &[TypeId]| {
        ids.iter()
            .map(|id| shape(parsed, *id))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match &parsed.ty(id).unwrap().kind {
        TypeKind::Unit => "()".into(),
        TypeKind::Named { name, arguments } if arguments.is_empty() => {
            name.segments.join(".")
        }
        TypeKind::Named { name, arguments } => {
            let arguments: Vec<_> =
                arguments.iter().map(|id| shape(parsed, *id)).collect();
            format!("{}<{}>", name.segments.join("."), arguments.join(", "))
        }
        TypeKind::Number(value) => value.to_string(),
        TypeKind::Array { element, length } => {
            format!("[{}; {}]", shape(parsed, *element), shape(parsed, *length))
        }
        TypeKind::Tuple(parts) => format!("(* {})", all(parts)),
        TypeKind::Both(parts) => format!("(& {})", all(parts)),
        TypeKind::Either(parts) => format!("(| {})", all(parts)),
        TypeKind::Not(inner) => format!("(! {})", shape(parsed, *inner)),
        TypeKind::Function { parameter, result } => format!(
            "(-> {} {})",
            shape(parsed, *parameter),
            shape(parsed, *result)
        ),
        kind => format!("{kind:?}"),
    }
}

/// The type of `name: type`, a declaration.
fn declared(input: &str) -> String {
    let parsed = module(input);
    let [Entry::Item(Item {
        kind: ItemKind::Declare { ty, .. },
        ..
    })] = parsed.entries.as_slice()
    else {
        panic!("{input:?}: {:?}", parsed.entries);
    };
    shape(&parsed, *ty)
}

#[test]
fn type_signs_bind_not_then_both_either_tuple_and_arrow() {
    for (input, expected) in [
        ("f: !A & B | C * D -> E", "(-> (* (| (& (! A) B) C) D) E)"),
        ("f: A * B -> C", "(-> (* A B) C)"),
        ("f: A * B * C", "(* A B C)"),
        ("f: A & B & C", "(& A B C)"),
        ("f: A | B & C", "(| A (& B C))"),
        ("f: !(A | B)", "(! (| A B))"),
        ("f: (A * B) -> C -> D", "(-> (* A B) (-> C D))"),
        ("f: A -> B * C", "(-> A (* B C))"),
        ("f: [!T; 3]", "[(! T); 3]"),
        ("f: [T; n]", "[T; n]"),
        ("f: Bits<8> & Size<1>", "(& Bits<8> Size<1>)"),
        ("f: Ring<T, n>", "Ring<T, n>"),
        ("f: Vec<Shape> -> F64", "(-> Vec<Shape> F64)"),
    ] {
        assert_eq!(declared(input), expected, "{input}");
    }
    // `|!` is a pipe; in a type it is the error that asks for `| !`.
    let input = "f: A |! B";
    let result = errors(input);
    assert_eq!(result[0].reason, Reason::EitherNot);
    assert_eq!(Source::new(input).slice(result[0].span), Some("|!"));
    assert_eq!(declared("f: A | !B"), "(| A (! B))");
    // A type argument is a type or a non-negative integer.
    assert_eq!(
        errors("f: Bits<-8>")[0].reason,
        Reason::Expected(fipe::parse::Expected::NonnegativeInteger)
    );
}

#[test]
fn an_annotation_holds_a_sign_and_member_constraints() {
    let (parsed, id) = only("id: Integer | String = 3");
    let ExprKind::Bind {
        annotation: Some(annotation),
        value,
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    assert_eq!(shape(&parsed, *annotation), "(| Integer String)");
    integer(&parsed, *value, 3);
    let input = "older: b.age: a, Ord a => a -> b -> Bool = d e => e.age > d";
    let (parsed, id) = only(input);
    let ExprKind::Bind {
        annotation: Some(annotation),
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    let TypeKind::Qualified { constraints, ty } =
        &parsed.ty(*annotation).unwrap().kind
    else {
        panic!()
    };
    let [first, second] = constraints.as_slice() else {
        panic!()
    };
    let ConstraintKind::Member { member, ty: of } = &first.kind else {
        panic!()
    };
    assert_eq!(member.segments, ["b", "age"]);
    assert_eq!(shape(&parsed, *of), "a");
    assert_eq!(Source::new(input).slice(first.span), Some("b.age: a"));
    assert!(matches!(second.kind, ConstraintKind::Named { .. }));
    assert_eq!(shape(&parsed, *ty), "(-> a (-> b Bool))");
}

/// The parts of a file's one `type` item.
fn type_item(input: &str) -> (Module, Item) {
    let parsed = module(input);
    let [Entry::Item(item)] = parsed.entries.as_slice() else {
        panic!("{input:?}: {:?}", parsed.entries);
    };
    let item = item.clone();
    (parsed, item)
}

#[test]
fn a_type_item_takes_a_base_members_and_limits() {
    let input = "type U8: Natural & Bits<8> where self <= 255";
    let (parsed, item) = type_item(input);
    let ItemKind::Type {
        base: Some(base),
        members: None,
        limits,
        ..
    } = &item.kind
    else {
        panic!()
    };
    assert_eq!(shape(&parsed, *base), "(& Natural Bits<8>)");
    let [Limit::Test(test)] = limits.as_slice() else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, *test),
        ExprKind::Binary {
            op: BinaryOp::LessEqual,
            ..
        }
    ));
    // Signed, two limits; and each continuation of a limit list.
    for input in [
        "type I8: Integer & Bits<8> where -128 <= self, self <= 127",
        "type I8: Integer & Bits<8>\nwhere -128 <= self, self <= 127",
        "type I8: Integer & Bits<8> where\n  -128 <= self,\n  self <= 127",
    ] {
        let (_, item) = type_item(input);
        let ItemKind::Type { limits, .. } = &item.kind else {
            panic!()
        };
        assert_eq!(limits.len(), 2, "{input}");
    }
    let (parsed, item) = type_item("type Shape where self.area: F64");
    let ItemKind::Type {
        base: None, limits, ..
    } = &item.kind
    else {
        panic!()
    };
    let [Limit::Member { name, ty, .. }] = limits.as_slice() else {
        panic!()
    };
    assert_eq!(name.segments, ["self", "area"]);
    assert_eq!(shape(&parsed, *ty), "F64");
    let (_, item) = type_item("type Mode: Size<1> { read: Flag, write: Flag }");
    assert!(matches!(
        &item.kind,
        ItemKind::Type { base: Some(_), members: Some(members), limits, .. }
            if members.len() == 2 && limits.is_empty()
    ));
    let (_, item) = type_item(
        "type Range { lo: Integer, hi: Integer } where self.lo <= self.hi",
    );
    assert!(matches!(
        &item.kind,
        ItemKind::Type { base: None, members: Some(_), limits, .. }
            if limits.len() == 1
    ));
    let input = "type Sorted<T: Ord>: Vec<T> where is_sorted self";
    let (parsed, item) = type_item(input);
    let ItemKind::Type {
        parameters,
        base: Some(base),
        ..
    } = &item.kind
    else {
        panic!()
    };
    assert_eq!(shape(&parsed, parameters[0].bound.unwrap()), "Ord");
    assert_eq!(shape(&parsed, *base), "Vec<T>");
    let (_, item) = type_item("type Key: Eq & Show");
    assert!(matches!(
        &item.kind,
        ItemKind::Type { base: Some(_), members: None, limits, .. }
            if limits.is_empty()
    ));
    let (_, item) = type_item("type Rd<Api>");
    assert!(matches!(
        &item.kind,
        ItemKind::Type { base: None, members: None, limits, .. }
            if limits.is_empty()
    ));
}

#[test]
fn where_is_a_name_outside_a_type_item() {
    // After a blank line `where` begins a line of its own: a command.
    let parsed = module("type A: B\n\nwhere x");
    assert!(matches!(
        parsed.entries.as_slice(),
        [Entry::Item(_), Entry::Line(_)]
    ));
    let (parsed, id) = only("grep where");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let ArgumentKind::Positional(argument) = arguments[0].kind else {
        panic!()
    };
    name(&parsed, argument, "where");
    // `trait` is a name, and an item only before an uppercase name.
    let (parsed, id) = only("trait = 1");
    assert!(matches!(node(&parsed, id), ExprKind::Bind { .. }));
    let (parsed, id) = only("trait x");
    assert!(matches!(node(&parsed, id), ExprKind::Call { .. }));
    let (parsed, id) = only("trait T {}");
    assert!(matches!(node(&parsed, id), ExprKind::Call { .. }));
}

/// A tuple is `( )` not touching a callee with a top-level comma before
/// any `=>`; `(a)` stays a group and `()` unit.
#[test]
fn tuples_are_values_patterns_and_one_spaced_argument() {
    let (parsed, id) = only("(1, \"one\")");
    let ExprKind::Tuple(parts) = node(&parsed, id) else {
        panic!()
    };
    integer(&parsed, parts[0], 1);
    let (parsed, id) = only("(a,\n  b,\n  c\n)");
    assert!(
        matches!(node(&parsed, id), ExprKind::Tuple(parts) if parts.len() == 3)
    );
    let (parsed, id) = only("(a)");
    assert!(matches!(node(&parsed, id), ExprKind::Group(_)));
    let (parsed, id) = only("()");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Literal(Literal::Unit)
    ));
    // A comparison may stand in a part; type arguments' commas are theirs.
    let (parsed, id) = only("(a > b, c < d)");
    assert!(
        matches!(node(&parsed, id), ExprKind::Tuple(parts) if parts.len() == 2)
    );
    // Type arguments' commas are theirs.
    let (parsed, id) = only("(x | to<Either<A, B>>)");
    assert!(matches!(node(&parsed, id), ExprKind::Group(_)));
    // `f(a, b)` is two arguments, `f (a, b)` one tuple, `f ((a, b))` a
    // group holding one.
    let (parsed, id) = only("f(a, b)");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Call { arguments, style: CallStyle::Parenthesized, .. }
            if arguments.len() == 2
    ));
    let (parsed, id) = only("f ((a, b))");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    let ArgumentKind::Positional(argument) = arguments[0].kind else {
        panic!()
    };
    let ExprKind::Group(lines) = node(&parsed, argument) else {
        panic!()
    };
    assert!(matches!(node(&parsed, lines[0]), ExprKind::Tuple(_)));
    // Patterns: a binding and an arm head.
    let (parsed, id) = only("(n, name) = pair");
    let ExprKind::Bind { pattern, .. } = node(&parsed, id) else {
        panic!()
    };
    assert!(matches!(
        &parsed.pattern(*pattern).unwrap().kind,
        PatternKind::Tuple(parts) if parts.len() == 2
    ));
    let (parsed, id) = only("swap = (a, b) => (b, a)");
    let ExprKind::Bind { value, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *value) else {
        panic!()
    };
    assert!(matches!(
        parsed.pattern(arms[0].patterns[0]).unwrap().kind,
        PatternKind::Tuple(_)
    ));
    assert!(matches!(node(&parsed, arms[0].body), ExprKind::Tuple(_)));
    // An arm inside a tuple is grouped; there is no trailing comma.
    assert_eq!(errors("(a, b => c)")[0].reason, Reason::ArmsNeedGroup);
    assert_eq!(
        errors("(a, b,)")[0].reason,
        Reason::Expected(fipe::parse::Expected::Expression)
    );
    assert_eq!(errors("(a\n, b)")[0].reason, Reason::CommaBeforeLineBreak);
}

#[test]
fn a_gate_pattern_is_a_name_with_a_payload() {
    let input =
        "n | { Prime p => \"prime\", U8 b => \"small\", Integer n => n.show, _ => \"big\" }";
    let (parsed, id) = only(input);
    let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *right) else {
        panic!()
    };
    let gates: Vec<_> = arms[..3]
        .iter()
        .map(|arm| match &parsed.pattern(arm.patterns[0]).unwrap().kind {
            PatternKind::Constructor { name, .. } => name.segments.join("."),
            kind => panic!("{kind:?}"),
        })
        .collect();
    assert_eq!(gates, ["Prime", "U8", "Integer"]);
}

#[test]
fn an_impl_member_may_have_no_value() {
    let input = "impl Integer { add: Self -> (self: Self) -> Self, pub neg = self => 0 - self }";
    let parsed = module(input);
    let [Entry::Item(Item {
        kind: ItemKind::Impl { members, .. },
        ..
    })] = parsed.entries.as_slice()
    else {
        panic!()
    };
    let [ImplMember::Named {
        annotation: Some(annotation),
        value: None,
        ..
    }, ImplMember::Named {
        public: true,
        value: Some(_),
        ..
    }] = members.as_slice()
    else {
        panic!("{members:?}")
    };
    assert!(matches!(
        parsed.ty(*annotation).unwrap().kind,
        TypeKind::Function { .. }
    ));
    for input in [
        "impl Vec<T> { sequence: [T; n] -> Self }",
        "impl Ring<T, n> { sequence = slots => Self slots 0 }",
    ] {
        module(input);
    }
}

#[test]
fn concat_is_additive_and_continues_a_line() {
    let (parsed, id) = only("xs ++ [4, 5]");
    assert!(matches!(
        node(&parsed, id),
        ExprKind::Binary {
            op: BinaryOp::Concat,
            ..
        }
    ));
    // Left-associative with `+`, looser than `*`.
    let (parsed, id) = only("a + b ++ c * d");
    let ExprKind::Binary {
        op: BinaryOp::Concat,
        left,
        right,
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    assert!(matches!(
        node(&parsed, *left),
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    assert!(matches!(
        node(&parsed, *right),
        ExprKind::Binary {
            op: BinaryOp::Multiply,
            ..
        }
    ));
    for input in ["a ++\nb", "a\n++ b"] {
        let (parsed, id) = only(input);
        assert!(matches!(node(&parsed, id), ExprKind::Binary { .. }));
    }
    let (parsed, id) = only("a++b");
    assert_eq!(node(&parsed, id), &ExprKind::Word("a++b".into()));
}

#[test]
fn a_prefix_or_suffix_is_a_literal_in_values_and_patterns() {
    let (parsed, id) = only("f 10ms re\"a+b\"i");
    let ExprKind::Call { arguments, .. } = node(&parsed, id) else {
        panic!()
    };
    for argument in arguments {
        let ArgumentKind::Positional(value) = argument.kind else {
            panic!()
        };
        assert!(matches!(
            node(&parsed, value),
            ExprKind::Literal(Literal::Affixed(_))
        ));
    }
    let (parsed, id) = only("x | { 2i => 1, _ => 0 }");
    let ExprKind::Pipe { right, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *right) else {
        panic!()
    };
    assert!(matches!(
        parsed.pattern(arms[0].patterns[0]).unwrap().kind,
        PatternKind::Literal(Literal::Affixed(_))
    ));
}

#[test]
fn type_parameter_defaults_follow_bounds_and_are_trailing() {
    let parsed = module("type Add<T: Eq = Self> where self.add: T -> Self");
    let Entry::Item(item) = &parsed.entries[0] else {
        panic!()
    };
    let ItemKind::Type { parameters, .. } = &item.kind else {
        panic!()
    };
    assert!(parameters[0].bound.is_some());
    assert!(parameters[0].default.is_some());
    assert!(fipe::parse::parse(fipe::lex::lex("type P<A = Self, B>")).is_err());
}
