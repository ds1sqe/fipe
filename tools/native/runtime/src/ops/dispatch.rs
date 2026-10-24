//! Run-time dispatch, copied from `src/compile/dispatch.rs` (a row by a
//! concrete type or by a head meaning, a type's parameters bound or its
//! fixed session meanings substituted: the witness substitution) and
//! `src/vm/dispatch.rs` (a row by a witness or by a value's tag, a
//! packed receiver unpacked).
use super::Ctx;
use fipe::{
    ir::{self, Type},
    value::{Prelude, Value},
};
use std::rc::Rc;

/// A selected row and the row's parameters bound by the match.
#[derive(Debug)]
pub struct Selected {
    pub row: usize,
    pub bindings: Vec<(usize, Type)>,
}

/// The row of `member` for a receiver and the first other argument:
/// matched by head meaning, then by the argument's meaning; a field row
/// wins over a member row.
pub fn select(
    tables: &ir::Tables,
    member: usize,
    receiver: &Type,
    argument: Option<&Type>,
) -> Option<Selected> {
    let mut found = None;
    for (index, row) in tables.dispatch[member].rows.iter().enumerate() {
        let mut selected = Selected {
            row: index,
            bindings: Vec::new(),
        };
        if !unify(&row.receiver, receiver, &mut selected.bindings) {
            continue;
        }
        if let (Some(pattern), Some(actual)) = (&row.argument, argument) {
            if !unify(pattern, actual, &mut selected.bindings) {
                continue;
            }
        }
        if matches!(row.target, ir::Implementation::Field(_)) {
            return Some(selected);
        }
        found.get_or_insert(selected);
    }
    found
}

/// Whether a row's type matches an actual one, binding the row's
/// parameters consistently.
fn unify(
    pattern: &Type,
    actual: &Type,
    bindings: &mut Vec<(usize, Type)>,
) -> bool {
    let mut work = vec![(pattern, actual)];
    while let Some((pattern, actual)) = work.pop() {
        if let Type::Parameter(parameter) = pattern {
            if let Some((_, bound)) =
                bindings.iter().find(|(p, _)| p == parameter)
            {
                if !equal(bound, actual) {
                    return false;
                }
            } else {
                bindings.push((*parameter, copy(actual)));
            }
            continue;
        }
        let parts = match (pattern, actual) {
            (Type::Unit, Type::Unit) => continue,
            (Type::Session(a), Type::Session(b)) if a == b => continue,
            (Type::Number(a), Type::Number(b)) if a == b => continue,
            (
                Type::Meaning {
                    index: a,
                    arguments: x,
                },
                Type::Meaning {
                    index: b,
                    arguments: y,
                },
            ) if a == b => Some((x, y)),
            (Type::Tuple(x), Type::Tuple(y))
            | (Type::Packed(x), Type::Packed(y))
            | (Type::All(x), Type::All(y))
            | (Type::Sum(x), Type::Sum(y)) => Some((x, y)),
            (
                Type::Array {
                    element: a,
                    length: x,
                },
                Type::Array {
                    element: b,
                    length: y,
                },
            )
            | (
                Type::Function {
                    parameter: a,
                    result: x,
                },
                Type::Function {
                    parameter: b,
                    result: y,
                },
            ) => {
                work.push((a, b));
                work.push((x, y));
                continue;
            }
            (Type::Not(a), Type::Not(b)) => {
                work.push((a, b));
                continue;
            }
            _ => return false,
        };
        if let Some((a, b)) = parts {
            if a.len() != b.len() {
                return false;
            }
            work.extend(a.iter().zip(b));
        }
    }
    true
}

impl Drop for Selected {
    fn drop(&mut self) {
        let mut types = Types::default();
        for (_, ty) in self.bindings.drain(..) {
            types.0.push(ty);
        }
    }
}

/// The row of `member` for a value known only by its tag, the head
/// meaning of its type: the first row whose receiver is that meaning,
/// a field row winning; the row's parameters stay unbound.
#[must_use]
pub fn select_by_head(
    tables: &ir::Tables,
    member: usize,
    meaning: usize,
) -> Option<Selected> {
    let mut found = None;
    for (index, row) in tables.dispatch[member].rows.iter().enumerate() {
        let head = match &row.receiver {
            Type::Meaning { index, .. } => *index,
            _ => continue,
        };
        if head != meaning {
            continue;
        }
        let selected = Selected {
            row: index,
            bindings: Vec::new(),
        };
        if matches!(row.target, ir::Implementation::Field(_)) {
            return Some(selected);
        }
        found.get_or_insert(selected);
    }
    found
}

/// Whether a type names no parameter and no unfixed session meaning,
/// so a dispatch on it is decided at compile time.
pub fn concrete(ty: &Type) -> bool {
    let mut work = vec![ty];
    while let Some(ty) = work.pop() {
        match ty {
            Type::Parameter(_) | Type::Session(_) => return false,
            Type::Meaning { arguments, .. }
            | Type::Tuple(arguments)
            | Type::Packed(arguments)
            | Type::All(arguments)
            | Type::Sum(arguments) => work.extend(arguments),
            Type::Array { element, length } => {
                work.push(element);
                work.push(length);
            }
            Type::Function { parameter, result } => {
                work.push(parameter);
                work.push(result);
            }
            Type::Not(inner) => work.push(inner),
            Type::Unit | Type::Number(_) => {}
        }
    }
    true
}

/// A type with each parameter replaced by its binding.
pub fn substitute(ty: &Type, bindings: &[(usize, Type)]) -> Type {
    let borrowed: Vec<_> = bindings.iter().map(|(i, t)| (*i, t)).collect();
    materialize(ty, &borrowed, None)
}
pub(super) fn arguments(
    ty: &Type,
    parameters: &[usize],
    arguments: &[Type],
) -> Type {
    let bindings: Vec<_> =
        parameters.iter().copied().zip(arguments.iter()).collect();
    materialize(ty, &bindings, None)
}
pub(super) fn frame_type(
    tables: &ir::Tables,
    ty: &Type,
    parameters: &[usize],
    arguments: &[Rc<fipe::value::Meaning>],
) -> Type {
    let bindings: Vec<_> = parameters
        .iter()
        .copied()
        .zip(arguments.iter().map(|t| t.as_type()))
        .collect();
    materialize(ty, &bindings, Some(tables))
}

/// The meaning a value's tag names: a record's, or the prelude's for a
/// number, text or sequence.
pub fn tag(value: &Value, prelude: &Prelude) -> Option<usize> {
    match value {
        Value::Record(record) => Some(record.meaning),
        Value::Packed(packed) => match packed.source_type() {
            Type::Meaning { index, .. } => Some(*index),
            _ => None,
        },
        Value::Int(_) | Value::Big(_) => prelude.integer,
        Value::F32(_) => prelude.f32,
        Value::F64(_) => prelude.f64,
        Value::Text(_) => prelude.string,
        Value::Seq(_) => prelude.vec,
        Value::Resource(_)
        | Value::Unit
        | Value::Tuple(_)
        | Value::Function(_)
        | Value::Meaning(_) => None,
    }
}

/// A selected bound receiver, with the exact layer retained or opened.
fn exposed<'a>(
    value: &'a Value,
    receiver: &Type,
    field: bool,
    tables: &ir::Tables,
    prelude: &Prelude,
) -> Result<&'a Value, fipe::value::Reason> {
    let invalid = || {
        fipe::value::Reason::Message("invalid packed receiver metadata".into())
    };
    let mut value = value;
    let mut opened = None;
    while let Value::Packed(packed) = value {
        if equal(receiver, packed.target_type())
            && !field
            && super::fits::captured(value, receiver, tables, prelude)
        {
            return Ok(value);
        }
        if !super::fits::captured(
            packed.value(),
            packed.source_type(),
            tables,
            prelude,
        ) {
            return Err(invalid());
        }
        if let Value::Packed(inner) = packed.value() {
            if !equal(packed.source_type(), inner.target_type()) {
                return Err(invalid());
            }
        }
        opened = Some(packed.source_type());
        value = packed.value();
    }
    if opened.is_some_and(|source| !equal(source, receiver)) {
        return Err(invalid());
    }
    Ok(value)
}

fn eligible(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    selected: &Selected,
    value: &Value,
) -> bool {
    let row = &tables.dispatch[member].rows[selected.row];
    let receiver = Types(vec![substitute(&row.receiver, &selected.bindings)]);
    exposed(
        value,
        &receiver.0[0],
        matches!(row.target, ir::Implementation::Field(_)),
        tables,
        prelude,
    )
    .is_ok()
}

// Descriptive roots may be public Host data: bounds precede even static priority.
fn packed_indices(value: &Value, tables: &ir::Tables) -> bool {
    let mut value = value;
    while let Value::Packed(packed) = value {
        let mut types = vec![packed.source_type(), packed.target_type()];
        while let Some(ty) = types.pop() {
            match ty {
                Type::Meaning { index, arguments } => {
                    if tables.meanings.get(*index).is_none() {
                        return false;
                    }
                    types.extend(arguments);
                }
                Type::Tuple(parts)
                | Type::Packed(parts)
                | Type::All(parts)
                | Type::Sum(parts) => types.extend(parts),
                Type::Array { element, length } => {
                    types.push(element);
                    types.push(length);
                }
                Type::Function { parameter, result } => {
                    types.push(parameter);
                    types.push(result);
                }
                Type::Not(inner) => types.push(inner),
                Type::Unit
                | Type::Parameter(_)
                | Type::Session(_)
                | Type::Number(_) => {}
            }
        }
        value = packed.value();
    }
    true
}

/// Prefer each real wrapper target, then its full source, without erasing layers.
pub fn select_for(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    ty: Option<&Type>,
    value: Option<&Value>,
) -> Option<Selected> {
    if let Some(value @ Value::Packed(_)) = value {
        if !packed_indices(value, tables) {
            return None;
        }
        if let Some(ty) = ty.filter(|ty| concrete(ty)) {
            if let Some(selected) = select(tables, member, ty, None) {
                if eligible(tables, prelude, member, &selected, value) {
                    return Some(selected);
                }
            }
        }
        let original = value;
        let mut layer = value;
        while let Value::Packed(packed) = layer {
            for ty in [packed.target_type(), packed.source_type()] {
                if let Some(selected) = select(tables, member, ty, None) {
                    if eligible(tables, prelude, member, &selected, original) {
                        return Some(selected);
                    }
                }
            }
            if let Value::Packed(inner) = packed.value() {
                if !equal(packed.source_type(), inner.target_type()) {
                    return None;
                }
            }
            layer = packed.value();
        }
        return None;
    }
    if let Some(ty) = ty.filter(|ty| concrete(ty)) {
        if let Some(selected) = select(tables, member, ty, None) {
            return Some(selected);
        }
    }
    let head = match ty {
        Some(Type::Meaning { index, .. }) => Some(*index),
        _ => None,
    };
    let meaning = value.and_then(|v| tag(v, prelude)).or(head)?;
    select_by_head(tables, member, meaning)
}

/// Validate every operand before committing an opened receiver.
pub fn unpack(
    args: &mut [Value],
    spot: usize,
    row: &ir::DispatchRow,
    bindings: &[(usize, Type)],
    eq: bool,
    cmp: bool,
    ctx: &Ctx,
) -> Result<bool, fipe::value::Reason> {
    let Some(value @ Value::Packed(_)) = args.get(spot) else {
        return Ok(true);
    };
    let tables = ctx.tables();
    let prelude = &ctx.prelude;
    if !packed_indices(value, &tables) {
        return Err(fipe::value::Reason::Message(
            "invalid packed type index".into(),
        ));
    }
    let receiver = Types(vec![substitute(&row.receiver, bindings)]);
    let field = matches!(row.target, ir::Implementation::Field(_));
    let selected = exposed(value, &receiver.0[0], field, &tables, prelude)?;
    let opened = !std::ptr::eq(value, selected);
    let mut changes = vec![(spot, selected.clone())];
    if eq || cmp {
        for (i, other) in args.iter().enumerate() {
            if i == spot {
                continue;
            }
            if let Value::Packed(packed) = other {
                if !packed_indices(other, &tables) {
                    return Err(fipe::value::Reason::Message(
                        "invalid packed type index".into(),
                    ));
                }
                // Check the other side even when unequal source syntax short-circuits.
                let other_value =
                    exposed(other, &receiver.0[0], field, &tables, prelude);
                if opened {
                    let Value::Packed(this) = value else {
                        unreachable!()
                    };
                    if !equal(this.source_type(), packed.source_type()) {
                        // Source validity is independent of compatibility with this row.
                        if !super::fits::captured(
                            packed.value(),
                            packed.source_type(),
                            &tables,
                            prelude,
                        ) {
                            return Err(fipe::value::Reason::Message(
                                "invalid packed comparison operand".into(),
                            ));
                        }
                        if cmp {
                            return Err(fipe::value::Reason::Message(
                                "incompatible packed ordering types".into(),
                            ));
                        }
                        return Ok(false);
                    }
                }
                changes.push((i, other_value?.clone()));
            }
        }
    }
    for (i, value) in changes {
        args[i] = value;
    }
    Ok(true)
}

pub fn by_tag(
    tables: &ir::Tables,
    prelude: &Prelude,
    member: usize,
    value: &Value,
) -> Option<Selected> {
    select_for(tables, prelude, member, None, Some(value))
}

/// Newly materialized type syntax is always owned by an iterative destructor.
#[derive(Default)]
pub struct Types(pub(super) Vec<Type>);
impl Drop for Types {
    fn drop(&mut self) {
        while let Some(ty) = self.0.pop() {
            match ty {
                Type::Meaning { arguments, .. }
                | Type::Tuple(arguments)
                | Type::Packed(arguments)
                | Type::All(arguments)
                | Type::Sum(arguments) => self.0.extend(arguments),
                Type::Array { element, length } => {
                    self.0.push(*element);
                    self.0.push(*length);
                }
                Type::Function { parameter, result } => {
                    self.0.push(*parameter);
                    self.0.push(*result);
                }
                Type::Not(inner) => self.0.push(*inner),
                Type::Unit
                | Type::Parameter(_)
                | Type::Session(_)
                | Type::Number(_) => {}
            }
        }
    }
}

pub(super) fn copy(ty: &Type) -> Type {
    materialize(ty, &[], None)
}

/// Borrowed full syntax equality, without recursive comparison or clones.
pub(super) fn equal(a: &Type, b: &Type) -> bool {
    let mut work = vec![(a, b)];
    while let Some((a, b)) = work.pop() {
        let parts = match (a, b) {
            (Type::Unit, Type::Unit) => continue,
            (Type::Parameter(a), Type::Parameter(b))
            | (Type::Session(a), Type::Session(b))
                if a == b =>
            {
                continue
            }
            (Type::Number(a), Type::Number(b)) if a == b => continue,
            (
                Type::Meaning {
                    index: a,
                    arguments: x,
                },
                Type::Meaning {
                    index: b,
                    arguments: y,
                },
            ) if a == b => Some((x, y)),
            (Type::Tuple(x), Type::Tuple(y))
            | (Type::Packed(x), Type::Packed(y))
            | (Type::All(x), Type::All(y))
            | (Type::Sum(x), Type::Sum(y)) => Some((x, y)),
            (
                Type::Array {
                    element: a,
                    length: x,
                },
                Type::Array {
                    element: b,
                    length: y,
                },
            )
            | (
                Type::Function {
                    parameter: a,
                    result: x,
                },
                Type::Function {
                    parameter: b,
                    result: y,
                },
            ) => {
                work.push((a, b));
                work.push((x, y));
                continue;
            }
            (Type::Not(a), Type::Not(b)) => {
                work.push((a, b));
                continue;
            }
            _ => return false,
        };
        if let Some((a, b)) = parts {
            if a.len() != b.len() {
                return false;
            }
            work.extend(a.iter().zip(b));
        }
    }
    true
}

#[derive(Clone, Copy)]
enum Shape {
    Meaning(usize),
    Tuple,
    Packed,
    All,
    Sum,
    Array,
    Function,
    Not,
}
enum Copy<'a> {
    Read(&'a Type, bool),
    Finish(Shape, usize),
}
fn materialize<'a>(
    ty: &'a Type,
    bindings: &[(usize, &'a Type)],
    tables: Option<&'a ir::Tables>,
) -> Type {
    let mut jobs = vec![Copy::Read(ty, true)];
    let mut done = Types::default();
    while let Some(job) = jobs.pop() {
        match job {
            Copy::Read(ty, replace) => {
                if let Type::Session(i) = ty {
                    if let Some(Some(fixed)) =
                        tables.and_then(|t| t.session_meanings.get(*i))
                    {
                        jobs.push(Copy::Read(fixed, replace));
                        continue;
                    }
                }
                if replace {
                    if let Type::Parameter(i) = ty {
                        if let Some((_, actual)) =
                            bindings.iter().find(|(p, _)| p == i)
                        {
                            jobs.push(Copy::Read(actual, false));
                            continue;
                        }
                    }
                }
                let (shape, children): (Shape, Vec<&Type>) = match ty {
                    Type::Unit => {
                        done.0.push(Type::Unit);
                        continue;
                    }
                    Type::Parameter(i) => {
                        done.0.push(Type::Parameter(*i));
                        continue;
                    }
                    Type::Session(i) => {
                        done.0.push(Type::Session(*i));
                        continue;
                    }
                    Type::Number(n) => {
                        done.0.push(Type::Number(n.clone()));
                        continue;
                    }
                    Type::Meaning { index, arguments } => {
                        (Shape::Meaning(*index), arguments.iter().collect())
                    }
                    Type::Tuple(p) => (Shape::Tuple, p.iter().collect()),
                    Type::Packed(p) => (Shape::Packed, p.iter().collect()),
                    Type::All(p) => (Shape::All, p.iter().collect()),
                    Type::Sum(p) => (Shape::Sum, p.iter().collect()),
                    Type::Array { element, length } => {
                        (Shape::Array, vec![element, length])
                    }
                    Type::Function { parameter, result } => {
                        (Shape::Function, vec![parameter, result])
                    }
                    Type::Not(inner) => (Shape::Not, vec![inner]),
                };
                jobs.push(Copy::Finish(shape, children.len()));
                jobs.extend(
                    children.into_iter().rev().map(|t| Copy::Read(t, replace)),
                );
            }
            Copy::Finish(shape, n) => {
                let mut children = Types(done.0.split_off(done.0.len() - n));
                let ty = match shape {
                    Shape::Meaning(index) => Type::Meaning {
                        index,
                        arguments: std::mem::take(&mut children.0),
                    },
                    Shape::Tuple => {
                        Type::Tuple(std::mem::take(&mut children.0))
                    }
                    Shape::Packed => {
                        Type::Packed(std::mem::take(&mut children.0))
                    }
                    Shape::All => Type::All(std::mem::take(&mut children.0)),
                    Shape::Sum => Type::Sum(std::mem::take(&mut children.0)),
                    Shape::Array => {
                        let length = Box::new(children.0.pop().unwrap());
                        let element = Box::new(children.0.pop().unwrap());
                        Type::Array { element, length }
                    }
                    Shape::Function => {
                        let result = Box::new(children.0.pop().unwrap());
                        let parameter = Box::new(children.0.pop().unwrap());
                        Type::Function { parameter, result }
                    }
                    Shape::Not => {
                        Type::Not(Box::new(children.0.pop().unwrap()))
                    }
                };
                done.0.push(ty);
            }
        }
    }
    done.0.pop().expect("one copied root")
}
