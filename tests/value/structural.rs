use super::{meaning, unit};
use alloc::rc::Rc;
use core::cmp::Ordering;
use fipe::{
    ir,
    value::{structural, Prelude, Record, Value},
};

fn record(meaning: usize, constructor: usize, fields: Vec<Value>) -> Value {
    Value::Record(Rc::new(Record {
        meaning,
        constructor,
        fields,
    }))
}

#[test]
fn show_writes_the_language_spelling() {
    let unit = unit();
    let tables = unit.tables();
    let prelude = Prelude::new(&tables);
    let point = meaning(&tables, "Point");
    let point =
        |x: i64, y: i64| record(point, 0, vec![Value::Int(x), Value::Int(y)]);
    let show = |value: &Value| structural::show(value, &tables);
    assert_eq!(show(&prelude.just(Value::Int(2))), "Just 2");
    assert_eq!(show(&prelude.nothing()), "Nothing");
    assert_eq!(show(&point(1, 2)), "Point 1 2");
    assert_eq!(show(&prelude.just(point(1, 2))), "Just (Point 1 2)");
    assert_eq!(
        show(&Value::seq(vec![Value::Int(1), Value::Int(2)])),
        "[1, 2]"
    );
    assert_eq!(
        show(&Value::Tuple(Rc::new(vec![
            Value::Int(1),
            Value::text("a")
        ]))),
        "(1, \"a\")"
    );
    assert_eq!(
        show(&Value::seq(vec![point(1, 2), prelude.nothing()])),
        "[Point 1 2, Nothing]"
    );
    assert_eq!(show(&prelude.ok(Value::seq(vec![]))), "Ok []");
    assert_eq!(show(&Value::Unit), "()");
    assert_eq!(show(&Value::F64(0.5)), "0.5");
    assert_eq!(show(&prelude.bool(true)), "True");
    assert_eq!(show(&prelude.ordering(Ordering::Less)), "Less");
    let packed = fipe::value::Packed {
        meaning: meaning(&tables, "Integer"),
        value: Value::Int(3),
    };
    assert_eq!(show(&Value::Packed(Rc::new(packed))), "3");
}

#[test]
fn equality_and_order_are_structural() {
    let unit = unit();
    let tables = unit.tables();
    let prelude = Prelude::new(&tables);
    let point = meaning(&tables, "Point");
    let point =
        |x: i64, y: i64| record(point, 0, vec![Value::Int(x), Value::Int(y)]);
    let seq = |parts: &[i64]| {
        Value::seq(parts.iter().map(|part| Value::Int(*part)).collect())
    };
    assert!(structural::eq(&point(1, 2), &point(1, 2)));
    assert!(!structural::eq(&point(1, 2), &point(2, 1)));
    assert!(structural::eq(
        &prelude.just(seq(&[1])),
        &prelude.just(seq(&[1]))
    ));
    assert!(!structural::eq(
        &prelude.just(Value::Int(1)),
        &prelude.nothing()
    ));
    assert!(structural::eq(&Value::F64(-0.0), &Value::F64(0.0)));
    assert!(!structural::eq(&seq(&[1, 2]), &seq(&[1, 2, 3])));
    assert_eq!(
        structural::cmp(&prelude.just(Value::Int(9)), &prelude.nothing()),
        Ordering::Less
    );
    assert_eq!(
        structural::cmp(
            &prelude.err(Value::Int(0)),
            &prelude.ok(Value::Int(9))
        ),
        Ordering::Greater
    );
    assert_eq!(
        structural::cmp(&seq(&[1, 2]), &seq(&[1, 2, 3])),
        Ordering::Less
    );
    assert_eq!(
        structural::cmp(&seq(&[1, 3]), &seq(&[1, 2, 3])),
        Ordering::Greater
    );
    assert_eq!(structural::cmp(&seq(&[]), &seq(&[])), Ordering::Equal);
    assert_eq!(structural::cmp(&point(1, 2), &point(1, 3)), Ordering::Less);
    assert_eq!(
        structural::cmp(&point(2, 0), &point(1, 3)),
        Ordering::Greater
    );
    assert_eq!(
        structural::cmp(&Value::text("b"), &Value::text("a")),
        Ordering::Greater
    );
    assert_eq!(
        structural::cmp(&prelude.bool(true), &prelude.bool(false)),
        Ordering::Less,
        "the prelude declares True first"
    );
}

#[test]
fn defaults_follow_the_type() {
    let unit = unit();
    let tables = unit.tables();
    let prelude = Prelude::new(&tables);
    let named = |label: &str| ir::Type::Meaning {
        index: meaning(&tables, label),
        arguments: vec![],
    };
    let default =
        |ty: &ir::Type| structural::default(ty, &tables, &prelude).unwrap();
    assert_eq!(default(&named("Integer")), Value::Int(0));
    assert_eq!(default(&named("F64")), Value::F64(0.0));
    assert_eq!(default(&named("F32")), Value::F32(0.0));
    assert_eq!(default(&named("String")), Value::text(""));
    assert_eq!(
        default(&ir::Type::Meaning {
            index: meaning(&tables, "Vec"),
            arguments: vec![named("Integer")]
        }),
        Value::seq(vec![])
    );
    assert_eq!(
        default(&ir::Type::Meaning {
            index: meaning(&tables, "Maybe"),
            arguments: vec![named("Integer")]
        }),
        prelude.nothing()
    );
    assert_eq!(default(&named("Bool")), prelude.bool(false));
    assert_eq!(default(&ir::Type::Unit), Value::Unit);
    assert_eq!(
        default(&named("Point")),
        record(
            meaning(&tables, "Point"),
            0,
            vec![Value::Int(0), Value::Int(0)]
        )
    );
    assert_eq!(
        default(&ir::Type::Tuple(vec![named("Integer"), named("String")])),
        Value::Tuple(Rc::new(vec![Value::Int(0), Value::text("")]))
    );
    let three = ir::Type::Array {
        element: Box::new(named("Integer")),
        length: Box::new(ir::Type::Number(fipe::token::Magnitude::from(3))),
    };
    assert_eq!(
        default(&three),
        Value::seq(vec![Value::Int(0), Value::Int(0), Value::Int(0)])
    );
    assert!(structural::default(&named("Ordering"), &tables, &prelude).is_err());
}
