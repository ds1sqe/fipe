//! Whether a host's reply has the shape of the type its call declared:
//! the value's tag, a tuple's or array's length, a record's meaning and
//! constructor and its field count, a primitive's kind, through every
//! part. Limits are the host's; an opaque type, a parameter or a
//! function type admits any value of its kind after rejecting genuine carriers.
//! Ordinary shape checking is copied from `src/vm/fits.rs`; native execution
//! remains resource-free, including a function's captures and filled spots.
use fipe::{
    ir,
    value::{Meaning, Prelude, Reason, Value},
};
use std::rc::Rc;

/// Borrowed traversal: shared immutable allocations are expanded once.
/// The private visit count makes the unique-edge work bound testable.
fn carrier_scan<'a>(
    roots: impl IntoIterator<Item = &'a Value>,
) -> (bool, usize) {
    scan_carriers(roots, &mut |_| Ok::<(), core::convert::Infallible>(()))
        .unwrap_or_else(|never| match never {})
}

fn scan_carriers<'a, E>(
    roots: impl IntoIterator<Item = &'a Value>,
    pay: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<(bool, usize), E> {
    let mut pending: Vec<_> = roots.into_iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut visited = 0;
    while let Some(value) = pending.pop() {
        pay(1)?;
        visited += 1;
        if matches!(value, Value::Resource(_)) {
            return Ok((true, visited));
        }
        let identity = match value {
            Value::Seq(parts) => {
                Some((0, Rc::as_ptr(parts).cast::<()>() as usize))
            }
            Value::Tuple(parts) => {
                Some((1, Rc::as_ptr(parts).cast::<()>() as usize))
            }
            Value::Record(record) => {
                Some((2, Rc::as_ptr(record).cast::<()>() as usize))
            }
            Value::Packed(packed) => {
                Some((3, Rc::as_ptr(packed).cast::<()>() as usize))
            }
            Value::Function(closure) => {
                Some((4, Rc::as_ptr(closure).cast::<()>() as usize))
            }
            _ => None,
        };
        if identity.is_some_and(|identity| !seen.insert(identity)) {
            continue;
        }
        match value {
            Value::Seq(parts) | Value::Tuple(parts) => {
                pay(u64::try_from(parts.len()).unwrap_or(u64::MAX))?;
                pending.extend(parts.iter());
            }
            Value::Record(record) => {
                pay(u64::try_from(record.fields.len()).unwrap_or(u64::MAX))?;
                pending.extend(&record.fields);
            }
            Value::Packed(packed) => pending.push(packed.value()),
            Value::Function(closure) => {
                pay(u64::try_from(
                    closure.captures.len() + closure.filled.len(),
                )
                .unwrap_or(u64::MAX))?;
                pending.extend(&closure.captures);
                pending.extend(&closure.filled);
            }
            _ => {}
        }
    }
    Ok((false, visited))
}

fn allocation(value: &Value) -> Option<(u8, usize)> {
    match value {
        Value::Seq(v) => Some((0, Rc::as_ptr(v).cast::<()>() as usize)),
        Value::Tuple(v) => Some((1, Rc::as_ptr(v).cast::<()>() as usize)),
        Value::Record(v) => Some((2, Rc::as_ptr(v).cast::<()>() as usize)),
        Value::Packed(v) => Some((3, Rc::as_ptr(v).cast::<()>() as usize)),
        Value::Function(v) => Some((4, Rc::as_ptr(v).cast::<()>() as usize)),
        _ => None,
    }
}

/// Whether `value` has the shape of `ty`; one value at a time from a
/// work list, so a deep value grows the list, not the Rust stack.
pub(super) fn fits(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
    pay: &mut impl FnMut(u64) -> Result<(), Reason>,
) -> Result<bool, Reason> {
    fit(value, ty, tables, prelude, true, pay)
}

pub(super) fn captured(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
) -> bool {
    fit(value, ty, tables, prelude, false, &mut |_| Ok(()))
        .expect("unmetered captured fitting cannot exhaust work")
}

fn fit(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
    sessions: bool,
    pay: &mut impl FnMut(u64) -> Result<(), Reason>,
) -> Result<bool, Reason> {
    if carrier_scan(core::iter::once(value)).0 {
        return Ok(false);
    }
    let mut work =
        vec![(value, Rc::new(Meaning::new(super::dispatch::copy(ty))))];
    let mut seen: Vec<((u8, usize), Rc<Meaning>)> = Vec::new();
    while let Some((value, ty)) = work.pop() {
        if let Some(identity) = allocation(value) {
            if seen.iter().any(|(known, expected)| {
                *known == identity
                    && super::dispatch::equal(expected.as_type(), ty.as_type())
            }) {
                continue;
            }
            seen.push((identity, Rc::clone(&ty)));
        }
        let fits = match (ty.as_type(), value) {
            (ir::Type::Unit, Value::Unit)
            | (ir::Type::Function { .. }, Value::Function(_)) => true,
            (
                ir::Type::Parameter(_)
                | ir::Type::Number(_)
                | ir::Type::All(_)
                | ir::Type::Not(_)
                | ir::Type::Sum(_),
                _,
            ) => !scan_carriers(core::iter::once(value), pay)?.0,
            (ir::Type::Tuple(parts), Value::Tuple(values)) => {
                let same = parts.len() == values.len();
                if same {
                    work.extend(values.iter().zip(parts).map(
                        |(value, part)| {
                            (
                                value,
                                Rc::new(Meaning::new(super::dispatch::copy(
                                    part,
                                ))),
                            )
                        },
                    ));
                }
                same
            }
            (ir::Type::Array { element, length }, Value::Seq(values)) => {
                let same = match &**length {
                    ir::Type::Number(length) => {
                        length.to_u64() == u64::try_from(values.len()).ok()
                    }
                    _ => true,
                };
                let element =
                    Rc::new(Meaning::new(super::dispatch::copy(element)));
                work.extend(
                    values.iter().map(|value| (value, Rc::clone(&element))),
                );
                same
            }
            (ir::Type::Session(slot), value) => {
                if let Some(Some(fixed)) =
                    tables.session_meanings.get(*slot).filter(|_| sessions)
                {
                    work.push((
                        value,
                        Rc::new(Meaning::new(super::dispatch::copy(fixed))),
                    ));
                } else if scan_carriers(core::iter::once(value), pay)?.0 {
                    return Ok(false);
                }
                true
            }
            (ir::Type::Packed(parts), Value::Packed(packed)) => {
                let part = parts.iter().find(|part| {
                    super::dispatch::equal(part, packed.source_type())
                });
                if let Some(part) = part {
                    work.push((
                        packed.value(),
                        Rc::new(Meaning::new(super::dispatch::copy(part))),
                    ));
                }
                part.is_some()
            }
            (ir::Type::Meaning { index, arguments }, value) => meaning(
                value, *index, arguments, tables, prelude, &mut work, pay,
            )?,
            _ => false,
        };
        if !fits {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `value` is of meaning `index`: a primitive by its kind, a
/// sequence with its elements queued, a record with its fields queued.
fn meaning<'v>(
    value: &'v Value,
    index: usize,
    arguments: &[ir::Type],
    tables: &ir::Tables,
    prelude: &Prelude,
    work: &mut Vec<(&'v Value, Rc<Meaning>)>,
    pay: &mut impl FnMut(u64) -> Result<(), Reason>,
) -> Result<bool, Reason> {
    let is = |meaning: Option<usize>| meaning == Some(index);
    if is(prelude.integer) {
        return Ok(matches!(value, Value::Int(_) | Value::Big(_)));
    }
    if is(prelude.f32) {
        return Ok(matches!(value, Value::F32(_)));
    }
    if is(prelude.f64) {
        return Ok(matches!(value, Value::F64(_)));
    }
    if is(prelude.string) {
        return Ok(matches!(value, Value::Text(_)));
    }
    if is(prelude.vec) || is(prelude.deque) {
        let Value::Seq(values) = value else {
            return Ok(false);
        };
        if let Some(element) = arguments.first() {
            let element = Rc::new(Meaning::new(super::dispatch::copy(element)));
            work.extend(
                values.iter().map(|value| (value, Rc::clone(&element))),
            );
        }
        return Ok(true);
    }
    let Some(meaning) = tables.meanings.get(index) else {
        return Ok(false);
    };
    if meaning.resource {
        return Ok(false);
    }
    // An opaque type's values are the host's to choose.
    if meaning.constructors.is_empty() {
        return Ok(!scan_carriers(core::iter::once(value), pay)?.0);
    }
    let Value::Record(record) = value else {
        return Ok(false);
    };
    let Some(constructor) = meaning.constructors.get(record.constructor) else {
        return Ok(false);
    };
    if record.meaning != index
        || constructor.fields.len() != record.fields.len()
    {
        return Ok(false);
    }
    work.extend(constructor.fields.iter().zip(&record.fields).map(
        |(field, value)| {
            let ty = super::dispatch::arguments(
                &field.ty,
                &meaning.parameters,
                arguments,
            );
            (value, Rc::new(Meaning::new(ty)))
        },
    ));
    Ok(true)
}

/// A type in the language's spelling, with an article: `an Integer`,
/// `a Vec<String>`, `a ()`.
pub(super) fn named(ty: &ir::Type, tables: &ir::Tables) -> String {
    let spelled = spell(ty, tables);
    let vowel = spelled
        .chars()
        .next()
        .is_some_and(|first| "AEIOUaeiou".contains(first));
    format!("{} {spelled}", if vowel { "an" } else { "a" })
}

fn spell(ty: &ir::Type, tables: &ir::Tables) -> String {
    enum Piece<'a> {
        Type(&'a ir::Type, bool),
        Text(String),
    }
    let mut work = vec![Piece::Type(ty, false)];
    let mut out = String::new();
    while let Some(piece) = work.pop() {
        let Piece::Type(ty, group) = piece else {
            let Piece::Text(text) = piece else {
                unreachable!()
            };
            out.push_str(&text);
            continue;
        };
        let grouped = group
            && matches!(
                ty,
                ir::Type::Tuple(_)
                    | ir::Type::Function { .. }
                    | ir::Type::Packed(_)
                    | ir::Type::All(_)
                    | ir::Type::Sum(_)
            );
        if grouped {
            out.push('(');
            work.push(Piece::Text(")".into()));
        }
        match ty {
            ir::Type::Unit => out.push_str("()"),
            ir::Type::Parameter(_) => out.push('_'),
            ir::Type::Session(i) => match tables.session_meanings.get(*i) {
                Some(Some(fixed)) => work.push(Piece::Type(fixed, false)),
                _ => out.push('_'),
            },
            ir::Type::Number(n) => out.push_str(&n.to_string()),
            ir::Type::Meaning { index, arguments } => {
                out.push_str(
                    tables
                        .meanings
                        .get(*index)
                        .map_or("_", |m| m.label.as_str()),
                );
                if !arguments.is_empty() {
                    out.push('<');
                    work.push(Piece::Text(">".into()));
                    for (i, ty) in arguments.iter().enumerate().rev() {
                        work.push(Piece::Type(ty, false));
                        if i > 0 {
                            work.push(Piece::Text(", ".into()));
                        }
                    }
                }
            }
            ir::Type::Tuple(parts)
            | ir::Type::Packed(parts)
            | ir::Type::All(parts)
            | ir::Type::Sum(parts) => {
                let between = match ty {
                    ir::Type::Tuple(_) => " * ",
                    ir::Type::Packed(_) => " | ",
                    ir::Type::All(_) => " & ",
                    _ => " + ",
                };
                for (i, ty) in parts.iter().enumerate().rev() {
                    work.push(Piece::Type(ty, true));
                    if i > 0 {
                        work.push(Piece::Text(between.into()));
                    }
                }
            }
            ir::Type::Array { element, length } => {
                out.push('[');
                work.push(Piece::Text("]".into()));
                work.push(Piece::Type(length, false));
                work.push(Piece::Text("; ".into()));
                work.push(Piece::Type(element, false));
            }
            ir::Type::Function { parameter, result } => {
                work.push(Piece::Type(result, false));
                work.push(Piece::Text(" -> ".into()));
                work.push(Piece::Type(parameter, true));
            }
            ir::Type::Not(inner) => {
                out.push('!');
                work.push(Piece::Type(inner, true));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fits(
        value: &Value,
        ty: &ir::Type,
        tables: &ir::Tables,
        prelude: &Prelude,
    ) -> bool {
        super::fits(value, ty, tables, prelude, &mut |_| Ok(())).unwrap()
    }

    fn genuine_resource() -> Value {
        let mut declarations = fipe::decl::Declarations::new();
        declarations
            .module("csv", "pub type resource Table\npub get: Table")
            .unwrap();
        let module =
            fipe::parse::parse(fipe::lex::lex("use csv\nf = _ => csv.get"))
                .unwrap();
        let unit = fipe::check::check(
            &module,
            &declarations,
            &[],
            fipe::check::Options::default(),
        )
        .unwrap();
        let ty = fipe::value::resource::ResourceType::registered(
            &unit.tables(),
            "csv",
            "Table",
        )
        .unwrap();
        fipe::value::resource::ResourceDomain::new()
            .issue(&ty, Rc::from(&b"private-native-refusal"[..]))
    }
    fn closure(captures: Vec<Value>, filled: Vec<Value>) -> Value {
        Value::Function(Rc::new(fipe::value::Closure {
            body: usize::MAX,
            captures,
            meanings: vec![],
            filled,
            guards: vec![],
        }))
    }
    fn native_refusal(_out: &mut Value) -> u32 {
        let ctx = super::super::Ctx::get();
        let resource = genuine_resource();
        let tables = ctx.tables();
        let opaque = tables
            .meanings
            .iter()
            .position(|meaning| meaning.label == "Opaque")
            .unwrap();
        for ty in [
            ir::Type::Parameter(0),
            ir::Type::Session(usize::MAX),
            ir::Type::Meaning {
                index: opaque,
                arguments: vec![],
            },
        ] {
            for nested in [
                resource.clone(),
                Value::seq(vec![resource.clone()]),
                Value::Tuple(Rc::new(vec![resource.clone()])),
                Value::Record(Rc::new(fipe::value::Record {
                    meaning: opaque,
                    constructor: 0,
                    fields: vec![resource.clone()],
                })),
            ] {
                assert!(!fits(&nested, &ty, &tables, &ctx.prelude));
            }
            assert!(fits(&Value::Int(7), &ty, &tables, &ctx.prelude));
        }
        assert!(fipe::value::Packed::from_data(
            ir::Type::Unit,
            ir::Type::Parameter(0),
            &resource
        )
        .is_none());
        let function = ir::Type::Function {
            parameter: Box::new(ir::Type::Unit),
            result: Box::new(ir::Type::Unit),
        };
        for hidden in [
            closure(vec![resource.clone()], vec![]),
            closure(vec![], vec![resource.clone()]),
        ] {
            assert!(!fits(&hidden, &function, &tables, &ctx.prelude));
        }
        assert!(fits(
            &closure(vec![Value::Int(7)], vec![]),
            &function,
            &tables,
            &ctx.prelude
        ));
        assert_eq!(super::super::dispatch::tag(&resource, &ctx.prelude), None);
        let node = ctx.program.bodies[ctx.program.main].node;
        drop(tables);
        assert!(super::super::walk::show(&ctx, resource, node).is_err());
        crate::start::with(|state| {
            assert!(
                matches!(&state.fault, Some(fipe::vm::Fault { reason: fipe::value::Reason::Message(message), .. }) if message == "native execution does not support resource values")
            );
        });
        1
    }
    #[test]
    fn resource_value_boundary_actual_native_fits_tag_and_show_refusal() {
        let (unit, _) = fipe_native_core::codec::Unit::new(
            "use plain\nf = _ => plain.get\n1",
            vec![("plain".into(), "pub type Opaque\npub get: Opaque".into())],
            vec![],
        )
        .unwrap();
        let bytes = Box::leak(unit.encode().into_boxed_slice());
        assert_eq!(crate::start::main(bytes, native_refusal), 1);
    }
    #[test]
    fn resource_value_boundary_native_scan_memoizes_shared_capture_edges() {
        let mut shared = Value::Tuple(Rc::new(vec![Value::Int(7)]));
        for _ in 0..12 {
            shared = Value::Tuple(Rc::new(vec![shared.clone(), shared]));
        }
        let (found, visits) = carrier_scan([&shared]);
        assert!(!found);
        assert!(visits <= 32);
        let hidden = closure(vec![genuine_resource()], vec![shared]);
        let (found, visits) = carrier_scan([&hidden]);
        assert!(found);
        assert!(visits <= 36);
    }
}

#[cfg(test)]
mod metered_data_tests {
    use super::*;

    #[test]
    fn logical_carrier_scan_matches_shared_dag_work_and_exact_budget() {
        let mut value = Value::Int(7);
        for _ in 0..48 {
            value = Value::Tuple(Rc::new(vec![value.clone(), value]));
        }
        let mut whole = 0;
        assert!(
            !scan_carriers([&value], &mut |units| {
                whole += units;
                Ok::<(), Reason>(())
            })
            .unwrap()
            .0
        );
        assert_eq!(whole, 193);
        for allowance in [0, whole - 1, whole] {
            let mut left = allowance;
            let result = scan_carriers([&value], &mut |units| {
                if left < units {
                    left = 0;
                    return Err(Reason::Budget);
                }
                left -= units;
                Ok(())
            });
            assert_eq!(result.is_ok(), allowance == whole);
            assert_eq!(left, 0);
        }
    }
}
