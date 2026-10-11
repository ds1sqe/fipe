//! The budget: one unit per node evaluated plus the size terms; the
//! first node that cannot be paid Faults with its span (H1).
use super::run::{budgeted, program, spanned};
use fipe::value::Reason;

/// Every budget below the full cost Faults `Budget` and spends it; the
/// span of each, in budget order; the full cost returns the value.
fn sweep(source: &str) -> Vec<String> {
    let program = program(source);
    let (result, full) = budgeted(&program, 1 << 20);
    assert!(result.is_ok(), "{source}: {result:?}");
    let spans = (0..full)
        .map(|budget| {
            let (result, charged) = budgeted(&program, budget);
            let fault = result.expect_err("a smaller budget Faults");
            assert_eq!(fault.reason, Reason::Budget);
            assert_eq!(charged, budget, "a Fault spends the budget");
            spanned(source, &fault).to_owned()
        })
        .collect();
    let (result, charged) = budgeted(&program, full);
    assert!(result.is_ok());
    assert_eq!(charged, full);
    spans
}

#[test]
fn the_exact_budget_boundary_faults_before_the_first_unaffordable_action() {
    assert_eq!(sweep("1 + 2"), ["+", "1", "2", "1 + 2", "1 + 2"]);
    assert_eq!(
        sweep("[1, 2, 3] | sum"),
        [
            "[1, 2, 3] | sum",
            "sum",
            "1",
            "2",
            "3",
            "[1, 2, 3]",
            "sum",
            // One per element the sum visits, at the sum.
            "sum",
            "sum",
            "sum",
        ]
    );
    assert_eq!(
        sweep("f = n => n * 2\nf 3"),
        [
            "f = n => n * 2",
            "f",
            "3",
            "f 3",
            "*",
            "n",
            "2",
            "2",
            "n * 2",
            "n * 2",
        ]
    );
}

#[test]
fn an_integer_add_charges_one_per_limb_of_the_larger_operand() {
    let cost = |source| budgeted(&program(source), 1 << 20).1;
    // 2^64 takes two limbs, 1 one; the node counts are the same.
    let small = cost("a = 1\na + 1");
    assert_eq!(cost("a = 18446744073709551616\na + 1"), small + 1);
    assert_eq!(cost("a = 1\n1 + a"), small);
    // A product charges the product of the limb counts.
    let small = cost("a = 1\na * a");
    assert_eq!(cost("a = 18446744073709551616\na * a"), small + 3);
}

#[test]
fn a_container_operation_charges_one_per_element_visited() {
    let cost = |source| budgeted(&program(source), 1 << 20).1;
    assert_eq!(
        cost("[1, 2, 3, 4] | map (n => n)") - cost("[1, 2, 3] | map (n => n)"),
        // One more element, one more node, one more visit, one more call
        // of the body (its node and its read of `n`).
        1 + 1 + 1
    );
    // `len` visits no element: only the element's own node.
    assert_eq!(cost("[1, 2, 3, 4] | len"), cost("[1, 2, 3] | len") + 1);
}

#[test]
fn a_recursion_without_end_faults_at_exactly_the_budget() {
    let source = "r = n => r (n + 1)\nr 0";
    let program = program(source);
    let (result, charged) = budgeted(&program, 1000);
    let fault = result.unwrap_err();
    assert_eq!(fault.reason, Reason::Budget);
    assert_eq!(charged, 1000);
    let (again, _) = budgeted(&program, 1000);
    assert_eq!(again.unwrap_err().span, fault.span);
}

#[test]
fn a_sum_under_a_small_budget_faults_at_the_sum() {
    let source = "[1, 2, 3] | sum";
    let (result, _) = budgeted(&program(source), 8);
    let fault = result.unwrap_err();
    assert_eq!(fault.reason, Reason::Budget);
    assert_eq!(spanned(source, &fault), "sum");
}

#[test]
fn a_skipped_side_charges_nothing() {
    let cost = |source| budgeted(&program(source), 1 << 20).1;
    assert_eq!(cost("Just 3 || (1 + 2 + 3 + 4)"), cost("Just 3 || 4"),);
}
