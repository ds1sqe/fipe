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
    assert_eq!(line(&mut session, "xs = [1, 300]").diagnostics, []);
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
