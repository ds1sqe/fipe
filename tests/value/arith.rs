use super::{form, lcg, unit};
use core::cmp::Ordering;
use fipe::{
    token::Magnitude,
    value::{arith, Prelude, Reason, Value},
};

fn big(text: &str) -> Value {
    let digits = text.strip_prefix('-').unwrap_or(text);
    Value::integer(text.starts_with('-'), Magnitude::parse(digits, 10).unwrap())
}

#[test]
fn small_sums_stay_small_and_overflow_spills_to_big() {
    assert_eq!(arith::add(&Value::Int(1), &Value::Int(2)), Value::Int(3));
    let spilled = arith::add(&Value::Int(i64::MAX), &Value::Int(1));
    assert!(matches!(spilled, Value::Big(_)));
    assert_eq!(arith::show(&spilled), "9223372036854775808");
    assert_eq!(
        arith::sub(&spilled, &Value::Int(1)),
        Value::Int(i64::MAX),
        "a difference that fits is Int again"
    );
    assert_eq!(
        arith::show(&arith::mul(&Value::Int(i64::MAX), &Value::Int(2))),
        "18446744073709551614"
    );
    assert_eq!(
        arith::show(&arith::neg(&Value::Int(i64::MIN))),
        "9223372036854775808"
    );
    assert_eq!(
        arith::neg(&big("9223372036854775808")),
        Value::Int(i64::MIN)
    );
    assert_eq!(
        arith::add(&big("-18446744073709551616"), &big("18446744073709551616")),
        Value::Int(0)
    );
    assert_eq!(
        arith::sub(&Value::Int(0), &big("18446744073709551616")),
        big("-18446744073709551616")
    );
}

#[test]
fn division_truncates_toward_zero_and_the_remainder_keeps_the_dividend_sign() {
    let div = |a: i64, b: i64| arith::div(&Value::Int(a), &Value::Int(b));
    let rem = |a: i64, b: i64| arith::rem(&Value::Int(a), &Value::Int(b));
    assert_eq!(div(-7, 2), Ok(Value::Int(-3)));
    assert_eq!(rem(-7, 2), Ok(Value::Int(-1)));
    assert_eq!(div(7, -2), Ok(Value::Int(-3)));
    assert_eq!(rem(7, -2), Ok(Value::Int(1)));
    assert_eq!(div(1, 0), Err(Reason::DivideByZero));
    assert_eq!(rem(1, 0), Err(Reason::DivideByZero));
    assert_eq!(div(i64::MIN, -1), Ok(big("9223372036854775808")));
    assert_eq!(rem(i64::MIN, -1), Ok(Value::Int(0)));
    let two64 = big("18446744073709551616");
    assert_eq!(
        arith::div(&two64, &Value::Int(3)),
        Ok(Value::Int(6_148_914_691_236_517_205))
    );
    assert_eq!(arith::rem(&two64, &Value::Int(3)), Ok(Value::Int(1)));
    assert_eq!(
        arith::div(&arith::neg(&two64), &Value::Int(3)),
        Ok(Value::Int(-6_148_914_691_236_517_205))
    );
    assert_eq!(
        arith::rem(&arith::neg(&two64), &Value::Int(3)),
        Ok(Value::Int(-1))
    );
    assert_eq!(
        arith::div(&two64, &Value::Int(0)),
        Err(Reason::DivideByZero)
    );
}

#[test]
fn order_and_equality_cross_the_forms() {
    let two64 = big("18446744073709551616");
    assert_eq!(arith::cmp(&Value::Int(1), &two64), Ordering::Less);
    assert_eq!(
        arith::cmp(&arith::neg(&two64), &Value::Int(i64::MIN)),
        Ordering::Less
    );
    assert_eq!(arith::cmp(&two64, &two64), Ordering::Equal);
    assert_eq!(arith::cmp(&Value::Int(-1), &Value::Int(1)), Ordering::Less);
    assert!(arith::eq(&two64, &big("18446744073709551616")));
    assert!(!arith::eq(&two64, &Value::Int(1)));
    assert_eq!(arith::zero(), Value::Int(0));
}

#[test]
fn show_and_read_round_trip_integers_to_128_bits() {
    let unit = unit();
    let prelude = Prelude::new(&unit.tables());
    let mut state = 7;
    for _ in 0..1000 {
        let (hi, lo) = (lcg(&mut state), lcg(&mut state));
        let bits = lcg(&mut state);
        let magnitude = match bits % 3 {
            0 => Magnitude::from(lo),
            1 => Magnitude::from(lo >> (bits % 64)),
            _ => {
                let two64 = Magnitude::from(u64::MAX).add(&Magnitude::from(1));
                Magnitude::from(hi)
                    .multiply(&two64)
                    .add(&Magnitude::from(lo))
            }
        };
        let value = Value::integer(bits & 1 == 1, magnitude);
        let text = arith::show(&value);
        assert_eq!(arith::read(&text, &prelude), prelude.ok(value), "{text}");
    }
    assert_eq!(arith::read("-0", &prelude), prelude.ok(Value::Int(0)));
    assert_eq!(
        arith::read("abc", &prelude),
        prelude.parse_error("not an integer: \"abc\"")
    );
    assert_eq!(
        arith::read("", &prelude),
        prelude.parse_error("not an integer: \"\"")
    );
    assert_eq!(
        arith::read("1.5", &prelude),
        prelude.parse_error("not an integer: \"1.5\"")
    );
}

#[test]
fn a_written_form_reads_as_its_integer_or_is_not_exact() {
    let unit = unit();
    let tables = unit.tables();
    assert_eq!(
        arith::literal(&form(&tables, Some(Value::Int(42)), None)),
        Ok(Value::Int(42))
    );
    assert_eq!(
        arith::literal(&form(
            &tables,
            None,
            Some((Value::Int(20), Value::Int(-1)))
        )),
        Ok(Value::Int(2))
    );
    assert_eq!(
        arith::literal(&form(
            &tables,
            None,
            Some((Value::Int(25), Value::Int(-1)))
        )),
        Err(Reason::NotExact {
            value: "25e-1".into(),
            target: "Integer".into()
        })
    );
    assert_eq!(
        arith::literal(&form(
            &tables,
            None,
            Some((Value::Int(1), Value::Int(20)))
        )),
        Ok(big("100000000000000000000"))
    );
}
