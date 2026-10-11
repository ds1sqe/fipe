use super::run::{run, shown};
use super::{ir, Value};
use fipe::value::Reason;

#[test]
fn integers_add_exactly() {
    assert_eq!(shown("1 + 2"), "3");
    assert_eq!(shown("9223372036854775807 + 1"), "9223372036854775808");
    assert_eq!(shown("9223372036854775808 - 1"), "9223372036854775807");
    assert!(matches!(run("9223372036854775808 - 1"), Ok(Value::Int(_))));
    assert_eq!(shown("2 * 3 - 4"), "2");
}

#[test]
fn division_rounds_toward_zero_and_the_remainder_keeps_the_dividend_sign() {
    assert_eq!(shown("-7 / 2"), "-3");
    assert_eq!(shown("-7 % 2"), "-1");
    assert_eq!(shown("7 % -2"), "1");
}

#[test]
fn a_zero_divisor_faults_at_the_divisor_by_the_division_test() {
    let source = "d = x => 1 / x\nd 0";
    let fault = run(source).unwrap_err();
    // The divisor's open entry runs before the division can.
    assert!(
        matches!(&fault.reason, Reason::Failed { value, .. } if value == "0")
    );
    assert_eq!(&source[fault.span.start..fault.span.end], "x");
}

#[test]
fn floats_fault_instead_of_an_infinity() {
    assert_eq!(shown("1.0 / 3.0"), "0.3333333333333333");
    assert_eq!(run("1.0 / 0.0").unwrap_err().reason, Reason::NotFinite);
    assert_eq!(shown("0.1 + 0.2"), "0.30000000000000004");
    assert_eq!(shown("round 2.5"), "2.0");
    assert_eq!(shown("x: F32 = 1.5\nx * 2.0"), "3.0");
}

#[test]
fn comparisons_switch_the_ordering() {
    assert_eq!(shown("1 < 2"), "True");
    assert_eq!(shown("2 < 1"), "False");
    assert_eq!(shown("1 == 1"), "True");
    assert_eq!(shown("1 != 1"), "False");
    assert_eq!(shown("\"a\" < \"b\""), "True");
    assert_eq!(shown("Less < Greater"), "True");
}

#[test]
fn literals_read_by_a_program_reader_run_its_body() {
    let source = "type Mask: Literal { bits: U8 }\n\
                  impl Mask: Literal { literal = f => Self 0 }\n\
                  m: Mask = 7\nm";
    assert_eq!(shown(source), "Mask 0");
}

#[test]
fn the_span_of_a_fault_is_in_the_unit() {
    let source = "f = x => x / 0\nf 1";
    let fault = run(source).unwrap_err();
    // The divisor's division test Faults first, at the divisor.
    assert_eq!(&source[fault.span.start..fault.span.end], "0");
    let _ = ir::Phase::Eval;
}
