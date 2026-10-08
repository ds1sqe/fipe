use super::{form, lcg, unit};
use core::cmp::Ordering;
use fipe::value::{float, Prelude, Reason, Value};

const fn f(value: f64) -> Value {
    Value::F64(value)
}

fn bits(value: &Value) -> u64 {
    match value {
        Value::F64(float) => float.to_bits(),
        other => panic!("{other:?}"),
    }
}

/// Within one unit in the last place of the reference.
fn close(value: Result<Value, Reason>, reference: f64) {
    let value = value.unwrap();
    let got = bits(&value);
    let want = reference.to_bits();
    assert!(got.abs_diff(want) <= 1, "{value:?} against {reference:?}");
}

fn exact(value: Result<Value, Reason>, reference: f64) {
    let value = value.unwrap();
    assert_eq!(
        bits(&value),
        reference.to_bits(),
        "{value:?} against {reference:?}"
    );
}

#[test]
fn arithmetic_rounds_and_faults_instead_of_producing_a_non_finite_value() {
    assert_eq!(
        float::show(&float::div(&f(1.0), &f(3.0)).unwrap()),
        "0.3333333333333333"
    );
    assert_eq!(
        float::show(&float::add(&f(0.1), &f(0.2)).unwrap()),
        "0.30000000000000004"
    );
    assert_eq!(float::div(&f(1.0), &f(0.0)), Err(Reason::NotFinite));
    assert_eq!(float::div(&f(0.0), &f(0.0)), Err(Reason::NotFinite));
    assert_eq!(float::mul(&f(1e308), &f(10.0)), Err(Reason::NotFinite));
    assert_eq!(float::sub(&f(-1e308), &f(1e308)), Err(Reason::NotFinite));
    assert_eq!(float::rem(&f(5.5), &f(2.0)), Ok(f(1.5)));
    assert_eq!(float::rem(&f(-5.5), &f(2.0)), Ok(f(-1.5)));
    assert_eq!(float::rem(&f(1.0), &f(0.0)), Err(Reason::NotFinite));
    assert_eq!(
        float::add(&Value::F32(0.1), &Value::F32(0.2)),
        Ok(Value::F32(0.3))
    );
    assert_eq!(
        float::show(&float::add(&Value::F32(0.1), &Value::F32(0.2)).unwrap()),
        "0.3"
    );
    assert_eq!(
        float::mul(&Value::F32(1e38), &Value::F32(10.0)),
        Err(Reason::NotFinite)
    );
    assert_eq!(float::neg(&f(0.0)), f(-0.0));
    assert_eq!(float::zero(true), f(0.0));
    assert_eq!(float::zero(false), Value::F32(0.0));
}

#[test]
fn signed_zero_is_equal_and_shown_with_its_sign() {
    assert!(float::eq(&f(-0.0), &f(0.0)));
    assert_eq!(float::cmp(&f(-0.0), &f(0.0)), Ordering::Equal);
    assert_eq!(float::show(&f(-0.0)), "-0.0");
    assert_eq!(float::cmp(&f(-1.0), &f(1.0)), Ordering::Less);
    assert_eq!(
        float::cmp(&Value::F32(2.0), &Value::F32(1.0)),
        Ordering::Greater
    );
}

#[test]
fn show_is_the_shortest_round_trip_with_a_fraction_part_and_no_exponent() {
    assert_eq!(float::show(&f(1e21)), "1000000000000000000000.0");
    assert_eq!(float::show(&f(1e-7)), "0.0000001");
    assert_eq!(float::show(&f(2.0)), "2.0");
    assert_eq!(
        float::show(&f(18_446_744_073_709_551_616.0)),
        "18446744073709552000.0"
    );
    assert_eq!(float::show(&Value::F32(1.5)), "1.5");
    assert_eq!(float::show(&Value::F32(0.1)), "0.1");
}

#[test]
fn show_then_read_round_trips_every_finite_value() {
    let unit = unit();
    let prelude = Prelude::new(&unit.tables());
    let mut state = 11;
    let mut seen = 0;
    while seen < 1000 {
        let value = f64::from_bits(lcg(&mut state));
        if !value.is_finite() {
            continue;
        }
        seen += 1;
        let shown = float::show(&f(value));
        assert_eq!(
            float::read(&shown, true, &prelude),
            prelude.ok(f(value)),
            "{shown}"
        );
    }
    seen = 0;
    while seen < 1000 {
        let value = f32::from_bits((lcg(&mut state) >> 32) as u32);
        if !value.is_finite() {
            continue;
        }
        seen += 1;
        let shown = float::show(&Value::F32(value));
        assert_eq!(
            float::read(&shown, false, &prelude),
            prelude.ok(Value::F32(value)),
            "{shown}"
        );
    }
    assert_eq!(
        float::read("inf", true, &prelude),
        prelude.parse_error("not a number: \"inf\"")
    );
    assert_eq!(
        float::read("abc", true, &prelude),
        prelude.parse_error("not a number: \"abc\"")
    );
    assert_eq!(
        float::read("1e39", false, &prelude),
        prelude.parse_error("not a number: \"1e39\"")
    );
    assert_eq!(float::read("3", true, &prelude), prelude.ok(f(3.0)));
}

#[test]
fn the_roundings_follow_ieee_with_ties_to_even() {
    let round = |x: f64| bits(&float::round(&f(x)));
    assert_eq!(round(2.5), 2.0_f64.to_bits());
    assert_eq!(round(3.5), 4.0_f64.to_bits());
    assert_eq!(round(-2.5), (-2.0_f64).to_bits());
    assert_eq!(round(-0.3), (-0.0_f64).to_bits());
    assert_eq!(round(0.499_999_999_999_999_94), 0.0_f64.to_bits());
    assert_eq!(round(1e300), 1e300_f64.to_bits());
    assert_eq!(float::floor(&f(-0.5)), f(-1.0));
    assert_eq!(bits(&float::ceil(&f(-0.5))), (-0.0_f64).to_bits());
    assert_eq!(float::ceil(&f(0.5)), f(1.0));
    assert_eq!(float::trunc(&f(2.7)), f(2.0));
    assert_eq!(float::trunc(&f(-2.7)), f(-2.0));
    assert_eq!(float::floor(&f(2.0)), f(2.0));
    assert_eq!(float::round(&Value::F32(2.5)), Value::F32(2.0));
    assert_eq!(float::floor(&Value::F32(-0.5)), Value::F32(-1.0));
    assert_eq!(float::trunc(&Value::F32(1e30)), Value::F32(1e30));
}

#[test]
fn the_square_root_is_correctly_rounded() {
    exact(float::sqrt(&f(2.0)), core::f64::consts::SQRT_2);
    exact(float::sqrt(&f(3.0)), 1.732_050_807_568_877_2);
    exact(float::sqrt(&f(0.5)), core::f64::consts::FRAC_1_SQRT_2);
    exact(float::sqrt(&f(4.0)), 2.0);
    exact(float::sqrt(&f(1e300)), 1e150);
    exact(float::sqrt(&f(1e-320)), 9.999_944_335_758_49e-161);
    exact(float::sqrt(&f(0.0)), 0.0);
    assert_eq!(float::sqrt(&f(-1.0)), Err(Reason::NotFinite));
    // Against the squares of a thousand roots: the root of a square is
    // exactly the number when the square is exact.
    let mut state = 3;
    for _ in 0..1000 {
        let root = f64::from_bits((lcg(&mut state) >> 12) | (0x3ff << 52));
        let square = root * root;
        if (square / root).to_bits() == root.to_bits() {
            exact(float::sqrt(&f(square)), root);
        }
    }
}

#[test]
fn the_trigonometric_functions_are_within_one_ulp_of_the_platform() {
    close(float::sin(&f(0.5)), 0.479_425_538_604_203);
    close(float::cos(&f(0.5)), 0.877_582_561_890_372_8);
    close(float::sin(&f(1.0)), 0.841_470_984_807_896_5);
    close(float::cos(&f(1.0)), 0.540_302_305_868_139_8);
    close(float::sin(&f(100.0)), -0.506_365_641_109_758_8);
    close(float::cos(&f(100.0)), 0.862_318_872_287_683_9);
    close(float::sin(&f(1e6)), -0.349_993_502_171_292_94);
    close(float::cos(&f(1e6)), 0.936_752_127_533_144_7);
    close(float::sin(&f(-2.0)), -0.909_297_426_825_681_7);
    close(float::cos(&f(3.0)), -0.989_992_496_600_445_4);
    exact(float::sin(&f(1e-9)), 1e-9);
    exact(float::cos(&f(0.0)), 1.0);
    exact(float::sin(&f(0.0)), 0.0);
    close(float::atan2(&f(1.0), &f(1.0)), core::f64::consts::FRAC_PI_4);
    close(float::atan2(&f(1.0), &f(-1.0)), 2.356_194_490_192_345);
    close(float::atan2(&f(-1.0), &f(-1.0)), -2.356_194_490_192_345);
    close(float::atan2(&f(0.5), &f(2.0)), 0.244_978_663_126_864_14);
    close(float::atan2(&f(1.0), &f(0.0)), core::f64::consts::FRAC_PI_2);
    close(float::atan2(&f(-3.0), &f(0.1)), -1.537_475_330_916_649_3);
    close(
        float::atan2(&f(2.0), &f(1e-300)),
        core::f64::consts::FRAC_PI_2,
    );
    close(float::atan2(&f(1e-300), &f(-2.0)), core::f64::consts::PI);
    exact(float::atan2(&f(0.0), &f(1.0)), 0.0);
    exact(float::atan2(&f(-0.0), &f(-1.0)), -core::f64::consts::PI);
    close(float::hypot(&f(3.0), &f(4.0)), 5.0);
    close(
        float::hypot(&f(1e200), &f(1e200)),
        1.414_213_562_373_095e200,
    );
    close(
        float::hypot(&f(1e-200), &f(1e-200)),
        1.414_213_562_373_095e-200,
    );
    close(float::hypot(&f(5.0), &f(1e-300)), 5.0);
    close(float::hypot(&f(0.1), &f(0.2)), 0.223_606_797_749_979);
    close(
        float::hypot(&f(1e308), &f(1e308)),
        1.414_213_562_373_095_1e308,
    );
    assert_eq!(
        float::hypot(&f(1.5e308), &f(1.5e308)),
        Err(Reason::NotFinite)
    );
}

#[test]
fn a_written_form_reads_as_the_nearest_float_or_is_not_finite() {
    let unit = unit();
    let tables = unit.tables();
    let fraction = |digits: i64, exponent: i64| {
        form(
            &tables,
            None,
            Some((Value::Int(digits), Value::Int(exponent))),
        )
    };
    assert_eq!(float::literal(&fraction(25, -1), true), Ok(f(2.5)));
    assert_eq!(float::literal(&fraction(1, -1), true), Ok(f(0.1)));
    assert_eq!(float::literal(&fraction(1, -1), false), Ok(Value::F32(0.1)));
    assert_eq!(
        float::literal(&form(&tables, Some(Value::Int(1)), None), true),
        Ok(f(1.0))
    );
    assert_eq!(
        float::literal(&fraction(1, 400), true),
        Err(Reason::NotFinite)
    );
    assert_eq!(
        float::literal(&fraction(1, 39), false),
        Err(Reason::NotFinite)
    );
    assert_eq!(float::literal(&fraction(1, 39), true), Ok(f(1e39)));
    assert_eq!(float::literal(&fraction(1, -400), true), Ok(f(0.0)));
}
