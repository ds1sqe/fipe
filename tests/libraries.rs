//! Pure libraries use the same file-module path as application code.
use fipe::{
    check, compile,
    decl::Declarations,
    embed::Outcome,
    lex::lex,
    parse::parse,
    value::{Reason, Value},
    vm::{Fault, Machine},
};

fn evaluate(source: &str) -> Outcome<Result<Value, Fault>> {
    let files = [
        ("math", include_str!("../library/math.fipe")),
        ("stats", include_str!("../library/stats.fipe")),
    ]
    .into_iter()
    .map(|(name, text)| (name.into(), parse(lex(text)).unwrap()))
    .collect::<Vec<_>>();
    let main = parse(lex(source)).unwrap();
    let unit = check::check(
        &main,
        &Declarations::new(),
        &files,
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
    let program = compile::compile(&unit);
    Machine::new(Some(100_000)).run(&program)
}

fn run(source: &str) -> Value {
    match evaluate(source) {
        Outcome::Done(Ok(value)) => value,
        other => panic!("{source}: {other:?}"),
    }
}

#[test]
fn statistics_match_reference_values_and_small_sample_rules() {
    for (expression, expected) in [
        ("stats.mean [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]", 5.0),
        (
            "stats.variance [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]",
            4.0,
        ),
        ("stats.stddev [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]", 2.0),
        ("stats.sample_variance [1.0, 2.0, 3.0]", 1.0),
        ("stats.sample_stddev [1.0, 2.0, 3.0]", 1.0),
        (
            "stats.sample_variance [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]",
            32.0 / 7.0,
        ),
        (
            "stats.sample_stddev [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]",
            (32.0_f64 / 7.0).sqrt(),
        ),
        ("stats.mean [7.0]", 7.0),
        ("stats.mean [7.0, 7.0, 7.0]", 7.0),
        ("stats.variance [7.0]", 0.0),
        ("stats.stddev [7.0]", 0.0),
        ("stats.variance [7.0, 7.0, 7.0]", 0.0),
        ("stats.sample_variance [7.0, 7.0, 7.0]", 0.0),
        ("stats.sample_stddev [7.0, 7.0, 7.0]", 0.0),
        ("stats.mean [1e308, 1e308]", 1e308),
        ("stats.variance [1e308, 1e308]", 0.0),
        (
            "stats.mean [1000000001.0, 1000000002.0, 1000000003.0]",
            1_000_000_002.0,
        ),
        (
            "stats.variance [1000000001.0, 1000000002.0, 1000000003.0]",
            2.0 / 3.0,
        ),
        (
            "stats.sample_variance [1000000001.0, 1000000002.0, 1000000003.0]",
            1.0,
        ),
    ] {
        let Value::F64(actual) =
            run(&format!("use stats\n({expression}) || -1.0"))
        else {
            panic!("expected F64");
        };
        assert!((actual - expected).abs() < 1e-12, "{expression}: {actual}");
    }
    for expression in [
        "stats.mean []",
        "stats.variance []",
        "stats.stddev []",
        "stats.sample_variance []",
        "stats.sample_variance [7.0]",
        "stats.sample_stddev []",
        "stats.sample_stddev [7.0]",
    ] {
        assert_eq!(
            run(&format!("use stats\n({expression}) || -1.0")),
            Value::F64(-1.0)
        );
    }
}

#[test]
fn interpolation_and_dot_cover_endpoints_and_length_mismatch() {
    for (expression, expected) in [
        ("math.lerp 10.0 20.0 0.25", 12.5),
        ("math.lerp 10.0 20.0 2.0", 30.0),
        ("math.lerp 10.0 20.0 -1.0", 0.0),
        ("math.lerp 1.0 1.0 1e16", 1.0),
        ("math.lerp 1.0 1.0 -1e16", 1.0),
        ("math.lerp 1e308 1e308 2.0", 1e308),
        ("math.lerp -1e308 -1e308 -2.0", -1e308),
        ("math.lerp -1e308 1e308 0.0", -1e308),
        ("math.lerp -1e308 1e308 1.0", 1e308),
        ("math.lerp -1e308 1e308 0.5", 0.0),
        ("math.dot [1.0, 2.0] [3.0, 4.0] || -1.0", 11.0),
        ("math.dot [] [] || -1.0", 0.0),
        ("math.dot [1.0] [] || -1.0", -1.0),
        ("math.dot [] [1.0] || -1.0", -1.0),
    ] {
        assert_eq!(
            run(&format!("use math\n{expression}")),
            Value::F64(expected)
        );
    }
}

#[test]
fn examples_check_and_run() {
    let Value::Tuple(summary) = run(include_str!("../library/summary.fipe"))
    else {
        panic!("expected summary tuple");
    };
    assert_eq!(summary.len(), 3);
    for (value, expected) in summary.iter().zip([5.0, 4.0, 2.0]) {
        assert_just(value, expected);
    }
    let Value::Tuple(interpolation) =
        run(include_str!("../library/interpolate.fipe"))
    else {
        panic!("expected interpolation tuple");
    };
    assert_eq!(interpolation.len(), 2);
    assert_eq!(interpolation[0], Value::F64(12.5));
    assert_just(&interpolation[1], 11.0);
}

fn assert_just(value: &Value, expected: f64) {
    let Value::Record(record) = value else {
        panic!("expected Just, got {value:?}");
    };
    // Just is the first constructor in the prelude's Maybe declaration.
    assert_eq!(record.constructor, 0);
    assert_eq!(record.fields, [Value::F64(expected)]);
}

#[test]
fn intermediate_overflow_faults_at_runtime() {
    for source in [
        "use math\nmath.lerp -1e308 1e308 2.0",
        "use math\nmath.dot [1e308] [2.0]",
        "use math\nmath.dot [1e308, 1e308] [1.0, 1.0]",
        "use stats\nstats.mean [-1e308, 1e308]",
        "use stats\nstats.variance [0.0, 1e308]",
        "use stats\nstats.sample_variance [0.0, 1e308]",
        "use stats\nstats.stddev [0.0, 1e308]",
        "use stats\nstats.sample_stddev [0.0, 1e308]",
    ] {
        match evaluate(source) {
            Outcome::Done(Err(fault)) => {
                assert_eq!(fault.reason, Reason::NotFinite, "{source}");
            }
            other => panic!(
                "{source}: expected runtime NotFinite Fault, got {other:?}"
            ),
        }
    }
}
