//! Whether a host's reply has the shape of the type its call declared:
//! the value's tag, a tuple's or array's length, a record's meaning and
//! constructor and its field count, a primitive's kind, through every
//! part. Limits are the host's; an opaque type, a parameter or a
//! function type admits any value of its kind.
use super::resource::materialize;
use crate::{
    ir,
    value::{Prelude, Value},
};
use alloc::{format, string::String, vec, vec::Vec};
use core::fmt::Write as _;

/// Whether `value` has the shape of `ty`; one value at a time from a
/// work list, so a deep value grows the list, not the Rust stack.
pub(super) fn fits(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
    meter: &super::resource::Work,
) -> Result<bool, crate::value::Reason> {
    fit(value, ty, tables, prelude, meter, Mode::Ordinary)
}
#[derive(Clone, Copy)]
enum Mode {
    Ordinary,
    Source,
    Retained,
}
pub(super) fn source_receiver(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
    meter: &super::resource::Work,
) -> Result<bool, crate::value::Reason> {
    fit(value, ty, tables, prelude, meter, Mode::Source)
}
pub(super) fn retained_receiver(
    value: &Value,
    ty: &ir::Type,
    field: bool,
    tables: &ir::Tables,
    prelude: &Prelude,
    meter: &super::resource::Work,
) -> Result<bool, crate::value::Reason> {
    if field && !matches!(value, Value::Record(_)) {
        return Ok(false);
    }
    fit(value, ty, tables, prelude, meter, Mode::Retained)
}
fn fit(
    value: &Value,
    ty: &ir::Type,
    tables: &ir::Tables,
    prelude: &Prelude,
    meter: &super::resource::Work,
    mode: Mode,
) -> Result<bool, crate::value::Reason> {
    use super::resource::Work;
    use crate::value::resource::Forest;
    let ordinary = Work::new(None);
    let paid = if tables.has_resources && !matches!(mode, Mode::Ordinary) {
        meter
    } else {
        &ordinary
    };
    let captured = ir::Tables::default();
    let syntax = if matches!(mode, Mode::Ordinary) {
        tables
    } else {
        &captured
    };
    let scan = if matches!(mode, Mode::Ordinary) {
        meter
    } else {
        paid
    };
    let context = Fit {
        tables,
        prelude,
        paid,
        scan,
        syntax,
        mode,
    };
    let mut pending =
        vec![(value, materialize(ty, &[], &[], syntax, paid, &mut Ok)?)];
    let mut seen: Vec<((u8, usize), Forest)> = Vec::new();
    while let Some((value, owned)) = pending.pop() {
        paid.pay(1)?;
        let ty = &owned.0[0];
        if let Some(identity) = super::resource::allocation(value) {
            let mut known = false;
            for (prior, expected) in &seen {
                paid.pay(1)?;
                if *prior == identity
                    && super::resource::equal_current(ty, &expected.0[0], paid)?
                {
                    known = true;
                    break;
                }
            }
            if known {
                continue;
            }
            seen.push((
                identity,
                materialize(ty, &[], &[], syntax, paid, &mut Ok)?,
            ));
        }
        let valid = context.part(value, ty, &mut pending)?;
        if !valid {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct Fit<'a> {
    tables: &'a ir::Tables,
    prelude: &'a Prelude,
    paid: &'a super::resource::Work,
    scan: &'a super::resource::Work,
    syntax: &'a ir::Tables,
    mode: Mode,
}
impl Fit<'_> {
    fn data_only(&self, value: &Value) -> Result<bool, crate::value::Reason> {
        if matches!(self.mode, Mode::Ordinary) {
            crate::value::resource::logical_data_only(value, &mut |units| {
                self.scan.pay(units)
            })
        } else {
            super::resource::data_only(value, self.scan)
        }
    }

    fn part<'a>(
        &self,
        value: &'a Value,
        ty: &ir::Type,
        pending: &mut Vec<(&'a Value, crate::value::resource::Forest)>,
    ) -> Result<bool, crate::value::Reason> {
        let Self { paid, syntax, .. } = *self;
        let mut queue =
            |value, part: &ir::Type| -> Result<(), crate::value::Reason> {
                pending.push((
                    value,
                    materialize(part, &[], &[], syntax, paid, &mut Ok)?,
                ));
                Ok(())
            };
        Ok(match (ty, value) {
            (ir::Type::Unit, Value::Unit)
            | (ir::Type::Function { .. }, Value::Function(_)) => true,
            (
                ir::Type::Parameter(_)
                | ir::Type::Session(_)
                | ir::Type::Number(_)
                | ir::Type::All(_)
                | ir::Type::Not(_)
                | ir::Type::Sum(_),
                _,
            ) => self.data_only(value)?,
            (ir::Type::Tuple(parts), Value::Tuple(values)) => {
                if parts.len() == values.len() {
                    for (part, value) in parts.iter().zip(values.iter()) {
                        queue(value, part)?;
                    }
                    true
                } else {
                    false
                }
            }
            (ir::Type::Array { element, length }, Value::Seq(values)) => {
                if matches!(length.as_ref(),ir::Type::Number(n) if n.to_u64()!=u64::try_from(values.len()).ok())
                {
                    false
                } else {
                    for value in values.iter() {
                        queue(value, element)?;
                    }
                    true
                }
            }
            (ir::Type::Packed(parts), Value::Packed(packed)) => {
                let mut part = None;
                for candidate in parts {
                    if super::resource::equal_current(
                        candidate,
                        packed.source_type(),
                        paid,
                    )? {
                        part = Some(candidate);
                        break;
                    }
                }
                if let Some(part) = part {
                    queue(packed.value(), part)?;
                    true
                } else {
                    false
                }
            }
            (ir::Type::Meaning { index, arguments }, value) => {
                return self.nominal(value, ty, (*index, arguments), pending);
            }
            _ => false,
        })
    }
    fn nominal<'a>(
        &self,
        value: &'a Value,
        ty: &ir::Type,
        nominal: (usize, &[ir::Type]),
        pending: &mut Vec<(&'a Value, crate::value::resource::Forest)>,
    ) -> Result<bool, crate::value::Reason> {
        let Self {
            tables,
            prelude,
            paid,
            scan: _,
            syntax,
            mode,
        } = *self;
        let (index, arguments) = nominal;
        let index = &index;
        let mut queue =
            |value, part: &ir::Type| -> Result<(), crate::value::Reason> {
                pending.push((
                    value,
                    materialize(part, &[], &[], syntax, paid, &mut Ok)?,
                ));
                Ok(())
            };
        let Some(meaning) = tables.meanings.get(*index) else {
            return Ok(false);
        };
        Ok(if meaning.resource && !matches!(mode, Mode::Ordinary) {
            if let Value::Resource(reference) = value {
                let current = super::resource::current(
                    reference.resource_type(),
                    tables,
                    paid,
                )?;
                super::resource::equal_current(ty, &current.0[0], paid)?
            } else {
                false
            }
        } else if Some(*index) == prelude.integer {
            matches!(value, Value::Int(_) | Value::Big(_))
        } else if Some(*index) == prelude.f32 {
            matches!(value, Value::F32(_))
        } else if Some(*index) == prelude.f64 {
            matches!(value, Value::F64(_))
        } else if Some(*index) == prelude.string {
            matches!(value, Value::Text(_))
        } else if Some(*index) == prelude.vec || Some(*index) == prelude.deque {
            if let Value::Seq(values) = value {
                if let Some(element) = arguments.first() {
                    for value in values.iter() {
                        queue(value, element)?;
                    }
                }
                true
            } else {
                false
            }
        } else if meaning.constructors.is_empty() {
            self.opaque(value, ty)?
        } else if let Value::Record(record) = value {
            if record.meaning != *index {
                false
            } else if let Some(constructor) =
                meaning.constructors.get(record.constructor)
            {
                if constructor.fields.len() == record.fields.len() {
                    for (field, value) in
                        constructor.fields.iter().zip(&record.fields)
                    {
                        pending.push((
                            value,
                            materialize(
                                &field.ty,
                                &meaning.parameters,
                                arguments,
                                syntax,
                                paid,
                                &mut Ok,
                            )?,
                        ));
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        })
    }
    fn opaque(
        &self,
        value: &Value,
        ty: &ir::Type,
    ) -> Result<bool, crate::value::Reason> {
        let Self {
            tables, paid, mode, ..
        } = *self;
        Ok(if self.data_only(value)? {
            true
        } else if matches!(mode, Mode::Retained | Mode::Source) {
            if let Value::Packed(packed) = value {
                if packed.association().is_some() {
                    let target = super::resource::packed_type(
                        packed, true, tables, paid,
                    )?;
                    if super::resource::equal_current(ty, &target.0[0], paid)? {
                        super::resource::validate_current(value, tables, paid)?;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        })
    }
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
    enum Job<'a> {
        Read(&'a ir::Type, bool),
        Text(&'a str),
    }
    let mut out = String::new();
    let mut jobs = vec![Job::Read(ty, false)];
    while let Some(job) = jobs.pop() {
        let Job::Read(ty, grouped) = job else {
            if let Job::Text(text) = job {
                out.push_str(text);
            }
            continue;
        };
        let wrap = grouped
            && matches!(
                ty,
                ir::Type::Tuple(_)
                    | ir::Type::Function { .. }
                    | ir::Type::Packed(_)
                    | ir::Type::All(_)
                    | ir::Type::Sum(_)
            );
        if wrap {
            out.push('(');
            jobs.push(Job::Text(")"));
        }
        match ty {
            ir::Type::Unit => out.push_str("()"),
            ir::Type::Parameter(_) => out.push('_'),
            ir::Type::Session(slot) => match tables.session_meanings.get(*slot)
            {
                Some(Some(fixed)) => jobs.push(Job::Read(fixed, false)),
                _ => out.push('_'),
            },
            ir::Type::Number(n) => {
                let _ = write!(out, "{n}");
            }
            ir::Type::Meaning { index, arguments } => {
                out.push_str(
                    tables
                        .meanings
                        .get(*index)
                        .map_or("checked value", |meaning| {
                            meaning.label.as_str()
                        }),
                );
                if !arguments.is_empty() {
                    out.push('<');
                    jobs.push(Job::Text(">"));
                    for (index, part) in arguments.iter().enumerate().rev() {
                        jobs.push(Job::Read(part, false));
                        if index > 0 {
                            jobs.push(Job::Text(", "));
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
                for (index, part) in parts.iter().enumerate().rev() {
                    jobs.push(Job::Read(part, true));
                    if index > 0 {
                        jobs.push(Job::Text(between));
                    }
                }
            }
            ir::Type::Array { element, length } => {
                out.push('[');
                jobs.push(Job::Text("]"));
                jobs.push(Job::Read(length, false));
                jobs.push(Job::Text("; "));
                jobs.push(Job::Read(element, false));
            }
            ir::Type::Function { parameter, result } => {
                jobs.push(Job::Read(result, false));
                jobs.push(Job::Text(" -> "));
                jobs.push(Job::Read(parameter, true));
            }
            ir::Type::Not(inner) => {
                out.push('!');
                jobs.push(Job::Read(inner, true));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    #[test]
    fn spelling_preserves_grouping_and_argument_order() {
        let ty = ir::Type::Function {
            parameter: Box::new(ir::Type::Tuple(vec![
                ir::Type::Unit,
                ir::Type::Parameter(7),
            ])),
            result: Box::new(ir::Type::Array {
                element: Box::new(ir::Type::Not(Box::new(
                    ir::Type::Parameter(2),
                ))),
                length: Box::new(ir::Type::Unit),
            }),
        };
        assert_eq!(
            named(&ty, &ir::Tables::default()),
            "a (() * _) -> [!_; ()]"
        );
        let alternatives = ir::Type::Packed(vec![
            ir::Type::Unit,
            ir::Type::Sum(vec![ir::Type::Parameter(1), ir::Type::Unit]),
        ]);
        assert_eq!(
            spell(&alternatives, &ir::Tables::default()),
            "() | (_ + ())"
        );
        assert_eq!(
            spell(
                &ir::Type::Meaning {
                    index: usize::MAX,
                    arguments: vec![]
                },
                &ir::Tables::default()
            ),
            "checked value"
        );
    }

    #[test]
    fn spelling_deep_dynamic_witness_uses_iterative_traversal() {
        let mut ty = ir::Type::Parameter(0);
        for _ in 0..20_000 {
            ty = ir::Type::Not(Box::new(ty));
        }
        let guarded = crate::value::Meaning::new(ty);
        let label = named(guarded.as_type(), &ir::Tables::default());
        assert_eq!(label.len(), 20_003);
        assert!(label.starts_with("a !!!"));
        assert!(label.ends_with('_'));
    }
}
