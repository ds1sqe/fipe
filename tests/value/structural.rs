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
    let packed = fipe::value::Packed::from_data(
        ir::Type::Meaning {
            index: meaning(&tables, "Integer"),
            arguments: vec![],
        },
        ir::Type::Meaning {
            index: meaning(&tables, "Eq"),
            arguments: vec![],
        },
        &Value::Int(3),
    )
    .unwrap();
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

#[test]
fn packed_equality_uses_full_source_but_not_target_metadata() {
    use fipe::value::Packed;
    let source = ir::Type::Tuple(vec![ir::Type::Parameter(0), ir::Type::Unit]);
    let payload = Value::Tuple(Rc::new(vec![Value::Int(7), Value::Unit]));
    let pack = |source, target| {
        Value::Packed(Rc::new(
            Packed::from_data(source, target, &payload).unwrap(),
        ))
    };
    let a = pack(source.clone(), ir::Type::Unit);
    let b = pack(source.clone(), ir::Type::Parameter(1));
    let c = pack(source.clone(), ir::Type::Session(2));
    assert_eq!(a, b);
    assert_eq!(b, c);
    assert_eq!(a, c);
    assert!(structural::eq(&a, &c));
    assert_ne!(
        a,
        pack(
            ir::Type::Tuple(vec![ir::Type::Parameter(1), ir::Type::Unit]),
            ir::Type::Unit
        )
    );
    let Value::Packed(packed) = &b else {
        unreachable!()
    };
    assert_eq!(packed.source_type(), &source);
    assert_eq!(packed.target_type(), &ir::Type::Parameter(1));
    assert_eq!(packed.value(), &payload);
    let cloned = packed.as_ref().clone();
    assert_eq!(cloned, **packed);
}

#[test]
fn packed_order_preserves_nominal_head_payload_order_and_same_structural_source(
) {
    use fipe::value::Packed;
    let pack = |source, value| {
        Value::Packed(Rc::new(
            Packed::from_data(source, ir::Type::Unit, &Value::Int(value))
                .unwrap(),
        ))
    };
    let nominal = |index, argument| ir::Type::Meaning {
        index,
        arguments: vec![argument],
    };
    let a = pack(nominal(3, ir::Type::Unit), 1);
    let b = pack(nominal(3, ir::Type::Parameter(0)), 2);
    assert_ne!(a, b);
    assert_eq!(structural::cmp(&a, &b), Ordering::Less);
    assert_eq!(
        structural::cmp(&b, &pack(nominal(4, ir::Type::Unit), 0)),
        Ordering::Less
    );
    let source = ir::Type::Tuple(vec![ir::Type::Unit]);
    assert_eq!(
        structural::cmp(&pack(source.clone(), 1), &pack(source, 2)),
        Ordering::Less
    );
}

#[test]
fn packed_deep_metadata_clone_and_last_drop_use_guarded_ownership() {
    let mut source = ir::Type::Unit;
    let mut target = ir::Type::Unit;
    for _ in 0..4096 {
        source = ir::Type::Function {
            parameter: alloc::boxed::Box::new(source),
            result: alloc::boxed::Box::new(ir::Type::Unit),
        };
        target = ir::Type::Not(alloc::boxed::Box::new(target));
    }
    let packed =
        fipe::value::Packed::from_data(source, target, &Value::Int(7)).unwrap();
    let alias = packed.clone();
    assert_eq!(packed.value(), &Value::Int(7));
    drop(packed);
    assert_eq!(alias.value(), &Value::Int(7));
    drop(alias);
}

#[test]
fn packed_data_constructor_visits_shared_payload_allocations_once() {
    let mut payload = Value::Int(7);
    for _ in 0..48 {
        payload = Value::Tuple(Rc::new(vec![payload.clone(), payload]));
    }
    // Forty-eight shared levels represent 2^48 leaves, but only 48 allocations.
    let packed = fipe::value::Packed::from_data(
        ir::Type::Unit,
        ir::Type::Unit,
        &payload,
    )
    .unwrap();
    let (Value::Tuple(actual), Value::Tuple(original)) =
        (packed.value(), &payload)
    else {
        unreachable!()
    };
    assert!(Rc::ptr_eq(actual, original));
}
