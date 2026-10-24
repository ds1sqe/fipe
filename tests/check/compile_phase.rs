//! The compile phase: tests on exact values run on the machine after
//! lowering, under the compile budget.
use fipe::{
    check::{self, Evaluated, Options},
    decl::Declarations,
    embed::Outcome,
    ir,
    lex::lex,
    parse::parse,
    session::{Diagnostic, Report, Session},
    typed::CheckReason,
    value::Reason,
};

const PRIME: [&str; 3] = [
    "trial = n d => if d * d > n { True } else { if n % d == 0 { False } else { trial n (d + 1) } }",
    "is_prime = n => n > 1 && trial n 2",
    "type Prime: Integer where is_prime self",
];

fn line(session: &mut Session, text: &str) -> Report {
    match session.line(text) {
        Outcome::Done(report) => report,
        Outcome::Request(request) => panic!("{text}: {request:?}"),
    }
}

fn session(lines: &[&str]) -> Session {
    let mut session = Session::new(Declarations::new());
    for text in lines {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}");
    }
    session
}

fn checked(
    source: &str,
    options: Options,
) -> Result<ir::Unit, Vec<check::Error>> {
    let module = parse(lex(source)).unwrap();
    check::check(&module, &Declarations::new(), &[], options)
}

#[test]
fn a_proved_entry_leaves_its_value_as_the_one_child_of_its_node() {
    let mut session = session(&PRIME);
    let report = line(&mut session, "p: Prime = 7");
    let seven = report.offset + "p: Prime = ".len();
    let unit = report.unit.unwrap();
    let tables = unit.tables();
    let literals: Vec<_> = (0..tables.nodes.len())
        .filter(|index| {
            tables.nodes[*index].span.start == seven
                && matches!(tables.nodes[*index].kind, ir::Kind::Literal { .. })
        })
        .collect();
    assert_eq!(literals.len(), 1, "one node holds the literal");
    let parents = tables
        .nodes
        .iter()
        .filter(|node| matches!(&node.kind, ir::Kind::Block(parts) if parts == &literals))
        .count();
    assert_eq!(parents, 1, "the proved node holds the literal");
}

#[test]
fn a_named_test_on_an_exact_value_is_proved_at_compile_time() {
    let mut session = session(&PRIME);
    let report = line(&mut session, "p: Prime = 7");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks, []);
    assert_eq!(report.settled.len(), 1, "{:?}", report.settled);
    assert_eq!(report.settled[0].check.name, "Prime");
    assert_eq!(report.settled[0].value, "7");
    let unit = report.unit.unwrap();
    assert_eq!(unit.plan, []);
    assert_eq!(session.checks(Some("p")), []);
    assert_eq!(session.settled(Some("p")).len(), 1);
}

#[test]
fn a_refuted_test_is_a_check_error_naming_the_limit_and_the_value() {
    let mut session = session(&PRIME);
    let report = line(&mut session, "p: Prime = 1");
    let [Diagnostic::Compile { span, what, reason }] =
        report.diagnostics.as_slice()
    else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(&"p: Prime = 1"[span.start..span.end], "Prime");
    assert_eq!(
        what,
        &Evaluated::Entry {
            limit: "Prime".into(),
            value: Some("1".into()),
        }
    );
    assert!(matches!(reason, Reason::Failed { .. }), "{reason:?}");
    assert!(report.unit.is_none());
    // The refused line leaves the session as it was.
    assert_eq!(session.lines().count(), 3);
    assert_eq!(line(&mut session, "p: Prime = 2").diagnostics, []);
    let report = line(&mut session, "q: Prime = 8");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile { what: Evaluated::Entry { limit, value: Some(value) }, .. }]
                if limit == "Prime" && value == "8"
        ),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn both_parts_of_a_limit_settle() {
    let mut session = session(&PRIME);
    let report = line(&mut session, "n: U8 & Prime = 13");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks, []);
    assert_eq!(report.settled.len(), 1);
    assert_ne!(line(&mut session, "m: U8 & Prime = 1").diagnostics, []);
}

#[test]
fn a_test_that_faults_has_failed() {
    let mut session = session(&[
        "tenth = n => if n == 0 { fault \"no tenth of zero\" } else { True }",
        "type Tenth: Integer where tenth self",
    ]);
    let report = line(&mut session, "t: Tenth = 0");
    let [Diagnostic::Compile { what, reason, .. }] =
        report.diagnostics.as_slice()
    else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(
        what,
        &Evaluated::Entry {
            limit: "Tenth".into(),
            value: Some("0".into()),
        }
    );
    assert_eq!(reason, &Reason::Message("no tenth of zero".into()));
    assert_eq!(line(&mut session, "t: Tenth = 3").diagnostics, []);
}

#[test]
fn an_exhausted_compile_budget_leaves_the_entry_open() {
    let mut session = session(&PRIME);
    session.compile_budget(1);
    let report = line(&mut session, "p: Prime = 7");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.settled, []);
    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.checks[0].reason, CheckReason::CompileBudget);
    assert_eq!(
        report.checks[0].reason.to_string(),
        "compile budget exhausted"
    );
    let unit = report.unit.unwrap();
    assert_eq!(unit.plan.len(), 1);
    assert_eq!(unit.plan[0].reason, ir::Reason::CompileBudget);
    // Open, so it is tested when it runs.
    let program = fipe::compile::compile(&unit);
    let mut machine = fipe::vm::Machine::new(Some(1 << 20));
    assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
}

#[test]
fn a_strict_session_refuses_only_what_the_phase_leaves_open() {
    let mut session = session(&PRIME);
    session.strict(true);
    assert_eq!(line(&mut session, "p: Prime = 7").diagnostics, []);
    session.compile_budget(1);
    assert_ne!(line(&mut session, "q: Prime = 7").diagnostics, []);
}

#[test]
fn a_file_settles_its_entries_and_refuses_a_refuted_one() {
    let prime = "is_prime: Integer -> Bool = n => n > 1\n\
                 type Prime: Integer where is_prime self\n";
    let unit =
        checked(&format!("{prime}p: Prime = 7"), Options::default()).unwrap();
    assert_eq!(unit.plan, []);
    let errors = checked(
        &format!("{prime}p: Prime = 7\nq: Prime = 0"),
        Options::default(),
    )
    .unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let exhausted = checked(
        &format!("{prime}p: Prime = 7"),
        Options {
            compile_budget: 1,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(exhausted.plan.len(), 1);
}

#[test]
fn no_compile_entry_reaches_a_body_once_settled() {
    let mut session = session(&PRIME);
    let report = line(&mut session, "p: Prime = 7");
    let unit = report.unit.unwrap();
    let program = fipe::compile::compile(&unit);
    let tables = unit.tables();
    for body in &program.bodies {
        for block in &body.blocks {
            for statement in &block.statements {
                if let fipe::mir::Statement::Enter { entry, .. } = statement {
                    assert_ne!(
                        tables.entries[*entry].phase,
                        ir::Phase::Compile
                    );
                }
            }
        }
    }
}

const MASK: [&str; 2] = [
    "type Mask: Literal { bits: U8 }",
    "impl Mask: Literal {\n    literal = {\n        Whole { value, radix: 2, digits: 8 } => Self value,\n        _ => fault \"a Mask is written as eight binary digits\",\n    },\n}",
];

const SIEVE: [&str; 2] = [
    "upto: Integer -> Integer -> Vec<Integer> = lo hi => if lo > hi { [] } else { ([lo] | to<Vec<Integer>>) ++ (upto (lo + 1) hi) }",
    "sieve = n => upto 2 n | filter is_prime",
];

fn run(unit: &ir::Unit) -> fipe::value::Value {
    let program = fipe::compile::compile(unit);
    match fipe::vm::Machine::new(Some(1 << 20)).run(&program) {
        Outcome::Done(Ok(value)) => value,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_program_reader_runs_at_compile_time_and_its_fault_is_a_check_error() {
    let mut session = session(&MASK);
    let report = line(&mut session, "m: Mask = 0b0000_1111");
    assert_eq!(report.diagnostics, []);
    let unit = report.unit.unwrap();
    assert!(!unit.tables().prepared.is_empty());
    let report = line(&mut session, "k: Mask = 15");
    let [Diagnostic::Compile { span, what, reason }] =
        report.diagnostics.as_slice()
    else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(&"k: Mask = 15"[span.start..span.end], "15");
    assert_eq!(
        what,
        &Evaluated::Literal {
            meaning: "Mask".into()
        }
    );
    assert_eq!(
        reason,
        &Reason::Message("a Mask is written as eight binary digits".into())
    );
    let report = line(&mut session, "k: Mask = 0b0000_0011\nk");
    assert_eq!(report.diagnostics, []);
    let value = run(&report.unit.unwrap());
    assert!(matches!(value, fipe::value::Value::Record(_)), "{value:?}");
}

#[test]
fn a_read_cast_of_a_literal_has_its_verdict_at_compile_time() {
    let mut session = session(&[]);
    let report = line(&mut session, "n: I64 = \"42\"\nn");
    assert_eq!(report.diagnostics, []);
    assert_eq!(run(&report.unit.unwrap()), fipe::value::Value::Int(42));
    let report = line(&mut session, "n: I64 = \"x\"");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile { what: Evaluated::Literal { meaning }, reason: Reason::Cast, .. }]
                if meaning == "Integer"
        ),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn a_compile_demand_is_prepared_and_exact_for_later_lines() {
    let mut lines = PRIME.to_vec();
    lines.extend(SIEVE);
    let mut session = session(&lines);
    let report = line(&mut session, "primes = Compile (sieve 100)");
    assert_eq!(report.diagnostics, []);
    let unit = report.unit.unwrap();
    // One machine runs a session's lines; its globals are their values.
    let mut machine = fipe::vm::Machine::new(Some(1 << 20));
    let program = fipe::compile::compile(&unit);
    assert!(matches!(machine.run(&program), Outcome::Done(Ok(_))));
    assert!(unit.tables().prepared.values().any(|constant| matches!(
        constant,
        ir::Constant::Sequence(parts) if parts.len() == 25
    )));
    let query = match session.query("primes") {
        Outcome::Done(report) => report,
        Outcome::Request(request) => panic!("{request:?}"),
    };
    let typed = query.typed.unwrap();
    let module = query.module.unwrap();
    let fipe::ast::Entry::Line(id) = module.entries.last().unwrap() else {
        panic!()
    };
    let known = typed.known(*id).unwrap();
    assert_eq!(
        known.exact.as_deref(),
        Some(
            "[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, \
             59, 61, 67, 71, 73, 79, 83, 89, 97]"
        )
    );
    let report = line(&mut session, "primes | len");
    let program = fipe::compile::compile(&report.unit.unwrap());
    assert_eq!(
        machine.run(&program),
        Outcome::Done(Ok(fipe::value::Value::Int(25)))
    );
}

#[test]
fn a_demand_that_faults_or_runs_out_is_a_check_error() {
    let mut session = session(&[]);
    let report = line(&mut session, "z = Compile (1 / ([0] | head))");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile {
                what: Evaluated::Demand,
                reason: Reason::DivideByZero,
                ..
            }]
        ),
        "{:?}",
        report.diagnostics
    );
    session.compile_budget(3);
    let report =
        line(&mut session, "f = n => n + 1\ny: Compile & Integer = f 7");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile {
                what: Evaluated::Demand,
                reason: Reason::Budget,
                ..
            }]
        ),
        "{:?}",
        report.diagnostics
    );
    session.compile_budget(1 << 20);
    let report =
        line(&mut session, "f = n => n + 1\ny: Compile & Integer = f 7");
    assert_eq!(report.diagnostics, []);
}

#[test]
fn a_compile_test_settles_at_compile_time_or_is_a_check_error() {
    let mut lines = PRIME.to_vec();
    lines.push("type Seed: Integer where Compile (is_prime self)");
    let mut session = session(&lines);
    let report = line(&mut session, "q: Seed = 7");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.settled.len(), 1);
    assert_ne!(line(&mut session, "q: Seed = 8").diagnostics, []);
    session.compile_budget(10);
    let report = line(&mut session, "q: Seed = 7");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile { what: Evaluated::Entry { limit, value: None }, reason: Reason::Budget, .. }]
                if limit == "Seed"
        ),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn a_float_demanded_at_compile_keeps_its_holding() {
    let mut session = session(&[]);
    let report = line(&mut session, "x: Compile & F64 = 2.0");
    assert_eq!(report.diagnostics, []);
    let typed = report.typed.unwrap();
    let module = report.module.unwrap();
    let fipe::ast::Entry::Line(id) = module.entries.last().unwrap() else {
        panic!()
    };
    let fipe::ast::ExprKind::Bind { value, .. } =
        module.expr(*id).unwrap().kind
    else {
        panic!()
    };
    let known = typed.known(value).unwrap();
    assert_eq!(known.holding.as_ref().unwrap().to_string(), "held Size<8>");
}

/// Run each line's unit on one machine, as a session's driver does; the
/// value of the last.
fn run_lines(session: &mut Session, lines: &[&str]) -> fipe::value::Value {
    let mut machine = fipe::vm::Machine::new(Some(1 << 20));
    let mut last = fipe::value::Value::Unit;
    for text in lines {
        let report = line(session, text);
        assert_eq!(report.diagnostics, [], "{text}");
        let program = fipe::compile::compile(&report.unit.unwrap());
        match machine.run(&program) {
            Outcome::Done(Ok(value)) => last = value,
            other => panic!("{text}: {other:?}"),
        }
    }
    last
}

const SORTED: [&str; 3] = [
    "step = { (b, Nothing) x => (b, Just x), (b, Just p) x => (if p <= x { b } else { False }, Just x) }",
    "is_sorted: Ord a => Vec<a> -> Bool = xs => { (b, _) = xs | fold (True, Nothing) step\n b }",
    "type Sorted<T: Ord>: Vec<T> where is_sorted self",
];

#[test]
fn a_limited_bracket_of_exact_elements_settles() {
    let mut session = session(&SORTED);
    let report = line(&mut session, "s: Sorted<Integer> = [1, 2, 3]");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.settled[0].value, "[1, 2, 3]");
    let report = line(&mut session, "t: Sorted<Integer> = [3, 1]");
    assert!(
        matches!(
            report.diagnostics.as_slice(),
            [Diagnostic::Compile { what: Evaluated::Entry { limit, value: Some(value) }, .. }]
                if limit == "Sorted" && value == "[3, 1]"
        ),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn a_program_sequence_runs_on_its_bracket() {
    let mut session = session(&[
        "type Ring<T, n>: Sequence<T> { slots: [T; n], at: Natural }",
        "impl Ring<U, k>: Sequence<U> { sequence = slots => Self slots 0 }",
        "type Pos: Sequence<Integer> { first: Integer }",
        "impl Pos: Sequence<Integer> { sequence = xs => if (xs | head) > 0 { Self (xs | head) } else { fault \"not positive\" } }",
    ]);
    let value = run_lines(
        &mut session,
        &["ring: Ring<Integer, 3> = [1, 2, 3]", "ring"],
    );
    let fipe::value::Value::Record(record) = value else {
        panic!("{value:?}")
    };
    assert_eq!(record.fields[1], fipe::value::Value::Int(0));
    // On exact elements the bracket is a constant, its Fault a check
    // error at the bracket.
    let report = line(&mut session, "p: Pos = [0]");
    let [Diagnostic::Compile { span, what, reason }] =
        report.diagnostics.as_slice()
    else {
        panic!("{:?}", report.diagnostics)
    };
    assert_eq!(&"p: Pos = [0]"[span.start..span.end], "[0]");
    assert_eq!(
        what,
        &Evaluated::Sequence {
            meaning: "Pos".into()
        }
    );
    assert_eq!(reason, &Reason::Message("not positive".into()));
    // On elements known when it runs, it runs then.
    let report = line(&mut session, "f = x => { r: Pos = [x]\n r }");
    assert_eq!(report.diagnostics, []);
}

#[test]
fn an_empty_sum_is_the_zero_of_its_meaning() {
    let mut session = session(&[
        "type Vec2: Add & Zero { x: F64, y: F64 }",
        "impl Vec2: Add { add = o self => Self (self.x + o.x) (self.y + o.y) }",
        "impl Vec2: Zero { zero = Self 0.0 0.0 }",
    ]);
    let zero = run_lines(&mut session, &["t: Vec2 = [] | sum", "t"]);
    let fipe::value::Value::Record(record) = zero else {
        panic!("{zero:?}")
    };
    assert_eq!(
        record.fields,
        [fipe::value::Value::F64(0.0), fipe::value::Value::F64(0.0)]
    );
    let report = line(&mut session, "c: Compile & Vec2 = [] | sum");
    assert_eq!(report.diagnostics, []);
    let unit = report.unit.unwrap();
    assert!(unit.tables().prepared.values().any(|constant| matches!(
        constant,
        ir::Constant::Record { fields, .. }
            if fields == &[ir::Constant::F64(0), ir::Constant::F64(0)]
    )));
}

#[test]
fn a_rounded_cast_stays_open_and_runs_to_its_value() {
    let mut session = session(&[]);
    let report = line(&mut session, "k: I64 = round 2.5");
    assert_eq!(report.diagnostics, []);
    assert!(report
        .checks
        .iter()
        .all(|check| check.reason == CheckReason::UnknownInput));
    let mut session = Session::new(Declarations::new());
    let value = run_lines(&mut session, &["k: I64 = round 2.5", "k"]);
    assert_eq!(value, fipe::value::Value::Int(2));
}

/// The lines before a refused one: a weak session meaning a later line
/// fixes, a generic function a later line adds dispatch rows to, and
/// the prime test.
fn before_refusal() -> Vec<&'static str> {
    let mut lines = alloc_lines();
    lines.extend(PRIME);
    lines
}

fn alloc_lines() -> Vec<&'static str> {
    vec!["[empty] = [[]]", "inc = n => n + 1"]
}

const RETRIED: &str = "fixed: [Integer; 0] = empty\nx = inc 2.0\n\
                       type T { a: Integer }\nt = T 1";

#[test]
fn a_refused_line_retried_without_its_error_is_as_on_a_fresh_session() {
    let mut refused = session(&before_refusal());
    let report = line(&mut refused, &format!("{RETRIED}\nq: Prime = 8"));
    assert!(
        matches!(report.diagnostics.as_slice(), [Diagnostic::Compile { .. }]),
        "{:?}",
        report.diagnostics
    );
    let retried = line(&mut refused, RETRIED);
    assert_eq!(retried.diagnostics, []);
    let mut fresh = session(&before_refusal());
    let first = line(&mut fresh, RETRIED);
    let (retried, first) = (retried.unit.unwrap(), first.unit.unwrap());
    assert_eq!(*retried.tables(), *first.tables());
    assert_eq!(retried.main, first.main);
    assert_eq!(retried.plan, first.plan);
    // The line number is reused: the retried line is the next line.
    assert!(retried
        .tables()
        .namespaces
        .contains(&format!("line {}", before_refusal().len() + 1)));
    assert!(!retried
        .tables()
        .namespaces
        .contains(&format!("line {}", before_refusal().len() + 2)));
}

#[test]
fn a_refused_line_leaves_earlier_units_unchanged() {
    let mut session = Session::new(Declarations::new());
    let mut earlier = None;
    for text in before_refusal() {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}");
        earlier = report.unit;
    }
    let earlier = earlier.unwrap();
    let before = earlier.tables().clone();
    let report = line(&mut session, &format!("{RETRIED}\nq: Prime = 8"));
    assert_ne!(report.diagnostics, []);
    assert_eq!(*earlier.tables(), before);
    assert_eq!(before.session_meanings, [None]);
}

#[test]
fn grouped_record_chains_keep_each_known_part_and_remain_compile_pure() {
    use fipe::{ast::Entry, limits::Phase};
    let declarations = Declarations::new();
    for (outer, inner) in [(9, 7), (7, 300)] {
        let prefix = format!("type Inner {{ n: Integer }}\ntype Outer {{ n: Integer, inner: Inner }}\nitem = Outer {outer} (Inner {inner})\n");
        let mut facts = Vec::new();
        for ending in [
            "item.inner.n".to_string(),
            "(item).inner.n".to_string(),
            format!("{inner}"),
        ] {
            let source = format!("{prefix}{ending}");
            let module = parse(lex(&source)).expect("record chain");
            let resolved = fipe::resolve::resolve(&module, &declarations, &[])
                .expect("resolved");
            let (typed, errors) = fipe::infer::infer_partial(
                &module,
                &resolved,
                &declarations,
                &[],
            );
            assert!(errors.is_empty(), "{errors:?}");
            let Entry::Line(value) = module.entries.last().expect("last line")
            else {
                panic!("expression")
            };
            let known = typed.known(*value).expect("checked value");
            assert_eq!(
                known.phase,
                Phase::Compile,
                "pure record projection stays compile phase"
            );
            facts.push((known.range.clone(), known.exact.clone()));
        }
        assert_eq!(facts[0], facts[2], "named chain retains inner value");
        assert_eq!(
            facts[1], facts[2],
            "grouped chain must not use the outer same-named field"
        );
        assert!(
            facts[2].0.is_some() || facts[2].1.is_some(),
            "comparison has actual exact numeric facts"
        );
    }
}

#[test]
fn grouped_intermediate_default_effect_is_not_compile_pure() {
    use fipe::{ast::Entry, limits::Phase};
    let mut declarations = Declarations::new();
    declarations.module("probe", "pub echo: Integer -> Integer\npub type B { pub n: Integer }\npub type A { pub seed: Integer }\nimpl A { pub step: (self: Self) -> (n: Integer = (echo 7)) -> B = self n => B n }").expect("Host member/default");
    for source in [
        "use probe\n(probe.A 0).step.n",
        "use probe\n(probe.A 0).step",
    ] {
        let module = parse(lex(source)).expect("grouped member");
        let resolved = fipe::resolve::resolve(&module, &declarations, &[])
            .expect("resolved");
        let (typed, errors) =
            fipe::infer::infer_partial(&module, &resolved, &declarations, &[]);
        assert!(errors.is_empty(), "{errors:?}");
        let Entry::Line(value) = module.entries.last().expect("last line")
        else {
            panic!("expression")
        };
        let span = module.expr(*value).expect("actual Field").span;
        let steps = &typed.accesses[&(span.start, span.end)];
        if source.ends_with(".step.n") {
            assert!(
                steps.iter().enumerate().any(|(index, step)| index + 1
                    < steps.len()
                    && step.bound.as_ref().is_some_and(|bound| bound
                        .defaults
                        .iter()
                        .any(|(_, default)| matches!(
                            default,
                            fipe::typed::DefaultValue::Expression(_)
                        )))),
                "actual intermediate default"
            );
        } else {
            let bound = &typed.bound[&(span.start, span.end)];
            assert!(!bound.spots.is_empty(), "actual final call Bound");
            let [(spot, fipe::typed::DefaultValue::Declared { origin, value })] =
                bound.defaults.as_slice()
            else {
                panic!(
                    "final method retains the ordinary Declared default policy"
                )
            };
            let instance =
                &typed.default_instances[&(span.start, span.end, *spot)];
            assert_eq!(
                &instance.origin, origin,
                "exact receiving-call default owner"
            );
            assert_eq!(
                &instance.value, value,
                "exact receiving-call default expression"
            );
            assert_ne!(
                instance
                    .typed
                    .known(instance.value)
                    .expect("actual default facts")
                    .phase,
                Phase::Compile,
                "the retained default itself requests Host work"
            );
        }
        assert_ne!(
            typed.known(*value).expect("checked chain").phase,
            Phase::Compile,
            "Host default must remain runtime work"
        );
    }
}

#[test]
fn declared_written_signature_labels_keep_qualified_types_and_lazy_defaults() {
    let mut declarations = Declarations::new();
    declarations.module("probe", "pub keep: Eq a => (seed: a) -> (copy: a = seed) -> a = seed copy => copy\npub unused: (value: Integer = missing) -> Integer = value => value").expect("labeled written declarations");
    let module =
        parse(lex("use probe\nprobe.keep 7")).expect("qualified default use");
    check::check(&module, &declarations, &[], Options::default())
        .expect("earlier parameter default and untouched invalid default");
    let module =
        parse(lex("use probe\nprobe.unused")).expect("reach invalid default");
    let errors = check::check(&module, &declarations, &[], Options::default())
        .expect_err("invalid default is checked when needed");
    assert!(errors.iter().any(|error| matches!(error, check::Error::Type(error) if matches!(&error.reason, fipe::infer::Reason::Default { name, .. } if name == "value"))), "{errors:?}");
}

#[test]
fn declared_signature_overlay_keeps_refined_spots_and_body_arity_errors() {
    let mut declarations = Declarations::new();
    declarations.module("probe", "pub narrow: (seed: U8) -> (copy: U8 = seed) -> U8 = x y => y\npub bad: (value: Integer) -> Integer = x y => x").expect("declaration signatures");
    let module =
        parse(lex("use probe\nprobe.narrow 7")).expect("healthy refined use");
    check::check(&module, &declarations, &[], Options::default())
        .expect("checked refined spot and lazy unrelated bad body");
    let module =
        parse(lex("use probe\nprobe.narrow 300")).expect("bad refined use");
    assert!(
        check::check(&module, &declarations, &[], Options::default()).is_err(),
        "overlay must retain the actual U8 checked type"
    );
    let module = parse(lex("use probe\nprobe.bad 7"))
        .expect("reach mismatched written arity");
    let errors = check::check(&module, &declarations, &[], Options::default())
        .expect_err("body arity mismatch remains a diagnostic");
    assert!(errors.iter().any(|error| matches!(error, check::Error::Type(error) if matches!(error.reason, fipe::infer::Reason::DeclarationValue { .. }))), "{errors:?}");
}
