use super::{errors, integer, module, name, node, only};
use fipe::{
    ast::{
        Entry, ExprKind, ImplMember, ItemKind, Member, PatternKind, TypeKind,
    },
    parse::Reason,
    token::Source,
};

#[test]
fn imports_take_names_and_a_version() {
    let input = "use fs\nuse text 2\nuse x.inc x.Point";
    let parsed = module(input);
    let [Entry::Item(fs), Entry::Item(text), Entry::Item(names)] =
        parsed.entries.as_slice()
    else {
        panic!()
    };
    assert!(
        matches!(&fs.kind, ItemKind::Use { names, version: None } if names[0].segments == ["fs"])
    );
    assert!(
        matches!(&text.kind, ItemKind::Use { names, version: Some(2) } if names.len() == 1)
    );
    let ItemKind::Use {
        names,
        version: None,
    } = &names.kind
    else {
        panic!()
    };
    assert_eq!(names[0].segments, ["x", "inc"]);
    assert_eq!(names[1].segments, ["x", "Point"]);
    assert!(!fs.public);
}

#[test]
fn bindings_carry_patterns_annotations_and_arm_heads() {
    let input = "pub add: i64 -> i64 -> i64 = a (b = 0) => a + b";
    let (parsed, id) = only(input);
    let ExprKind::Bind {
        public: true,
        pattern,
        annotation: Some(annotation),
        value,
    } = node(&parsed, id)
    else {
        panic!()
    };
    assert!(
        matches!(&parsed.pattern(*pattern).unwrap().kind, PatternKind::Name(name) if name.segments == ["add"])
    );
    let TypeKind::Function { parameter, result } =
        &parsed.ty(*annotation).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        Source::new(input).slice(parsed.ty(*parameter).unwrap().span),
        Some("i64")
    );
    assert!(matches!(
        parsed.ty(*result).unwrap().kind,
        TypeKind::Function { .. }
    ));
    let ExprKind::Arms(arms) = node(&parsed, *value) else {
        panic!()
    };
    let [a, b] = arms[0].patterns.as_slice() else {
        panic!()
    };
    assert!(matches!(
        parsed.pattern(*a).unwrap().kind,
        PatternKind::Name(_)
    ));
    let PatternKind::Default { name: b, value } =
        &parsed.pattern(*b).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(b.segments, ["b"]);
    integer(&parsed, *value, 0);
    assert_eq!(parsed.expr(id).unwrap().span.start, 4);

    for (input, heads) in [
        ("f = a b => a", 2),
        ("f = Just e rest => e", 2),
        ("f = Just Just e => e", 1),
        ("f = Point { x, .. } y => x", 2),
        ("f = [:e] => e", 1),
        ("f = () => 1", 1),
    ] {
        let (parsed, id) = only(input);
        let ExprKind::Bind { value, .. } = node(&parsed, id) else {
            panic!("{input}")
        };
        let ExprKind::Arms(arms) = node(&parsed, *value) else {
            panic!("{input}")
        };
        assert_eq!(arms[0].patterns.len(), heads, "{input}");
    }
    let (parsed, id) = only("f = Just Just e => e");
    let ExprKind::Bind { value, .. } = node(&parsed, id) else {
        panic!()
    };
    let ExprKind::Arms(arms) = node(&parsed, *value) else {
        panic!()
    };
    let PatternKind::Constructor { payload, .. } =
        &parsed.pattern(arms[0].patterns[0]).unwrap().kind
    else {
        panic!()
    };
    assert!(matches!(
        parsed.pattern(*payload).unwrap().kind,
        PatternKind::Constructor { .. }
    ));
    for input in ["x = 1", "[x, y] = e", "Just x = e", "x: i64 = e"] {
        let (parsed, id) = only(input);
        assert!(
            matches!(node(&parsed, id), ExprKind::Bind { .. }),
            "{input}"
        );
    }
    let (parsed, id) = only("cmd -c=80");
    assert!(matches!(node(&parsed, id), ExprKind::Call { .. }));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One item corpus, checked member by member"
)]
fn types_traits_and_impls_have_member_lists() {
    let input = "pub type Point { pub x: i64, pub y: i64 = 0, len: u64 }\ntype Shape { Circle(r: f64), Dot }\ntype Maybe<T: Show> { Just(v: T), Nothing }\ntrait Ord: Eq { cmp: Self -> Self -> Ordering, max: Self -> Self -> Self = a b => a }\nimpl Point { x (y = 0) => Self { x, y, len: 0 }, pub line: i64 -> Self = p => Self p p }\nimpl Point: From<Polar> { from = pl => Self { x: 1, y: 2, len: 3 } }";
    let parsed = module(input);
    let [Entry::Item(point), Entry::Item(shape), Entry::Item(maybe), Entry::Item(ord), Entry::Item(inherent), Entry::Item(from)] =
        parsed.entries.as_slice()
    else {
        panic!()
    };
    assert!(point.public);
    let ItemKind::Type {
        name: type_name,
        parameters,
        members,
    } = &point.kind
    else {
        panic!()
    };
    assert_eq!(type_name.segments, ["Point"]);
    assert_eq!(parameters.len(), 0);
    let [Member::Field(x), Member::Field(y), Member::Field(len)] =
        members.as_slice()
    else {
        panic!()
    };
    assert!(x.public && y.public && !len.public);
    assert_eq!(x.default, None);
    integer(&parsed, y.default.unwrap(), 0);
    assert_eq!(
        Source::new(input).slice(parsed.ty(len.ty).unwrap().span),
        Some("u64")
    );
    let ItemKind::Type { members, .. } = &shape.kind else {
        panic!()
    };
    let [Member::Constructor {
        name: circle,
        fields,
        ..
    }, Member::Constructor {
        name: dot,
        fields: none,
        ..
    }] = members.as_slice()
    else {
        panic!()
    };
    assert_eq!(circle.segments, ["Circle"]);
    assert_eq!(fields[0].name.segments, ["r"]);
    assert_eq!(dot.segments, ["Dot"]);
    assert_eq!(none.len(), 0);
    let ItemKind::Type { parameters, .. } = &maybe.kind else {
        panic!()
    };
    assert_eq!(parameters[0].name.segments, ["T"]);
    assert_eq!(parameters[0].bound.as_ref().unwrap().segments, ["Show"]);

    let ItemKind::Trait {
        name: trait_name,
        parent: Some(parent),
        members,
    } = &ord.kind
    else {
        panic!()
    };
    assert_eq!(trait_name.segments, ["Ord"]);
    assert_eq!(parent.segments, ["Eq"]);
    assert_eq!(members[0].name.segments, ["cmp"]);
    assert_eq!(members[0].default, None);
    assert!(matches!(
        node(&parsed, members[1].default.unwrap()),
        ExprKind::Arms(_)
    ));

    let ItemKind::Impl {
        target,
        implements: None,
        members,
    } = &inherent.kind
    else {
        panic!()
    };
    assert_eq!(
        Source::new(input).slice(parsed.ty(*target).unwrap().span),
        Some("Point")
    );
    let [ImplMember::Constructor(arm), ImplMember::Named {
        public: true,
        name: line,
        annotation: Some(_),
        value,
        ..
    }] = members.as_slice()
    else {
        panic!()
    };
    assert_eq!(arm.patterns.len(), 2);
    assert!(matches!(node(&parsed, arm.body), ExprKind::Record { .. }));
    assert_eq!(line.segments, ["line"]);
    assert!(matches!(node(&parsed, *value), ExprKind::Arms(_)));

    let ItemKind::Impl {
        implements: Some(implements),
        members,
        ..
    } = &from.kind
    else {
        panic!()
    };
    assert_eq!(
        Source::new(input).slice(parsed.ty(*implements).unwrap().span),
        Some("From<Polar>")
    );
    assert!(matches!(
        members[0],
        ImplMember::Named { public: false, .. }
    ));
    module("type Marker {}\nimpl Poly {\n    [] => fault \"empty\",\n    ps => Self { pts: ps },\n}\nimpl P: T { Error = E, f = (a => 1, b => 2) }");
    // A member value is one arm; the next element is a constructor arm.
    let parsed = module("impl P { f = a => 1, b => 2 }");
    let [Entry::Item(item)] = parsed.entries.as_slice() else {
        panic!()
    };
    let ItemKind::Impl { members, .. } = &item.kind else {
        panic!()
    };
    assert!(matches!(
        members.as_slice(),
        [ImplMember::Named { .. }, ImplMember::Constructor(_)]
    ));
    assert_eq!(
        errors("impl P { f = a => 1 b => 2 }")[0].reason,
        Reason::ArmsNeedGroup
    );
}

#[test]
fn nested_generic_closers_have_separate_source_spans() {
    let input = "x: Vec<Vec<u8>> = values";
    let parsed = module(input);
    let [Entry::Line(id)] = parsed.entries.as_slice() else {
        panic!()
    };
    let ExprKind::Bind {
        annotation: Some(outer),
        value,
        ..
    } = node(&parsed, *id)
    else {
        panic!()
    };
    name(&parsed, *value, "values");
    let outer = parsed.ty(*outer).unwrap();
    assert_eq!(Source::new(input).slice(outer.span), Some("Vec<Vec<u8>>"));
    let TypeKind::Named { arguments, .. } = &outer.kind else {
        panic!()
    };
    let [inner] = arguments.as_slice() else {
        panic!()
    };
    let inner = parsed.ty(*inner).unwrap();
    assert_eq!(Source::new(input).slice(inner.span), Some("Vec<u8>"));
    assert_eq!(inner.span.end + 1, outer.span.end);
}

#[test]
fn function_types_nest_right_and_array_extents_are_preserved() {
    let input =
        "f: (T -> [u8; 4]) -> () -> Vec<Vec<T>> = g => g\nunit: () = ()";
    let parsed = module(input);
    let [Entry::Line(binding), Entry::Line(unit)] = parsed.entries.as_slice()
    else {
        panic!()
    };
    let ExprKind::Bind {
        annotation: Some(annotation),
        ..
    } = node(&parsed, *binding)
    else {
        panic!()
    };
    let TypeKind::Function { parameter, result } =
        &parsed.ty(*annotation).unwrap().kind
    else {
        panic!()
    };
    let TypeKind::Function {
        parameter: argument,
        result: array,
    } = &parsed.ty(*parameter).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        Source::new(input).slice(parsed.ty(*argument).unwrap().span),
        Some("T")
    );
    let TypeKind::Array {
        element,
        length,
        length_span,
    } = &parsed.ty(*array).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(*length, 4);
    assert_eq!(Source::new(input).slice(*length_span), Some("4"));
    assert_eq!(
        Source::new(input).slice(parsed.ty(*element).unwrap().span),
        Some("u8")
    );
    let TypeKind::Function {
        parameter: unit_parameter,
        result: vector,
    } = &parsed.ty(*result).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(parsed.ty(*unit_parameter).unwrap().kind, TypeKind::Unit);
    assert_eq!(
        Source::new(input).slice(parsed.ty(*vector).unwrap().span),
        Some("Vec<Vec<T>>")
    );
    let ExprKind::Bind {
        annotation: Some(ty),
        ..
    } = node(&parsed, *unit)
    else {
        panic!()
    };
    assert_eq!(parsed.ty(*ty).unwrap().kind, TypeKind::Unit);
    let (parsed, id) = only("f: A -> B -> C = x => x");
    let ExprKind::Bind {
        annotation: Some(ty),
        ..
    } = node(&parsed, id)
    else {
        panic!()
    };
    let TypeKind::Function { result, .. } = &parsed.ty(*ty).unwrap().kind
    else {
        panic!()
    };
    assert!(matches!(
        parsed.ty(*result).unwrap().kind,
        TypeKind::Function { .. }
    ));
}
