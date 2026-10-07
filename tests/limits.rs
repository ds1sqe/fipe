use fipe::token::Magnitude;

#[test]
fn exact_magnitude_arithmetic_crosses_limb_boundaries() {
    let max = Magnitude::from(u64::MAX);
    let one = Magnitude::from(1);
    let next = max.add(&one);
    assert_eq!(next.to_string(), "18446744073709551616");
    assert_eq!(next.checked_sub(&one), Some(max.clone()));
    assert_eq!(one.checked_sub(&next), None);
    let square = max.multiply(&max);
    assert_eq!(
        square.to_string(),
        "340282366920938463426481119284349108225"
    );
    assert_eq!(square.div_rem(&max), Some((max, Magnitude::default())));
    assert_eq!(
        next.div_rem(&Magnitude::from(3)),
        Some((Magnitude::from(6_148_914_691_236_517_205), one))
    );
    assert!(next.div_rem(&Magnitude::default()).is_none());
}

fn line(
    session: &mut fipe::session::Session,
    text: &str,
) -> fipe::session::Report {
    match session.line(text) {
        fipe::embed::Outcome::Done(report) => report,
        other @ fipe::embed::Outcome::Request(_) => panic!("{other:?}"),
    }
}

#[test]
fn integer_entries_use_facts_and_keep_limits_out_of_variables() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["a: U8 = 200", "b: U8 = 100", "c: U8 = (a + b) / 2"] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
        assert_eq!(report.typed.unwrap().checks, [], "{text}");
    }
    assert_ne!(line(&mut session, "d: U8 = a + b").diagnostics, []);
    let report = line(&mut session, "sum2: U8 -> U8 -> U8 = x y => x + y");
    assert_eq!(report.diagnostics, [], "{:?}", report.diagnostics);
    let checks = report.typed.unwrap().checks;
    assert_eq!(checks.len(), 1, "{checks:?}");
    assert_eq!(
        checks[0].known.range.as_ref().unwrap().hi.to_string(),
        "510"
    );
    assert_eq!(line(&mut session, "d = x => x + x").diagnostics, []);
    let report = line(&mut session, "w: U8 = d a");
    assert_eq!(report.diagnostics, [], "{:?}", report.diagnostics);
    assert_ne!(report.typed.unwrap().checks, []);
}

#[test]
fn strict_session_rejects_open_entries_and_reports_plain_entries() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    let report = line(&mut session, "sum2: U8 -> U8 -> U8 = x y => x + y");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks.len(), 1);
    assert_eq!(session.checks(Some("sum2")).len(), 1);
    session.strict(true);
    assert_ne!(
        line(&mut session, "sum3: U8 -> U8 -> U8 = x y => x + y").diagnostics,
        []
    );
    assert_eq!(line(&mut session, "n: U8 = 200").diagnostics, []);
}

#[test]
fn user_tests_and_gates_check_the_meaning_and_entry() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(
            &mut session,
            "type Small: Integer where 0 <= self, self < 10"
        )
        .diagnostics,
        []
    );
    assert_eq!(line(&mut session, "a: Small = 3").diagnostics, []);
    assert_ne!(line(&mut session, "b: Small = 20").diagnostics, []);
    assert_eq!(line(&mut session, "Small 3").diagnostics, []);
    assert_ne!(line(&mut session, "Small 20").diagnostics, []);
    assert_ne!(line(&mut session, "20 | Small").diagnostics, []);
    assert_ne!(line(&mut session, "300 | to<U8>").diagnostics, []);
    assert_eq!(line(&mut session, "300 | try<U8>").diagnostics, []);
    assert_ne!(line(&mut session, "U8 2.0").diagnostics, []);
}

#[test]
fn intersections_unions_and_negations_use_one_meaning() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Small: Integer where self < 10",
        "type Positive: Integer where 0 < self",
        "a: Small & Positive = 3",
        "b: Small | Positive = 20",
        "c: !Small = 20",
        "type Large: Integer & !Small",
        "type Handy: Small | Positive",
        "h: Handy = 20",
        "d: Large = 20",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
        assert_eq!(report.checks, [], "{text}: {:?}", report.checks);
    }
    assert_ne!(
        line(&mut session, "e: Small & Positive = 20").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "f: !Small = 3").diagnostics, []);
}

#[test]
fn record_parts_lengths_and_division_are_checked() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Range { lo: Integer, hi: Integer } where self.lo <= self.hi",
        "Range 1 2",
        "type Name: String where 0 < self.len, self.len <= 64",
        "who: Name = \"fipe\"",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
        assert_eq!(report.checks, [], "{text}: {:?}", report.checks);
    }
    assert_ne!(line(&mut session, "Range { lo: 2, hi: 1 }").diagnostics, []);
    assert_ne!(line(&mut session, "3 / 0").diagnostics, []);
    assert_eq!(
        line(
            &mut session,
            "type Digit: Integer where 0 <= self, self <= 5"
        )
        .diagnostics,
        []
    );
    let report = line(
        &mut session,
        "div: Integer -> Digit -> Integer = x y => x / y",
    );
    assert_eq!(report.diagnostics, [], "{:?}", report.diagnostics);
    assert_eq!(report.checks.len(), 1);
}

#[test]
fn explicit_conversions_keep_the_written_target_and_allow_fallible_failure() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["try<U8> 300", "to<U8> 3", "read<U8> \"3\""] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    assert_ne!(line(&mut session, "to<U8> 300").diagnostics, []);
    session.strict(true);
    let report = line(&mut session, "g: Integer -> U8 = n => U8 n");
    assert_eq!(report.diagnostics, [], "{:?}", report.diagnostics);
}

#[test]
fn gates_cover_with_their_declared_complement_and_keep_passed_names() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["is_prime = n => n > 1", "type Prime: Integer where is_prime self", "type NonPrime: Integer & !Prime", "kind = n => n | { Prime p => \"prime\", NonPrime q => \"other\" }", "classify = n => n | { Prime p => \"prime\", U8 b => \"small\", _ => \"big\" }"] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    let report = line(&mut session, "p: Prime = 7");
    assert_eq!(report.diagnostics, []);
    assert_eq!(
        report.checks[0].reason,
        fipe::typed::CheckReason::CompileTimeEvaluation
    );
    assert_ne!(
        line(&mut session, "type Loop: Integer where (Loop self) == self")
            .diagnostics,
        []
    );
    assert_ne!(
        line(
            &mut session,
            "type Loop: Integer where (to<Loop> self) == self"
        )
        .diagnostics,
        []
    );
    assert_ne!(
        line(&mut session, "type Odd where self % 2 == 1").diagnostics,
        []
    );
}

#[test]
fn entries_reach_container_parts_and_payloads() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "xs: Vec<Integer> = [1, 300]").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "ys: Vec<U8> = xs").diagnostics, []);
    assert_eq!(line(&mut session, "m = Just 300").diagnostics, []);
    assert_ne!(line(&mut session, "n: Maybe<U8> = m").diagnostics, []);
    assert_eq!(line(&mut session, "empty: Vec<U8> = []").checks, []);
    session.strict(true);
    let empty = line(&mut session, "nothing: Maybe<U8> = Nothing");
    assert_eq!(empty.diagnostics, []);
    assert_eq!(empty.checks, []);
}

#[test]
fn strict_checks_include_imported_modules_and_file_facts_keep_identity() {
    use fipe::embed::{Outcome, Reply};
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    session.strict(true);
    assert!(matches!(session.line("use numbers"), Outcome::Request(_)));
    let Outcome::Done(report) = session.reply(Reply::File(Some(
        "pub sum2: U8 -> U8 -> U8 = x y => x + y".into(),
    ))) else {
        panic!()
    };
    assert!(report.diagnostics.iter().any(|d| matches!(d, fipe::session::Diagnostic::FileType { stem, .. } if stem == "numbers")));
    assert!(report.checks.iter().any(
        |c| c.place_origin == fipe::resolve::Origin::File("numbers".into())
    ));
}

#[test]
fn text_length_counts_utf8_bytes_and_branch_narrowing_stays_open() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Tiny: String where self.len <= 2").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "t: Tiny = \"éa\"").diagnostics, []);
    assert_eq!(line(&mut session, "flag = True").diagnostics, []);
    let report = line(&mut session, "n: U8 = if flag { 2 } else { 200 }");
    assert_eq!(report.diagnostics, []);
    assert_ne!(report.checks, []);
    assert!(report
        .checks
        .iter()
        .all(|c| c.reason == fipe::typed::CheckReason::Narrowing));
}

#[test]
fn reordered_constructor_arguments_and_piped_spots_enter_the_right_place() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(
            &mut session,
            "type Range { lo: Integer, hi: Integer } where self.lo <= self.hi"
        )
        .diagnostics,
        []
    );
    assert_ne!(line(&mut session, "Range -hi=1 -lo=2").diagnostics, []);
    assert_eq!(line(&mut session, "Range -hi=2 -lo=1").diagnostics, []);
    assert_eq!(
        line(&mut session, "take: U8 -> Integer = n => n").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "300 | take").diagnostics, []);
}

#[test]
fn interval_arithmetic_contains_all_small_integer_results() {
    use fipe::{
        ast::BinaryOp as B,
        limits::known::{End, Integer, Range},
    };
    let end = |n: i64| {
        End::Finite(Integer::new(n < 0, Magnitude::from(n.unsigned_abs())))
    };
    for lo in -3..=3 {
        for hi in lo..=3 {
            for divisor_lo in -3..=3 {
                for divisor_hi in divisor_lo..=3 {
                    let a = Range {
                        lo: end(lo),
                        hi: end(hi),
                    };
                    let b = Range {
                        lo: end(divisor_lo),
                        hi: end(divisor_hi),
                    };
                    for op in [
                        B::Add,
                        B::Subtract,
                        B::Multiply,
                        B::Divide,
                        B::Remainder,
                    ] {
                        let range = a.arithmetic(op, &b);
                        for x in lo..=hi {
                            for y in divisor_lo..=divisor_hi {
                                let value = match op {
                                    B::Add => x + y,
                                    B::Subtract => x - y,
                                    B::Multiply => x * y,
                                    B::Divide if y != 0 => x / y,
                                    B::Remainder if y != 0 => x % y,
                                    _ => continue,
                                };
                                assert!(range.lo <= end(value) && end(value) <= range.hi, "{a:?} {op:?} {b:?}: {range:?} excludes {value}");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn declared_default_values_enter_their_spots() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.commands("pub bad: (n: U8 = 300) -> Integer\npub good: (n: U8 = 3) -> Integer\npub open: (a: Integer) -> (n: U8 = a) -> Integer\n").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert_ne!(line(&mut session, "bad").diagnostics, []);
    assert_eq!(line(&mut session, "good").diagnostics, []);
    let open = line(&mut session, "open 3");
    assert_eq!(open.diagnostics, []);
    assert_ne!(open.checks, []);
    session.strict(true);
    assert_ne!(line(&mut session, "open 3").diagnostics, []);
}

#[test]
fn entry_places_keep_names_and_precise_result_and_argument_spans() {
    use fipe::{token::Source, typed::Place};
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    let result = line(&mut session, "sum2: U8 -> U8 -> U8 = x y => x + y");
    assert_eq!(result.checks.len(), 1);
    assert_eq!(result.checks[0].place_kind, Place::Result);
    assert_eq!(
        Source::new(session.source()).slice(result.checks[0].place),
        Some("U8")
    );
    assert_eq!(
        line(&mut session, "take: U8 -> Integer = amount => amount")
            .diagnostics,
        []
    );
    let call = line(&mut session, "use_it = x => take -amount=x");
    assert_eq!(call.diagnostics, []);
    assert_ne!(call.checks, []);
    for check in call.checks {
        assert_eq!(check.place_kind, Place::Parameter(Some("amount".into())));
        assert_eq!(Source::new(session.source()).slice(check.place), Some("x"));
    }
}

#[test]
fn timing_demands_and_host_inputs() {
    use fipe::typed::Phase;
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("cli", "pub port: Integer\npub seed: Integer")
        .unwrap();
    declarations
        .module("config", "pub max_port = 65535")
        .unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli config",
        "x = Compile (2 + 3)",
        "y: Init & Integer = cli.port",
        "f = n => n + 1",
        "z = Compile (f 2)",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    for text in [
        "Compile",
        "map Compile [1]",
        "f = Compile x => x",
        "Compile (cli.port)",
        "f = n => Init (cli.port)",
        "x: Compile & Integer = cli.port",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
    let report = line(&mut session, "x");
    let fipe::ast::Entry::Line(id) = report.module.as_ref().unwrap().entries[0]
    else {
        panic!()
    };
    assert_eq!(
        report.typed.unwrap().known(id).unwrap().phase,
        Phase::Compile
    );
}

#[test]
fn holding_fit_and_agreement() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Flag: Bool & Bits<1>",
        "type Mode: Size<1> { read: Flag, write: Flag, exec: Flag }",
        "type Pixel { rgb: [U8; 3] }",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    for text in [
        "type Odd8: Integer & Bits<8>",
        "b: Bool & Bits<8> & Bits<16> = True",
        "type Nib: U8 & Bits<4>",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn timed_tests_keep_three_phases_and_never_become_tests_themselves() {
    use fipe::typed::{CheckReason, Phase};
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("cli", "pub port: Integer\npub seed: Integer")
        .unwrap();
    declarations
        .module("config", "pub max_port = 65535")
        .unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli config",
        "is_prime = n => n > 1",
        "type Seed: Integer where Compile (is_prime self)",
        "type Port: Integer where 0 < self, self <= config.max_port",
        "type Start: Integer where Init (self > 0)",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    assert_ne!(line(&mut session, "q: Seed = cli.seed").diagnostics, []);
    let port = line(&mut session, "port: Port = cli.port");
    assert_eq!(port.diagnostics, [], "{:?}", port.diagnostics);
    assert_eq!(port.checks.len(), 2);
    assert!(port.checks.iter().all(|c| c.phase == Phase::Init));
    let seed = line(&mut session, "q: Seed = 7");
    assert_eq!(seed.diagnostics, [], "{:?}", seed.diagnostics);
    assert_eq!(seed.checks.len(), 1);
    assert_eq!(seed.checks[0].phase, Phase::Compile);
    assert_eq!(seed.checks[0].reason, CheckReason::CompileTimeEvaluation);
    assert_ne!(
        line(&mut session, "f: Integer -> Start = n => n").diagnostics,
        []
    );
    assert_eq!(
        line(&mut session, "start: Start = cli.port").diagnostics,
        []
    );
    assert_eq!(line(&mut session, "port: Port = 65535").checks, []);
    assert_ne!(line(&mut session, "port: Port = 65536").diagnostics, []);
}

#[test]
fn holding_widths_are_type_facts_not_value_ranges() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("pixels", "pub rgb: [U8; 3]").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use pixels",
        "rgb = pixels.rgb",
        "type Flag: Bool & Bits<1>",
        "type Mode: Size<1> { read: Flag, write: Flag, exec: Flag }",
        "m = Mode -read -write=False -exec",
        "type Byte: Integer & Bits<8> where 0 <= self, self <= 255",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    for (text, expected) in
        [("rgb", "held Size<3>"), ("m", "held Size<1>, 3 of 8 bits")]
    {
        let report = line(&mut session, text);
        let fipe::ast::Entry::Line(id) =
            report.module.as_ref().unwrap().entries[0]
        else {
            panic!()
        };
        assert_eq!(
            report
                .typed
                .unwrap()
                .known(id)
                .unwrap()
                .holding
                .as_ref()
                .unwrap()
                .to_string(),
            expected
        );
    }
    for text in ["type Clash: U8 & U16", "type TooBig: Integer & Bits<8> where -129 <= self, self <= 127", "type Huge: Integer & Bits<64> where 0 <= self, self <= 18446744073709551616", "type Payload: Bits<8> { A(x: U8), B }"] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
    assert_eq!(line(&mut session, "type Fits: Integer & Bits<65> where 0 <= self, self <= 18446744073709551616").diagnostics, []);
}

#[test]
fn timing_is_only_a_binding_limit_and_cannot_be_negated() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "f: (Compile & Integer) -> Integer = n => n",
        "f: Integer -> (Init & Integer) = n => n",
        "type R { x: Compile & Integer }",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
    assert_eq!(
        line(&mut session, "type T: Integer where Init (self > 0)").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "type N: Integer & !T").diagnostics, []);
}

#[test]
fn purity_follows_called_and_passed_functions() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli",
        "effect = n => cli.port + n",
        "alias = effect",
        "apply = f n => f n",
        "pure = n => n + 1",
        "answer = Compile (apply pure 2)",
        "root = Compile (sqrt 4.0)",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    for text in [
        "answer = Compile (effect 2)",
        "answer = Compile (alias 2)",
        "answer = Compile (apply effect 2)",
        "answer = Compile (sin 0.0)",
        "answer = Compile (log 1)",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn imported_phases_and_purity_keep_their_module_identity() {
    use fipe::{
        embed::{Outcome, Reply},
        typed::Phase,
    };
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert!(matches!(session.line("use functions"), Outcome::Request(_)));
    let Outcome::Done(report) = session.reply(Reply::File(Some("use cli\nhelper = n => cli.port + n\npub effect = n => helper n\npub pure = n => n + 1\npub port = cli.port\npub exact = 3".into()))) else { panic!() };
    assert_eq!(report.diagnostics, [], "{:?}", report.diagnostics);
    assert_eq!(
        line(&mut session, "x = Compile (functions.pure 1)").diagnostics,
        []
    );
    assert_ne!(
        line(&mut session, "x = Compile (functions.effect 1)").diagnostics,
        []
    );
    for (text, phase) in [
        ("functions.port", Phase::Init),
        ("functions.exact", Phase::Compile),
    ] {
        let report = line(&mut session, text);
        let fipe::ast::Entry::Line(id) =
            report.module.as_ref().unwrap().entries[0]
        else {
            panic!()
        };
        assert_eq!(report.typed.unwrap().known(id).unwrap().phase, phase);
    }
}

#[test]
fn operator_purity_uses_the_selected_meaning() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli",
        "type V: Add { n: Integer }",
        "impl V: Add { add = other self => V cli.port }",
        "type P: Add { n: Integer }",
        "impl P: Add { add = other self => P (self.n + other.n) }",
    ] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    assert_ne!(
        line(&mut session, "bad = Compile ((V 1) + (V 2))").diagnostics,
        []
    );
    assert_eq!(
        line(&mut session, "good = Compile ((P 1) + (P 2))").diagnostics,
        []
    );
}

#[test]
fn demands_reach_grouped_entries_and_host_constants_bound_holdings() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module("config", "pub max_port = 65535")
        .unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in ["use config", "type Port16: Integer & Bits<16> where 0 <= self, self <= config.max_port", "is_prime = n => n > 1", "type Seed: Integer where Compile (is_prime self)", "f = n => n + 1", "q: Compile & Seed = (f 7)"] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
        if text.starts_with("q:") {
            assert_eq!(report.checks.len(), 1);
            assert_eq!(report.checks[0].phase, fipe::typed::Phase::Compile);
        }
    }
}

#[test]
fn sequence_literals_enter_limited_elements() {
    for text in [
        "xs: Vec<U8> = [1, 2, 300]",
        "xs: Deque<U8> = [300]",
        "xs: Vec<Vec<U8>> = [[300]]",
    ] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn bound_tuples_enter_each_limited_part() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(line(&mut session, "t = (1, 300)").diagnostics, []);
    assert_ne!(line(&mut session, "ends: U8 * U8 = t").diagnostics, []);
}

#[test]
fn parameterized_records_enter_each_limited_field() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Pair<A, B> { first: A, second: B }")
            .diagnostics,
        []
    );
    assert_ne!(
        line(&mut session, "p: Pair<U8, U8> = Pair 300 400").diagnostics,
        []
    );
}

#[test]
fn union_proof_requires_the_values_meaning() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    let report = line(&mut session, "id: U8 | String = 300");
    assert_ne!(report.diagnostics, []);
}

#[test]
fn function_arguments_with_limits_require_narrowing_proof() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["f: U8 -> U8 = n => n", "apply = g x => g x"] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    let report = line(&mut session, "y = apply f 300");
    assert_eq!(report.diagnostics, []);
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.reason == fipe::typed::CheckReason::Narrowing),
        "{:?}",
        report.checks
    );
}

#[test]
fn division_faults_fail_limit_tests() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Q: Integer where 10 / self < 5").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "z: Q = 0").diagnostics, []);
}

#[test]
fn float_limits_do_not_create_integer_ranges() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type A: F64 where self < 5",
        "type B: F64 where self <= 4",
        "a: A = 4.5",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    let report = line(&mut session, "b: B = a");
    assert!(report.diagnostics.len() + report.checks.len() > 0);
}

#[test]
fn written_conversions_drop_source_parts() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type A { n: Integer }",
        "type B: From<A> { n: Integer }",
        "impl B: From<A> { from = a => B (a.n + 1000) }",
        "b = to<B> (A 5)",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    let report = line(&mut session, "x: U8 = b.n");
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.reason == fipe::typed::CheckReason::Narrowing),
        "{report:?}"
    );
}

#[test]
fn written_sequences_drop_bracket_parts() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Buf<T>: Sequence<T> { len: Integer }",
        "impl Buf<T>: Sequence<T> { sequence = slots => Self 1000 }",
        "b: Buf<Integer> = [1, 2, 3]",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    let report = line(&mut session, "x: U8 = b.len");
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.reason == fipe::typed::CheckReason::Narrowing),
        "{report:?}"
    );
}

#[test]
fn self_construction_enters_declared_tests() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(
            &mut session,
            "type R { lo: Integer, hi: Integer } where self.lo <= self.hi"
        )
        .diagnostics,
        []
    );
    let report = line(&mut session, "impl R { bad = Self 5 0 }");
    assert_ne!(report.diagnostics, []);
}

#[test]
fn signed_type_cycles_are_check_errors() {
    for text in [
        "type S: !S",
        "type S: Integer | !S",
        "type A: !B\ntype B: !A",
    ] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn complementary_gates_cannot_have_extra_limits() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Small: Integer where self < 10",
        "type Other: U8 & !Small",
        "type Large: Integer & !Small where self > 100",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    for text in [
        "kind = n => n | { Small p => 1, Other q => 2 }",
        "kind = n => n | { Small p => 1, Large q => 2 }",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn gate_coverage_reads_nested_and_multiple_patterns() {
    for text in [
        "k = Small p y => 1",
        "k = (Small p, y) => 1",
        "k = m => m | { Just (Small p) => 1, Nothing => 0 }",
    ] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        assert_eq!(
            line(&mut session, "type Small: Integer where self < 10")
                .diagnostics,
            []
        );
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn postfix_members_keep_called_phase() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli",
        "type V { x: Integer }",
        "impl V { get = self => cli.port + self.x }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    assert_ne!(
        line(&mut session, "y: Compile & Integer = (V 1).get").diagnostics,
        []
    );
}

#[test]
fn destructured_names_keep_binding_phase() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(line(&mut session, "(a, b) = (log 1, 5)").diagnostics, []);
    assert_ne!(
        line(&mut session, "z: Compile & Integer = b").diagnostics,
        []
    );
}

#[test]
fn known_array_length_does_not_make_elements_exact() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "is_sorted = v => True",
        "type Sorted3: [Integer; 3] where Compile (is_sorted self)",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    assert_ne!(
        line(&mut session, "f: [Integer; 3] -> Sorted3 = xs => xs").diagnostics,
        []
    );
}

#[test]
fn empty_ranges_stay_empty_and_prove_entries() {
    use fipe::{
        ast::BinaryOp as B,
        limits::known::{End, Range},
    };
    let empty = Range {
        lo: End::Finite(10.into()),
        hi: End::Finite(5.into()),
    };
    let scaled = empty.arithmetic(B::Multiply, &Range::exact(1_000_000.into()));
    assert!(scaled.lo > scaled.hi);
    assert_eq!(empty.compare(B::Less, &Range::exact(0.into())), Some(true));
}

#[test]
fn strict_division_is_planned_and_allowed() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert_eq!(line(&mut session, "use cli").diagnostics, []);
    session.strict(true);
    let report = line(&mut session, "x = 10 / cli.port");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks.len(), 1);
    let report = line(&mut session, "type Q: Integer where 100 / self <= 10");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks, []);
}

#[test]
fn holding_fit_ignores_member_limit_order() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Flag2: Show & Bool & Bits<1>").diagnostics,
        []
    );
}

#[test]
fn nested_instantiations_must_fit_their_holdings() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Packed<T>: Bits<8> { v: T }").diagnostics,
        []
    );
    for text in [
        "f: Packed<U16> -> Integer = _ => 1",
        "xs: Vec<Packed<U16>> = []",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn prelude_calls_follow_claim_member_purity() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in [
        "use cli",
        "type V: Zero & Add { x: Integer }",
        "impl V: Add { add = o self => V (self.x + o.x + cli.port) }",
        "impl V: Zero { zero = V 0 }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    assert_ne!(
        line(&mut session, "y: Compile & V = sum [V 1, V 2]").diagnostics,
        []
    );
}

#[test]
fn impl_arms_share_the_same_result_meaning() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["n = 1", "type P { x: Integer }"] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    let report =
        line(&mut session, "impl P { f = { True => n, False => 2.5 } }");
    assert_ne!(report.diagnostics, []);
}

#[test]
fn untyped_try_reports_one_diagnostic() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "probe = input => input | try")
            .diagnostics
            .len(),
        1
    );
}

#[test]
fn element_errors_point_at_the_failed_element() {
    use fipe::session::Diagnostic;
    for text in ["xs: [U8; 3] = [1, 2, 300]", "xs: Vec<U8> = [1, 2, 300]"] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics.len(), 1, "{report:?}");
        assert!(report.diagnostics.iter().any(|d| matches!(d, Diagnostic::Type(e) if &text[e.span.start..e.span.end] == "300")), "{:?}", report.diagnostics);
    }
}

#[test]
fn limited_float_literals_keep_exact_values() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type PF: F64 where self > 0.5").diagnostics,
        []
    );
    let report = line(&mut session, "a: PF = 4.5");
    assert_eq!(report.diagnostics, []);
    assert_eq!(
        report.checks[0].reason,
        fipe::typed::CheckReason::CompileTimeEvaluation
    );
}

#[test]
fn fact_names_contain_only_passed_tests() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert_eq!(line(&mut session, "use cli").diagnostics, []);
    assert_eq!(
        line(&mut session, "x: Init & Integer = cli.port").diagnostics,
        []
    );
    let report = line(&mut session, "x");
    let fipe::ast::Entry::Line(id) = report.module.unwrap().entries[0] else {
        panic!()
    };
    assert_eq!(report.typed.unwrap().known(id).unwrap().names, []);
}

#[test]
fn fact_printer_distinguishes_unknown_empty_and_exact_parts() {
    use fipe::limits::known::{End, Known, Range};
    let mut unknown = Known::default();
    unknown.range = Some(Range::unknown());
    assert_eq!(unknown.to_string(), "");
    let mut empty = Known::default();
    empty.range = Some(Range {
        lo: End::Finite(1.into()),
        hi: End::Finite(0.into()),
    });
    assert_eq!(empty.to_string(), "range empty\n");
    let mut text = Known::default();
    let mut part = Known::default();
    part.exact = Some("\"fipe\"".into());
    text.parts.push(("name".into(), part));
    assert_eq!(text.to_string(), "parts name: \"fipe\"\n");
}

#[test]
fn written_constructor_arms_return_the_enclosing_type() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["n = 1", "type P { x: Integer }"] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    assert_ne!(
        line(&mut session, "impl P { f = True => n, False => 2.5 }")
            .diagnostics,
        []
    );
}

#[test]
fn gate_bindings_test_before_narrowing() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Small: Integer where self <= 9").diagnostics,
        []
    );
    assert_ne!(line(&mut session, "Small z = 50").diagnostics, []);
}

#[test]
fn written_constructors_do_not_keep_argument_parts() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type C { n: Integer }",
        "impl C { n => Self { n: n + 300 } }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    let report = line(&mut session, "k: U8 = (C 5).n");
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.reason == fipe::typed::CheckReason::Narrowing),
        "{report:?}"
    );
}

#[test]
fn piped_constructor_spots_replace_default_facts() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Point { x: I64, y: I64 = 0 }").diagnostics,
        []
    );
    assert_eq!(
        line(&mut session, "z = 1 / (7 | Point 5).y").diagnostics,
        []
    );
}

#[test]
fn record_constructor_patterns_bind_field_types() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type P2 { x: Integer, y: String }",
        "p = r => r | { P2 a => a }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    let report = line(&mut session, "p");
    let fipe::ast::Entry::Line(id) = report.module.unwrap().entries[0] else {
        panic!()
    };
    assert_eq!(
        report.typed.unwrap().expr(id).unwrap().to_string(),
        "P2 -> Integer"
    );
}

#[test]
fn impure_limit_tests_are_check_errors() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert_eq!(line(&mut session, "use cli").diagnostics, []);
    for text in [
        "type T: Integer where (log self) > 0",
        "type T: Integer where self > cli.port",
    ] {
        assert_ne!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn generic_integer_division_has_a_zero_check() {
    let mut declarations = fipe::decl::Declarations::new();
    declarations.module("cli", "pub port: Integer").unwrap();
    let mut session = fipe::session::Session::new(declarations);
    for text in ["use cli", "dv = a b => a / b"] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    let report = line(&mut session, "q: Integer = dv 10 cli.port");
    assert_eq!(report.diagnostics, []);
    assert_ne!(session.checks(None), []);
}

#[test]
fn claimed_members_accept_their_result_meaning() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type L1: Len { n: Natural }").diagnostics,
        []
    );
    assert_eq!(
        line(&mut session, "impl L1: Len { len = self => self.n }").diagnostics,
        []
    );
}

#[test]
fn unit_and_timed_types_keep_holdings() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in ["t: U8 * () = (1, ())", "f: Compile & F64 = 1.5"] {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, []);
        let fipe::ast::Entry::Line(id) = report.module.unwrap().entries[0]
        else {
            panic!()
        };
        assert!(
            report.typed.unwrap().known(id).unwrap().holding.is_some(),
            "{text}"
        );
    }
}

#[test]
fn field_reads_keep_place_limits() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(line(&mut session, "type R { n: Natural }").diagnostics, []);
    let report = line(&mut session, "g: R -> Natural = r => r.n");
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks, []);
}

#[test]
fn conflicting_limit_clauses_report_one_refutation() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(
            &mut session,
            "type Empty: Integer where self < 5, self > 10"
        )
        .diagnostics,
        []
    );
    assert_eq!(line(&mut session, "x: Empty = 7").diagnostics.len(), 1);
}

#[test]
fn type_printer_preserves_grouping() {
    use fipe::{resolve::Origin, types::Type};
    let integer = Type::named(Origin::Prelude, "Integer", vec![]);
    let function = Type::function(integer.clone(), integer.clone());
    let both = Type::Both(vec![
        Type::named(Origin::Prelude, "Compile", vec![]),
        function,
    ]);
    assert_eq!(both.to_string(), "Compile & (Integer -> Integer)");
    let negated = Type::Not(Box::new(Type::Either(vec![
        integer,
        Type::named(Origin::Prelude, "String", vec![]),
    ])));
    assert_eq!(negated.to_string(), "!(Integer | String)");
}

#[test]
fn brackets_receive_places_through_operators_pipes_and_payloads() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "v: Vec<Integer> = [1, 2, 3]",
        "fv: Vec<Integer> -> Integer = xs => 1",
        "v ++ [4]",
        "v == [1, 2, 3]",
        "[4] | fv",
        "m: Maybe<Vec<Integer>> = Just [1]",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
}

#[test]
fn arbitrary_sequence_places_enter_elements_before_calling_sequence() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Buf<T>: Sequence<T> { len: Integer }",
        "impl Buf<T>: Sequence<T> { sequence = slots => Self 1000 }",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, []);
    }
    assert_ne!(line(&mut session, "b: Buf<U8> = [300]").diagnostics, []);
}

#[test]
fn array_length_and_holding_conflicts_report_one_cause() {
    for text in ["z: [Integer; 2] = [1, 2, 3]", "hb: Bits<8> & Bits<16> = 1"] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        assert_eq!(line(&mut session, text).diagnostics.len(), 1, "{text}");
    }
}

#[test]
fn holding_only_types_have_no_implicit_gate() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Flag: Bool & Bits<1>").diagnostics,
        []
    );
    assert!(
        matches!(&line(&mut session, "Flag True").diagnostics[..], [fipe::session::Diagnostic::Resolve(fipe::resolve::Error { reason: fipe::resolve::Reason::NoGate { name }, .. })] if name == "Flag")
    );
}

#[test]
fn unknown_sum_payload_limits_remain_open() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(
        line(&mut session, "type Choice<A> { Pick(value: A), Empty }")
            .diagnostics,
        []
    );
    let report =
        line(&mut session, "f: Choice<Integer> -> Choice<U8> = x => x");
    assert_eq!(report.diagnostics, []);
    assert_ne!(report.checks, []);
}

#[test]
fn conversion_and_strict_entries_report_one_cause() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert_eq!(line(&mut session, "xs = [300]").diagnostics, []);
    let report = line(&mut session, "vy: Vec<U8> = xs | to<Vec<U8>>");
    assert_eq!(report.diagnostics, []);
    assert_ne!(report.checks, []);
    assert!(report.checks.iter().all(|check| check.gate));
    session.strict(true);
    assert_eq!(
        line(&mut session, "f: Integer -> I8 = x => x")
            .diagnostics
            .len(),
        1
    );
}

#[test]
fn called_functions_keep_written_form_purity() {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in [
        "type Buf<T>: Sequence<T> { len: Integer }",
        "impl Buf<T>: Sequence<T> { sequence = slots => Self (log 1000) }",
        "make: Integer -> Buf<Integer> = x => [x]",
    ] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    assert_ne!(
        line(&mut session, "b: Compile & Buf<Integer> = make 1").diagnostics,
        []
    );
}

#[test]
fn written_constructors_follow_body_guard_and_default_purity() {
    for definition in [
        "impl C { n => Self { n: log n } }",
        "impl C { n if (log n) > 0 => Self n, _ => Self 0 }",
        "impl C { (n = (log 1)) => Self n }",
    ] {
        let mut session =
            fipe::session::Session::new(fipe::decl::Declarations::new());
        assert_eq!(line(&mut session, "type C { n: Integer }").diagnostics, []);
        assert_eq!(
            line(&mut session, definition).diagnostics,
            [],
            "{definition}"
        );
        assert_ne!(
            line(&mut session, "x: Compile & C = C 1").diagnostics,
            [],
            "{definition}"
        );
    }
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for source in [
        "type C { n: Integer }",
        "impl C { n => Self n }",
        "x: Compile & C = C 1",
    ] {
        assert_eq!(line(&mut session, source).diagnostics, [], "{source}");
    }
}
