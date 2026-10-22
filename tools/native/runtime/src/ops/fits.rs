//! Whether a host's reply has the shape of the type its call declared:
//! the value's tag, a tuple's or array's length, a record's meaning and
//! constructor and its field count, a primitive's kind, through every
//! part. Limits are the host's; an opaque type, a parameter or a
//! function type admits any value of its kind. Copied whole from
//! `src/vm/fits.rs`.
use fipe::{
    ir,
    value::{Prelude, Value},
};
use std::rc::Rc;

/// Whether `value` has the shape of `ty`; one value at a time from a
/// work list, so a deep value grows the list, not the Rust stack.
pub(super) fn fits(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
) -> bool {
    let mut work = vec![(value, Rc::new(ty.clone()))];
    while let Some((value, ty)) = work.pop() {
        let fits = match (&*ty, value) {
            (ir::Type::Unit, Value::Unit)
            | (ir::Type::Function { .. }, Value::Function(_))
            | (
                ir::Type::Parameter(_)
                | ir::Type::Number(_)
                | ir::Type::All(_)
                | ir::Type::Not(_)
                | ir::Type::Sum(_),
                _,
            ) => true,
            (ir::Type::Tuple(parts), Value::Tuple(values)) => {
                let same = parts.len() == values.len();
                if same {
                    work.extend(
                        values.iter().zip(parts).map(|(value, part)| {
                            (value, Rc::new(part.clone()))
                        }),
                    );
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
                let element = Rc::new((**element).clone());
                work.extend(
                    values.iter().map(|value| (value, Rc::clone(&element))),
                );
                same
            }
            (ir::Type::Session(slot), value) => {
                if let Some(Some(fixed)) = tables.session_meanings.get(*slot) {
                    work.push((value, Rc::new(fixed.clone())));
                }
                true
            }
            (ir::Type::Packed(parts), Value::Packed(packed)) => {
                let part = parts.iter().find(|part| {
                    matches!(part, ir::Type::Meaning { index, .. } if *index == packed.meaning)
                });
                if let Some(part) = part {
                    work.push((&packed.value, Rc::new(part.clone())));
                }
                part.is_some()
            }
            (ir::Type::Meaning { index, arguments }, value) => {
                meaning(value, *index, arguments, tables, prelude, &mut work)
            }
            _ => false,
        };
        if !fits {
            return false;
        }
    }
    true
}

/// Whether `value` is of meaning `index`: a primitive by its kind, a
/// sequence with its elements queued, a record with its fields queued.
fn meaning<'v>(
    value: &'v Value,
    index: usize,
    arguments: &[ir::Type],
    tables: &ir::Tables,
    prelude: &Prelude,
    work: &mut Vec<(&'v Value, Rc<ir::Type>)>,
) -> bool {
    let is = |meaning: Option<usize>| meaning == Some(index);
    if is(prelude.integer) {
        return matches!(value, Value::Int(_) | Value::Big(_));
    }
    if is(prelude.f32) {
        return matches!(value, Value::F32(_));
    }
    if is(prelude.f64) {
        return matches!(value, Value::F64(_));
    }
    if is(prelude.string) {
        return matches!(value, Value::Text(_));
    }
    if is(prelude.vec) || is(prelude.deque) {
        let Value::Seq(values) = value else {
            return false;
        };
        if let Some(element) = arguments.first() {
            let element = Rc::new(element.clone());
            work.extend(
                values.iter().map(|value| (value, Rc::clone(&element))),
            );
        }
        return true;
    }
    let meaning = &tables.meanings[index];
    // An opaque type's values are the host's to choose.
    if meaning.constructors.is_empty() {
        return true;
    }
    let Value::Record(record) = value else {
        return false;
    };
    let Some(constructor) = meaning.constructors.get(record.constructor) else {
        return false;
    };
    if record.meaning != index
        || constructor.fields.len() != record.fields.len()
    {
        return false;
    }
    work.extend(constructor.fields.iter().zip(&record.fields).map(
        |(field, value)| {
            let ty = field.ty.substituted(&meaning.parameters, arguments);
            (value, Rc::new(ty))
        },
    ));
    true
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
    let list = |parts: &[ir::Type], between: &str| {
        parts
            .iter()
            .map(|part| grouped(part, tables))
            .collect::<Vec<_>>()
            .join(between)
    };
    match ty {
        ir::Type::Unit => String::from("()"),
        ir::Type::Parameter(_) => String::from("_"),
        ir::Type::Session(slot) => match tables.session_meanings.get(*slot) {
            Some(Some(fixed)) => spell(fixed, tables),
            _ => String::from("_"),
        },
        ir::Type::Number(number) => format!("{number}"),
        ir::Type::Meaning { index, arguments } => {
            let label = &tables.meanings[*index].label;
            if arguments.is_empty() {
                label.clone()
            } else {
                let arguments: Vec<_> =
                    arguments.iter().map(|ty| spell(ty, tables)).collect();
                format!("{label}<{}>", arguments.join(", "))
            }
        }
        ir::Type::Tuple(parts) => list(parts, " * "),
        ir::Type::Array { element, length } => {
            format!("[{}; {}]", spell(element, tables), spell(length, tables))
        }
        ir::Type::Function { parameter, result } => {
            format!(
                "{} -> {}",
                grouped(parameter, tables),
                spell(result, tables)
            )
        }
        ir::Type::Packed(parts) => list(parts, " | "),
        ir::Type::All(parts) => list(parts, " & "),
        ir::Type::Sum(parts) => list(parts, " + "),
        ir::Type::Not(inner) => format!("!{}", grouped(inner, tables)),
    }
}

/// A part in parentheses when it is itself made of parts.
fn grouped(ty: &ir::Type, tables: &ir::Tables) -> String {
    let spelled = spell(ty, tables);
    match ty {
        ir::Type::Tuple(_)
        | ir::Type::Function { .. }
        | ir::Type::Packed(_)
        | ir::Type::All(_)
        | ir::Type::Sum(_) => format!("({spelled})"),
        _ => spelled,
    }
}
