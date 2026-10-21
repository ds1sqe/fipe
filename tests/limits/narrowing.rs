//! Conditions, guards, literal patterns and joins (07-checks §What is
//! known 4 and 7).
use super::line;
use fipe::typed::CheckReason;

fn session(lines: &[&str]) -> fipe::session::Session {
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    for text in lines {
        let report = line(&mut session, text);
        assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    }
    session
}

/// A line that checks with no open entry.
fn proved(lines: &[&str], text: &str) {
    let report = line(&mut session(lines), text);
    assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    assert_eq!(report.checks, [], "{text}");
}

/// A line that checks with an open entry for the reason given.
fn open(lines: &[&str], text: &str, reason: CheckReason) {
    let report = line(&mut session(lines), text);
    assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    assert!(
        !report.checks.is_empty()
            && report.checks.iter().all(|c| c.reason == reason),
        "{text}: {:?}",
        report.checks
    );
}

fn refuted(lines: &[&str], text: &str) {
    let report = line(&mut session(lines), text);
    assert_ne!(report.diagnostics, [], "{text}");
}

#[test]
fn branches_join_by_the_range_that_holds_both() {
    proved(&["flag = True"], "n: U8 = if flag { 2 } else { 200 }");
    let mut session = session(&["c = True"]);
    let report = line(&mut session, "y = if c { 2 } else { 200 }");
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
    let shown = typed.known(value).unwrap().to_string();
    assert_eq!(shown, "range 2..200\n");
    assert_eq!(line(&mut session, "a: U8 = y").checks, []);
    let report = line(&mut session, "b: I8 = y");
    assert_eq!(report.diagnostics, []);
    assert!(
        !report.checks.is_empty()
            && report
                .checks
                .iter()
                .all(|c| c.reason == CheckReason::UnknownInput),
        "{:?}",
        report.checks
    );
}

#[test]
fn conditions_narrow_their_bodies_and_negations_the_else_branch() {
    proved(
        &[],
        "clamp: Integer -> U8 = n => \
         if n < 0 { 0 } else { if n > 255 { 255 } else { n } }",
    );
    proved(
        &[],
        "f: Integer -> Natural = n => if n < 0 { 0 } else { n }",
    );
    proved(
        &[],
        "f: Integer -> U8 = n => if (0 <= n && n <= 255) { n } else { 0 }",
    );
    proved(
        &[],
        "f: Integer -> U8 = n => if (n < 0 || n > 255) { 0 } else { n }",
    );
    proved(
        &[],
        "f: Integer -> U8 = n => if !(n < 0 || n > 255) { n } else { 0 }",
    );
    // The then branch takes no negation: `n` is any Integer there.
    open(
        &[],
        "f: Integer -> U8 = n => if (n < 0 || n > 255) { n } else { 0 }",
        CheckReason::UnknownInput,
    );
    // The negation of `&&` is two ranges: the else branch takes none.
    open(
        &[],
        "f: Integer -> U8 = n => if (0 <= n && n <= 255) { 0 } else { n }",
        CheckReason::UnknownInput,
    );
}

#[test]
fn and_narrows_its_right_side_by_its_left() {
    proved(&[], "q: Integer -> Bool = n => n > 0 && 10 / n > 1");
    open(
        &[],
        "q: Integer -> Bool = n => n > -1 && 10 / n > 1",
        CheckReason::UnknownInput,
    );
}

#[test]
fn not_equal_narrows_only_at_an_end() {
    proved(&[], "f: U8 -> U8 = n => if n != 0 { n - 1 } else { 0 }");
    proved(&[], "f: U8 -> U8 = n => if n == 0 { 0 } else { n - 1 }");
    open(
        &[],
        "f: U8 -> U8 = n => if n != 5 { n - 1 } else { 0 }",
        CheckReason::UnknownInput,
    );
}

#[test]
fn a_length_narrows_by_its_condition() {
    proved(
        &[],
        "f: String -> Integer = s => if s.len > 0 { 10 / s.len } else { 0 }",
    );
}

#[test]
fn fault_and_empty_branches_are_skipped_by_the_join() {
    proved(
        &[],
        "g: Integer -> Natural = n => if n > 0 { n } else { fault \"x\" }",
    );
    refuted(
        &[],
        "g: Integer -> Natural = n => if n < 0 { n } else { fault \"x\" }",
    );
    proved(&[], "g: U8 -> U8 = n => if n > 300 { n + 1000 } else { n }");
}

#[test]
fn guards_narrow_their_body_and_later_arms_by_position() {
    proved(
        &[],
        "f: Integer -> U8 = n if (0 <= n && n <= 255) => n, _ => 0",
    );
    proved(&[], "f: Integer -> Natural = n if n < 0 => 0, m => m");
    proved(
        &[],
        "f: Integer -> Integer -> Natural = a b if b < 0 => 0, _ c => c",
    );
    // A later arm past a pattern that may fail takes no negation.
    open(
        &[],
        "f: Integer -> Integer -> Natural = 0 b if b < 0 => 0, _ c => c",
        CheckReason::UnknownInput,
    );
}

/// The open entries of a line for the test named `name`.
fn named(lines: &[&str], text: &str, name: &str) -> usize {
    let report = line(&mut session(lines), text);
    assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    report.checks.iter().filter(|c| c.name == name).count()
}

// The arm list piped into is a function value entering its place, open
// until functions are proved (rule 8); only the entries inside count.
#[test]
fn literal_patterns_narrow_a_piped_name() {
    let take = ["take: U8 -> Integer = k => k"];
    let divisor = "nonzero divisor";
    assert_eq!(
        named(
            &take,
            "g: Integer -> Integer = n => n | { 3 => take n, _ => 0 }",
            "U8"
        ),
        0
    );
    assert_eq!(
        named(
            &take,
            "g: Integer -> Integer = n => n | { _ => take n }",
            "U8"
        ),
        1
    );
    refuted(
        &take,
        "g: Integer -> Integer = n => n | { 300 => take n, _ => 0 }",
    );
    assert_eq!(
        named(
            &[],
            "g: Natural -> Integer = n => n | { 0 => 1, _ => 10 / n }",
            divisor
        ),
        0
    );
    assert_eq!(
        named(
            &[],
            "g: Natural -> Integer = n => \
             n | { 0 => 1, 1 => 2, _ => 10 / (n - 1) }",
            divisor
        ),
        0
    );
    // A guarded literal arm may fail by its guard: no `!=` after it.
    assert_eq!(
        named(
            &[],
            "g: Natural -> Bool -> Integer = n b => \
             n | { 0 if b => 1, _ => 10 / n }",
            divisor
        ),
        1
    );
}

// Only a range and names leave a branch, so a payload is unknown after
// the join; a stage's call result waits on rule 5.
#[test]
fn a_marked_pipe_joins_its_branches() {
    let lines = ["r: Maybe<Integer> = Just 5"];
    open(
        &lines,
        "x: Maybe<U8> = r |: (v => v)",
        CheckReason::Narrowing,
    );
    for text in [
        "x: Maybe<U8> = r |! Nothing",
        "x: Maybe<U8> = r |! fault \"none\"",
    ] {
        open(&lines, text, CheckReason::UnknownInput);
    }
}
