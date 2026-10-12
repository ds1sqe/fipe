use fipe::{
    decl::Declarations,
    embed::Outcome,
    session::{Report, Session},
};

fn line(session: &mut Session, text: &str) -> Report {
    match session.line(text) {
        Outcome::Done(report) => report,
        other @ Outcome::Request(_) => panic!("{other:?}"),
    }
}

#[test]
fn tuples_type_each_part_and_preserve_destructured_facts() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "pair: Integer * String = (1, \"one\")",
        "(n, name) = pair",
        "ends: U8 * U8 = (0, 255)",
        "(lo, hi) = ends",
        "last: U8 = hi",
        "swap = (a, b) => (b, a)",
        "nested: (Integer * String) * Integer = ((1, \"one\"), 2)",
        "equal = (1, \"one\") == (2, \"two\")",
    ] {
        let report = line(&mut session, text);
        assert!(
            report.diagnostics.is_empty(),
            "{text}: {:?}",
            report.diagnostics
        );
        assert!(report.checks.is_empty(), "{text}: {:?}", report.checks);
    }
    for text in [
        "bad: U8 * U8 = (0, 256)",
        "bad: Integer * String = (1, 2, 3)",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn call_readings_warn_without_rejecting_and_double_grouping_is_explicit() {
    let mut session = Session::new(Declarations::new());
    assert_eq!(line(&mut session, "g = x (y = 0) => x").diagnostics, []);
    let report = line(&mut session, "g (1, 2)");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(line(&mut session, "g(1, 2)").warnings.len(), 1);
    assert_eq!(line(&mut session, "g ((1, 2))").warnings, []);
    assert_eq!(line(&mut session, "divmod: Integer -> Integer -> Integer * Integer = a b => (a / b, a % b)").diagnostics, []);
    let report = line(&mut session, "divmod (7, 2)");
    assert!(
        report.diagnostics.iter().any(|d| matches!(
            d,
            fipe::session::Diagnostic::Type(fipe::infer::Error {
                reason: fipe::infer::Reason::CallReading { .. },
                ..
            })
        )),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn type_parameters_and_number_sums_are_checked() {
    use fipe::types::{Subst, Type};
    let mut subst = Subst::default();
    let n = Type::Var(subst.fresh());
    let m = Type::Var(subst.fresh());
    let left = Type::sum(vec![n.clone(), m.clone(), Type::Number(2.into())]);
    let right = Type::sum(vec![m, n.clone(), Type::Number(2.into())]);
    subst.unify(&left, &right).unwrap();
    subst
        .unify(
            &Type::sum(vec![n.clone(), Type::Number(2.into())]),
            &Type::Number(5.into()),
        )
        .unwrap();
    assert_eq!(subst.apply(&n), Type::Number(3.into()));
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Pair<A, B> { first: A, second: B }",
        "pair = Pair 1 \"a\"",
        "type Ring<T, n> { slots: [T; n] }",
        "same: [a; n + m] -> [a; m + n] = x => x",
    ] {
        let report = line(&mut session, text);
        assert!(
            report.diagnostics.is_empty(),
            "{text}: {:?}",
            report.diagnostics
        );
    }
    for text in [
        "bad: Ring<Integer, String> = 1",
        "bad: Pair<8, String> = 1",
        "bad: [a; a] -> a = x => x",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn brackets_arrays_and_collection_words() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "xs = [1, 2, 3]",
        "zs = xs ++ [4, 5]",
        "v = [1, 2.5]",
        "rgb: [U8; 3] = [255, 128, 0]",
        "ys = xs | map (x => x + 1)",
        "filtered = xs | filter (x => x > 0)",
        "total = xs | sum",
        "vector: Vec<Integer> = [1, 2, 3]",
        "vector | sum",
    ] {
        let report = line(&mut session, text);
        assert!(
            report.diagnostics.is_empty(),
            "{text}: {:?}",
            report.diagnostics
        );
        assert!(report.checks.is_empty(), "{text}: {:?}", report.checks);
    }
}

#[test]
fn program_sequence_claim_and_limited_bracket() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Ring<T, n>: Sequence<T> { slots: [T; n], at: Natural }",
        "impl Ring<U, k>: Sequence<U> { sequence = slots => Self slots 0 }",
        "ring: Ring<Integer, 3> = [1, 2, 3]",
        "converted = [1, 2, 3] | to<Vec<Integer>>",
        "is_sorted: Ord a => Vec<a> -> Bool = xs => True",
        "type Sorted<T: Ord>: Vec<T> where is_sorted self",
    ] {
        let report = line(&mut session, text);
        assert!(
            report.diagnostics.is_empty(),
            "{text}: {:?}",
            report.diagnostics
        );
    }
    let report = line(&mut session, "s: Sorted<Integer> = [1, 2, 3]");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.checks, [], "{:?}", report.checks);
    assert_eq!(report.settled.len(), 1, "{:?}", report.settled);
    assert_eq!(report.settled[0].value, "[1, 2, 3]");
    assert_ne!(
        line(&mut session, "bad: Ring<Integer, 3> = [1, 2]").diagnostics,
        []
    );
    assert_ne!(
        line(&mut session, "bad: Sorted<Integer -> Integer> = []").diagnostics,
        []
    );
}

#[test]
fn sum_asks_zero_and_fields_ask_default() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Vec2: Add & Zero { x: F64, y: F64 }",
        "impl Vec2: Add { add = o self => Self (self.x + o.x) (self.y + o.y) }",
        "impl Vec2: Zero { zero = Self 0.0 0.0 }",
        "t: Vec2 = [] | sum",
        "type Counter { n: Integer }",
    ] {
        let report = line(&mut session, text);
        assert!(
            report.diagnostics.is_empty(),
            "{text}: {:?}",
            report.diagnostics
        );
    }
    assert_ne!(line(&mut session, "bad: Counter = Counter").diagnostics, []);
    assert_eq!(
        line(&mut session, "type OnlyAdd: Add { n: Integer }").diagnostics,
        []
    );
    assert_eq!(
        line(
            &mut session,
            "impl OnlyAdd: Add { add = o self => Self (o.n + self.n) }"
        )
        .diagnostics,
        []
    );
    assert_ne!(
        line(&mut session, "bad: OnlyAdd = [] | sum").diagnostics,
        []
    );
}

#[test]
fn number_kinds_length_equations_and_exact_lengths() {
    use fipe::types::{Subst, Type};
    let mut subst = Subst::default();
    let n = Type::Var(subst.fresh());
    let m = Type::Var(subst.fresh());
    subst
        .unify(
            &Type::sum(vec![n.clone(), m.clone()]),
            &Type::Number(5.into()),
        )
        .unwrap();
    assert!(subst.settle_lengths().unwrap());
    subst.unify(&n, &Type::Number(2.into())).unwrap();
    assert!(!subst.settle_lengths().unwrap());
    assert_eq!(subst.apply(&m), Type::Number(3.into()));
    assert!(subst
        .unify(
            &Type::sum(vec![m, Type::Number(3.into())]),
            &Type::Number(2.into())
        )
        .is_err());
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Bad<n> { value: n }",
        "type Bad<T> { value: T<Integer> }",
        "type Bad<T> { value: [Integer; T] }",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
    for text in [
        "xs = [1, 2, 3]",
        "n: U8 = xs.len",
        "m: U8 = xs | len",
        "ends: U8 * U8 = (0, 255)",
        "rgb: [U8; 3] = [255, 128, 0]",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}");
        assert_eq!(report.checks, [], "{text}");
    }
}

#[test]
fn claim_members_quantify_accumulators_and_lengths() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Bag<T>: Items<T> { value: T }",
        "impl Bag<T>: Items<T> { fold = seed f self => f seed self.value }",
        "bag = Bag 2",
        "a = bag | fold 0 (n x => n + x)",
        "b = bag | fold True (b x => b)",
        "type Fixed: Items<Integer> {}",
        "type Short: Sequence<Integer> {}",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    for text in [
        "impl Fixed: Items<Integer> { fold = seed f self => 0 }",
        "impl Short: Sequence<Integer> { sequence: [Integer; 2] -> Self = xs => Self }",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn parameterized_holdings_are_checked_when_arguments_are_known() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Held<T, n>: Size<n> { value: T }",
        "ok: Held<U8, 1> = Held 255",
        "type Small<T>: Size<1> { value: T }",
        "small: Small<U8> = Small 2",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    for text in [
        "bad: Held<U16, 1> = Held 256",
        "bad: Small<Integer> = Small 2",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn container_schemes_packing_and_patterns() {
    let mut session = Session::new(Declarations::new());
    for text in [
        "type Pair<A, B> { first: A, second: B }",
        "impl Pair { first_value = self => self.first }",
        "first = (Pair 1 True).first_value",
        "type Shape where self.area: F64",
        "type Circle: Shape { radius: F64 }",
        "impl Circle: Shape { area = self => self.radius }",
        "type Square: Shape { side: F64 }",
        "impl Square: Shape { area = self => self.side }",
        "shapes: Vec<Shape> = [Circle 1.0, Square 2.0]",
        "xs = [1, 2, 3]",
        "split = [a, b] => (a, b)",
        "type Vec2: Add & Zero & Default { x: F64, y: F64 }",
        "impl Vec2: Add { add = o self => Self (o.x + self.x) (o.y + self.y) }",
        "impl Vec2: Zero { zero = Self 0.0 0.0 }",
        "impl Vec2: Default { default = Self.zero }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    for (text, expected) in [
        ("Pair 1 \"a\"", "Pair<Integer, String>"),
        ("xs", "[Integer; 3]"),
        ("xs ++ [4, 5]", "[Integer; 5]"),
        ("[1, 2.5]", "[F64; 2]"),
        ("xs | map (x => x > 0)", "[Bool; 3]"),
        ("xs | filter (x => x > 0)", "Vec<Integer>"),
        ("split", "[a; 2] -> a * a"),
        ("(a, b) => (b, a)", "a * b -> b * a"),
        ("Vec2.default", "Vec2"),
    ] {
        let Outcome::Done(report) = session.query(text) else {
            panic!("query")
        };
        assert_eq!(report.diagnostics, [], "{text}");
        let [fipe::ast::Entry::Line(id)] = report.entries() else {
            panic!("line")
        };
        assert_eq!(
            report
                .typed
                .as_ref()
                .unwrap()
                .binding(*id)
                .unwrap()
                .to_string(),
            expected,
            "{text}"
        );
    }
}
