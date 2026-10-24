use alloc::rc::Rc;
use fipe::{
    check,
    decl::Declarations,
    ir,
    lex::lex,
    parse::parse,
    value::{Prelude, Record, Value},
};

extern crate alloc;

/// A unit naming every prelude meaning the runtime constructs, so its
/// tables carry their layouts.
const NAMING: &str = "\
t = (True, Less, Just 1, Ok 1, Left 1, \"a\", [1], 1.5)
u: F32 = 1.5
v = \"3\" | read<I64>
w = utf8 [1]
c = count [1]
d: Deque<Integer> = [1]
type Mask: Literal { bits: U8 }
impl Mask: Literal { literal = f => Self 0 }
m: Mask = 7
type Point { x: Integer, y: Integer }
pt = Point 1 2
";

/// A linear congruential generator over 64 bits, for property samples.
const fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

/// The meaning index of a label in the unit's tables.
fn meaning(tables: &ir::Tables, label: &str) -> usize {
    tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == label)
        .unwrap_or_else(|| panic!("the unit names {label}"))
}

/// A number form: `Whole` of an integer, or `Fraction` of digits and an
/// exponent, with empty affixes.
fn form(
    tables: &ir::Tables,
    whole: Option<Value>,
    fraction: Option<(Value, Value)>,
) -> Value {
    let meaning = meaning(tables, "Form");
    let text = || Value::text("");
    let (constructor, fields) = match (whole, fraction) {
        (Some(value), None) => (
            0,
            vec![value, Value::Int(10), Value::Int(1), text(), text()],
        ),
        (None, Some((digits, exponent))) => {
            (1, vec![digits, exponent, text(), text()])
        }
        _ => unreachable!(),
    };
    Value::Record(Rc::new(Record {
        meaning,
        constructor,
        fields,
    }))
}

fn unit() -> ir::Unit {
    let module = parse(lex(NAMING)).unwrap();
    check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
}

#[test]
fn the_prelude_finds_every_meaning_the_runtime_constructs() {
    let unit = unit();
    let prelude = Prelude::new(&unit.tables());
    assert!(prelude.bool.is_some(), "{prelude:?}");
    assert!(prelude.ordering.is_some(), "{prelude:?}");
    assert!(prelude.maybe.is_some(), "{prelude:?}");
    assert!(prelude.result.is_some(), "{prelude:?}");
    assert!(prelude.either.is_some(), "{prelude:?}");
    assert!(prelude.parse_error.is_some(), "{prelude:?}");
    assert!(prelude.utf8_error.is_some(), "{prelude:?}");
    assert!(prelude.count.is_some(), "{prelude:?}");
    assert!(prelude.form.is_some(), "{prelude:?}");
    assert!(prelude.integer.is_some(), "{prelude:?}");
    assert!(prelude.f32.is_some(), "{prelude:?}");
    assert!(prelude.f64.is_some(), "{prelude:?}");
    assert!(prelude.string.is_some(), "{prelude:?}");
    assert!(prelude.vec.is_some(), "{prelude:?}");
    assert!(prelude.deque.is_some(), "{prelude:?}");
    let tables = unit.tables();
    let labels = |meaning: usize| -> Vec<String> {
        tables.meanings[meaning]
            .constructors
            .iter()
            .map(|constructor| constructor.label.clone())
            .collect()
    };
    assert_eq!(labels(prelude.bool.unwrap()), ["True", "False"]);
    assert_eq!(
        labels(prelude.ordering.unwrap()),
        ["Less", "Equal", "Greater"]
    );
    assert_eq!(labels(prelude.maybe.unwrap()), ["Just", "Nothing"]);
    assert_eq!(labels(prelude.result.unwrap()), ["Ok", "Err"]);
    assert_eq!(labels(prelude.either.unwrap()), ["Left", "Right"]);
    assert_eq!(
        labels(prelude.form.unwrap()),
        ["Whole", "Fraction", "Text", "Bytes"]
    );
}

#[path = "value/arith.rs"]
mod arith;
#[path = "value/float.rs"]
mod float;
#[path = "value/structural.rs"]
mod structural;
#[path = "value/text.rs"]
mod text;

#[test]
fn hidden_meaning_preserves_borrowed_type_debug_and_single_rc_alias() {
    let ty = ir::Type::Tuple(vec![ir::Type::Unit, ir::Type::Unit]);
    let expected_debug = format!("{ty:?}");
    let witness = Rc::new(fipe::value::Meaning::new(ty.clone()));
    assert_eq!(witness.as_type(), &ty);
    assert_eq!(format!("{witness:?}"), expected_debug);
    let value = Value::Meaning(Rc::clone(&witness));
    let alias = value.clone();
    let Value::Meaning(shared) = &alias else {
        unreachable!()
    };
    assert!(Rc::ptr_eq(&witness, shared));
    assert_eq!(Rc::strong_count(&witness), 3);
    assert_eq!(
        value,
        Value::Meaning(Rc::new(fipe::value::Meaning::new(ty)))
    );
    drop(value);
    assert_eq!(Rc::strong_count(&witness), 2);
    drop(alias);
    assert_eq!(Rc::strong_count(&witness), 1);
    assert_eq!(core::mem::size_of::<Value>(), 16);
    assert_eq!(core::mem::align_of::<Value>(), 8);
}
