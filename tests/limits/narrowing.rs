//! Conditions, guards, literal patterns and joins; calls and type
//! variables (07-checks §What is known 4 to 7).
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
    assert!(
        format!("{:?}", report.diagnostics).contains("refuted entry"),
        "{text}: {:?}",
        report.diagnostics
    );
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
    for text in [
        "x: Maybe<U8> = r |: (v => v)",
        "x: Maybe<U8> = r |! Nothing",
        "x: Maybe<U8> = r |! fault \"none\"",
    ] {
        open(&lines, text, CheckReason::UnknownInput);
    }
}

/// A line whose only open entries are of a function value, which wait
/// for rule 8: every other entry is proved.
fn functions_open(lines: &[&str], text: &str) {
    let report = line(&mut session(lines), text);
    assert_eq!(report.diagnostics, [], "{text}: {:?}", report.diagnostics);
    assert!(
        report.checks.iter().all(|c| c.name.contains("->")),
        "{text}: {:?}",
        report.checks
    );
}

const HALF: &str = "half: U8 -> Integer = n => n / 2";
const MAX: &str = "max = x y if x > y => x, _ y => y";
const ODD: [&str; 3] = [
    "type Odd: Integer where self % 2 == 1",
    "from: Integer -> Odd = n => n | { Odd q => q, _ => from (n + 1) }",
    "next: Odd -> Odd = p => from (p + 1)",
];

#[test]
fn a_call_is_known_by_its_body_at_the_declared_parameters() {
    // `half` on one line, its calls on the next: 0..127 from the body.
    proved(&[HALF], "h: U8 = half 200");
    proved(&[HALF], "h: I8 = half 200");
    proved(&[HALF], "h: U8 = 200 | half");
    open(
        &[HALF],
        "h: U8 = (half 200) + 200",
        CheckReason::UnknownInput,
    );
    // A body exact whatever goes in gives that value.
    proved(&["k: Integer -> Integer = x => 7"], "j: U8 = k 300");
    refuted(&["k: Integer -> Integer = x => 300"], "z: U8 = k 1");
    // An undeclared result of an Integer is open for an unknown input.
    open(
        &["g = n => n * 2"],
        "z: U8 = g 3",
        CheckReason::UnknownInput,
    );
}

#[test]
fn a_call_in_one_unit_reads_the_body_above_it() {
    let report = line(
        &mut session(&[]),
        "{\n    half: U8 -> Integer = n => n / 2\n    h: U8 = half 200\n    h\n}",
    );
    assert_eq!(report.diagnostics, []);
    assert_eq!(report.checks, []);
}

#[test]
fn a_recursive_callee_is_known_by_its_declared_result() {
    proved(&ODD, "o: Odd = next 5");
    // A later line keeps the identity of the names a body passed.
    let report = line(&mut session(&ODD), "o = next 5");
    let (typed, module) = (report.typed.unwrap(), report.module.unwrap());
    let fipe::ast::Entry::Line(id) = module.entries.last().unwrap() else {
        panic!()
    };
    let fipe::ast::ExprKind::Bind { value, .. } =
        module.expr(*id).unwrap().kind
    else {
        panic!()
    };
    assert_eq!(typed.known(value).unwrap().to_string(), "names Odd\n");
    // `from` calls itself: its body's join is Odd by the declared result.
    assert_eq!(line(&mut session(&ODD[..1]), ODD[1]).checks, []);
    let count =
        "count: Integer -> U8 = n => if n > 0 { count (n - 1) } else { 0 }";
    proved(&[count], "c: U8 = count 5");
    // A cycle of undeclared results is unknown.
    open(
        &["spin = n => if n > 0 { spin (n - 1) } else { 0 }"],
        "s: U8 = spin 5",
        CheckReason::UnknownInput,
    );
}

#[test]
fn a_result_variable_is_the_join_of_what_went_in() {
    let lines = [MAX, "a: U8 = 200", "b: U8 = 100"];
    proved(&lines, "m: U8 = max a b");
    proved(&lines, "m: U8 = b | max a");
    refuted(&lines, "m: U8 = max 300 400");
    open(&lines, "m: U8 = max a 300", CheckReason::UnknownInput);
    // `f = x => x` gives back what went in.
    refuted(&["f = x => x"], "y: U8 = f 256");
    proved(&["f = x => x"], "y: U8 = f 255");
    refuted(&["f: a -> a = x => x"], "y: U8 = f 256");
    // With nothing going in at the variable, it is unknown, not dead.
    open(
        &["spin = x => spin x"],
        "s: U8 = spin 1",
        CheckReason::UnknownInput,
    );
    // A limit whose member returns the variable may produce it unseen.
    open(
        &["inc = x => x + 1", "a: U8 = 255"],
        "z: U8 = inc a",
        CheckReason::UnknownInput,
    );
    open(
        &["inc: Number a => a -> a = x => x + 1", "a: U8 = 255"],
        "z: U8 = inc a",
        CheckReason::UnknownInput,
    );
}

#[test]
fn a_function_argument_returning_the_variable_gives_its_declared_result() {
    let lines = ["twice = f x => f (f x)", "cut: Integer -> U8 = n => 0"];
    functions_open(&lines, "t: U8 = twice cut 8");
    // An argument with no declared result limit may return anything.
    open(
        &lines,
        "t: U8 = twice (n => n) 8",
        CheckReason::UnknownInput,
    );
}

#[test]
fn a_partial_call_keeps_the_remaining_parameters_and_the_result() {
    let lines = [
        "add: U8 -> U8 -> U8 = x y => x",
        "seven: Integer -> Integer -> Integer = x y => 7",
    ];
    proved(&lines, "q: U8 = (add 3) 4");
    let mut session = session(&lines);
    for text in ["p = add 3", "s = seven 1"] {
        assert_eq!(line(&mut session, text).diagnostics, [], "{text}");
    }
    assert_eq!(line(&mut session, "q: U8 = p 4").checks, []);
    assert_eq!(line(&mut session, "r: U8 = s 2").checks, []);
}

#[test]
fn prelude_and_host_functions_are_known_by_their_declared_limits() {
    // `log` gives back its argument, but no prelude body is read.
    open(&[], "l: U8 = log 300", CheckReason::UnknownInput);
    let mut declarations = fipe::decl::Declarations::new();
    declarations
        .module(
            "calc",
            "pub small: Integer -> U8\npub same: Integer -> Integer = n => 3",
        )
        .unwrap();
    let mut session = fipe::session::Session::new(declarations);
    assert_eq!(line(&mut session, "use calc").diagnostics, []);
    assert_eq!(line(&mut session, "s: U8 = calc.small 300").checks, []);
    let report = line(&mut session, "t: U8 = calc.same 300");
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
fn a_file_module_s_function_is_known_by_its_body() {
    use fipe::embed::{Outcome, Reply};
    let mut session =
        fipe::session::Session::new(fipe::decl::Declarations::new());
    assert!(matches!(session.line("use numbers"), Outcome::Request(_)));
    let Outcome::Done(report) = session.reply(Reply::File(Some(
        "pub half: U8 -> Integer = n => n / 2".into(),
    ))) else {
        panic!()
    };
    assert_eq!(report.diagnostics, []);
    assert_eq!(line(&mut session, "h: U8 = numbers.half 200").checks, []);
}

#[test]
fn nested_and_member_arm_lists_are_read_at_their_declared_parameters() {
    proved(
        &[],
        "g: Integer -> U8 = x => {\n    k: U8 -> Integer = y => y / 2\n    k 100\n}",
    );
    let lines = ["type W { n: Integer }"];
    functions_open(&lines, "impl W { halve: U8 -> U8 = n => n / 2 }");
}

#[test]
fn a_fault_call_is_dead_and_proves_its_place() {
    proved(&[], "w: U8 = fault \"never\"");
    proved(
        &["boom: Integer -> Integer = n => fault \"no\""],
        "b: U8 = boom 1",
    );
}
