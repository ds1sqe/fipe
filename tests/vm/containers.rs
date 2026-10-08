use super::run::shown;

#[test]
fn sequences_fold_sort_count_and_flatten() {
    assert_eq!(shown("[3, 1, 2] | fold 0 (a b => a + b)"), "6");
    assert_eq!(shown("[3, 1, 2] | sort"), "[1, 2, 3]");
    assert_eq!(shown("[3, 1, 2] | sort --desc"), "[3, 2, 1]");
    assert_eq!(shown("x: Vec<Integer> = []\nx | sort"), "[]");
    assert_eq!(
        shown("[\"bb\", \"a\", \"ccc\"] | sort --by (s => len s)"),
        "[\"a\", \"bb\", \"ccc\"]"
    );
    assert_eq!(
        shown(
            "[(2, \"x\"), (1, \"y\"), (2, \"a\")] | sort --by { (k, _) => k }"
        ),
        "[(1, \"y\"), (2, \"x\"), (2, \"a\")]"
    );
    assert_eq!(
        shown("[1, 2, 1, 3, 1] | count"),
        "[Count 1 3, Count 2 1, Count 3 1]"
    );
    assert_eq!(
        shown("x: Vec<Vec<Integer>> = [[1, 2], [], [3]]\nx | flatten"),
        "[1, 2, 3]"
    );
    assert_eq!(shown("[Ok 1, Ok 2] | collect"), "Ok [1, 2]");
    assert_eq!(
        shown("[Ok 1, Err \"no\", Err \"late\"] | collect"),
        "Err \"no\""
    );
}

#[test]
fn a_written_order_and_equality_reach_sort_and_count() {
    let source = "type Rank: Eq & Ord { n: Integer }\n\
        impl Rank: Eq { eq = other self => self.n % 10 == other.n % 10 }\n\
        impl Rank: Ord { cmp = other self => (self.n % 10).cmp (other.n % 10) }\n";
    assert_eq!(
        shown(&format!("{source}[Rank 13, Rank 21, Rank 32] | sort")),
        "[Rank 21, Rank 32, Rank 13]"
    );
    assert_eq!(
        shown(&format!("{source}[Rank 1, Rank 11, Rank 2] | count | len")),
        "2"
    );
}

#[test]
fn list_and_tuple_patterns_run() {
    let pair = "f: Vec<Integer> -> Integer = { [a, b] => a + b, _ => 0 }\n";
    assert_eq!(shown(&format!("{pair}f [1, 2]")), "3");
    assert_eq!(shown(&format!("{pair}f [1]")), "0");
    assert_eq!(shown("g = { (a, b) => a }\ng (1, \"x\")"), "1");
}

#[test]
fn the_types_example_runs_until_its_inexact_length() {
    // `len: U64 = (hypot x y)` is exact for `Point 3 4` but not for
    // `Point -y 1 -x 2`: the number row is exact or Faults (E2).
    let source = include_str!("../../Documentation/examples/03-types.fipe");
    let fault = super::run::run(source).expect_err("an inexact length");
    assert_eq!(fault.reason, fipe::value::Reason::NotExact);
    let at = &source[fault.span.start..fault.span.end];
    assert!(at.contains("hypot x y"), "{at}");
}
