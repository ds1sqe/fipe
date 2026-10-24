//! Opaque Host references. These types prove checked language identity only;
//! liveness, grants, payloads and closing remain the Host's responsibility.
use crate::{
    ir::{self, Type},
    value::Value,
};
use alloc::{rc::Rc, vec::Vec};
use core::{cell::RefCell, fmt};

/// A Host issuer identity. Cloning preserves the issuer; `new` creates another.
#[derive(Clone)]
pub struct ResourceDomain(Rc<()>);
impl ResourceDomain {
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(()))
    }
    /// Wrap an already authorized Host entry. This does not acquire or validate it.
    #[must_use]
    pub fn issue(&self, ty: &ResourceType, key: Rc<[u8]>) -> Value {
        Value::Resource(Rc::new(ResourceRef {
            domain: self.clone(),
            ty: ty.clone(),
            key,
        }))
    }
}
impl Default for ResourceDomain {
    fn default() -> Self {
        Self::new()
    }
}
impl PartialEq for ResourceDomain {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ResourceDomain {}
impl fmt::Debug for ResourceDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<resource>")
    }
}

/// An exact checked resource type, independent of later table mutation.
#[derive(Clone)]
pub struct ResourceType(pub(crate) Rc<Descriptor>);
/// A pointer-like reference whose hidden entry key is interpreted only by its Host.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceRef {
    domain: ResourceDomain,
    ty: ResourceType,
    key: Rc<[u8]>,
}
impl ResourceRef {
    #[must_use]
    pub const fn domain(&self) -> &ResourceDomain {
        &self.domain
    }
    #[must_use]
    pub const fn resource_type(&self) -> &ResourceType {
        &self.ty
    }
    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }
}
impl fmt::Debug for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<resource>")
    }
}
impl fmt::Debug for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<resource>")
    }
}

/// Why a checked resource witness cannot be obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceError {
    NotFound,
    Ambiguous,
    NotResource,
    NotMonomorphic,
    NoPendingCall,
    NotResourceResult,
    NonConcrete,
}
impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "resource type not found",
            Self::Ambiguous => "ambiguous resource type",
            Self::NotResource => "type is not a resource",
            Self::NotMonomorphic => "resource type requires arguments",
            Self::NoPendingCall => "no pending Host call",
            Self::NotResourceResult => "result is not a direct resource",
            Self::NonConcrete => "resource identity is not concrete",
        })
    }
}
impl core::error::Error for ResourceError {}

/// Sparse nominal identities. Copying Tables deliberately starts a fresh context.
#[derive(Default)]
pub(crate) struct ResourceCache(RefCell<Vec<(usize, Rc<()>)>>);
impl Clone for ResourceCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for ResourceCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for ResourceCache {}
impl fmt::Debug for ResourceCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResourceCache")
    }
}
impl ResourceCache {
    pub(crate) fn len(&self) -> usize {
        self.0.borrow().len()
    }
    pub(crate) fn stamp(&self, index: usize) -> Rc<()> {
        let mut slots = self.0.borrow_mut();
        if let Some((_, stamp)) = slots.iter().find(|(slot, _)| *slot == index)
        {
            return Rc::clone(stamp);
        }
        let stamp = Rc::new(());
        slots.push((index, Rc::clone(&stamp)));
        stamp
    }
    pub(crate) fn index(&self, stamp: &Rc<()>) -> Option<usize> {
        self.0
            .borrow()
            .iter()
            .find(|(_, known)| Rc::ptr_eq(known, stamp))
            .map(|(index, _)| *index)
    }
    pub(crate) fn entries(&self) -> core::cell::Ref<'_, [(usize, Rc<()>)]> {
        core::cell::Ref::map(self.0.borrow(), |entries| entries.as_slice())
    }
    pub(crate) fn truncate(&self, length: usize) {
        self.0.borrow_mut().retain(|(index, _)| *index < length);
    }
}

/// An owned type forest that always destroys recursive syntax iteratively.
#[derive(Default)]
pub(crate) struct Forest(pub(crate) Vec<Type>);
impl fmt::Debug for Forest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TypeForest")
    }
}
impl Drop for Forest {
    fn drop(&mut self) {
        while let Some(ty) = self.0.pop() {
            drain(ty, &mut self.0);
        }
    }
}
fn drain(ty: Type, work: &mut Vec<Type>) {
    match ty {
        Type::Meaning { arguments, .. }
        | Type::Tuple(arguments)
        | Type::Packed(arguments)
        | Type::All(arguments)
        | Type::Sum(arguments) => work.extend(arguments),
        Type::Array { element, length } => {
            work.push(*element);
            work.push(*length);
        }
        Type::Function { parameter, result } => {
            work.push(*parameter);
            work.push(*result);
        }
        Type::Not(inner) => work.push(*inner),
        Type::Unit
        | Type::Parameter(_)
        | Type::Session(_)
        | Type::Number(_) => {}
    }
}
pub(crate) struct Descriptor {
    pub(crate) root: Forest,
    pub(crate) stamps: Vec<Rc<()>>,
}
impl ResourceType {
    /// Find an already materialized, marked, zero-parameter checked type.
    ///
    /// # Errors
    /// Returns a located lookup/classification error; unused declarations may not
    /// yet have a materialized meaning and return `NotFound`.
    pub fn registered(
        tables: &ir::Tables,
        namespace: &str,
        name: &str,
    ) -> Result<Self, ResourceError> {
        let mut found =
            tables.meanings.iter().enumerate().filter(|(_, meaning)| {
                meaning.label == name
                    && tables
                        .namespaces
                        .get(meaning.namespace)
                        .is_some_and(|known| known == namespace)
            });
        let Some((index, meaning)) = found.next() else {
            return Err(ResourceError::NotFound);
        };
        if found.next().is_some() {
            return Err(ResourceError::Ambiguous);
        }
        if !meaning.resource {
            return Err(ResourceError::NotResource);
        }
        if !meaning.parameters.is_empty() {
            return Err(ResourceError::NotMonomorphic);
        }
        Ok(Self(Rc::new(Descriptor {
            root: Forest(alloc::vec![Type::Meaning {
                index: 0,
                arguments: Vec::new()
            }]),
            stamps: alloc::vec![tables.resource_cache.stamp(index)],
        })))
    }
}
impl PartialEq for ResourceType {
    fn eq(&self, other: &Self) -> bool {
        let left_descriptor = &self.0;
        let right_descriptor = &other.0;
        let mut work = alloc::vec![(
            &left_descriptor.root.0[0],
            &right_descriptor.root.0[0]
        )];
        while let Some((left, right)) = work.pop() {
            match (left, right) {
                (
                    Type::Meaning {
                        index: left_index,
                        arguments: left,
                    },
                    Type::Meaning {
                        index: right_index,
                        arguments: right,
                    },
                ) => {
                    if !Rc::ptr_eq(
                        &left_descriptor.stamps[*left_index],
                        &right_descriptor.stamps[*right_index],
                    ) || left.len() != right.len()
                    {
                        return false;
                    }
                    work.extend(left.iter().zip(right));
                }
                (Type::Tuple(left), Type::Tuple(right))
                | (Type::Packed(left), Type::Packed(right))
                | (Type::All(left), Type::All(right))
                | (Type::Sum(left), Type::Sum(right)) => {
                    if left.len() != right.len() {
                        return false;
                    }
                    work.extend(left.iter().zip(right));
                }
                (
                    Type::Array {
                        element: left,
                        length: left_tail,
                    },
                    Type::Array {
                        element: right,
                        length: right_tail,
                    },
                )
                | (
                    Type::Function {
                        parameter: left,
                        result: left_tail,
                    },
                    Type::Function {
                        parameter: right,
                        result: right_tail,
                    },
                ) => {
                    work.push((left, right));
                    work.push((left_tail, right_tail));
                }
                (Type::Not(left), Type::Not(right)) => work.push((left, right)),
                (Type::Unit, Type::Unit) => {}
                (Type::Number(left), Type::Number(right)) if left == right => {}
                _ => return false,
            }
        }
        true
    }
}
impl Eq for ResourceType {}

/// Descriptive equality of type syntax, independent of any private authority.
pub(crate) fn type_equal(a: &Type, b: &Type) -> bool {
    let mut pending = alloc::vec![(a, b)];
    while let Some((a, b)) = pending.pop() {
        match (a, b) {
            (
                Type::Meaning {
                    index: a,
                    arguments: x,
                },
                Type::Meaning {
                    index: b,
                    arguments: y,
                },
            ) => {
                if a != b || x.len() != y.len() {
                    return false;
                }
                pending.extend(x.iter().zip(y));
            }
            (Type::Tuple(x), Type::Tuple(y))
            | (Type::Packed(x), Type::Packed(y))
            | (Type::All(x), Type::All(y))
            | (Type::Sum(x), Type::Sum(y)) => {
                if x.len() != y.len() {
                    return false;
                }
                pending.extend(x.iter().zip(y));
            }
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
                pending.push((a, b));
                pending.push((x, y));
            }
            (Type::Not(a), Type::Not(b)) => pending.push((a, b)),
            (Type::Unit, Type::Unit) => {}
            (Type::Parameter(a), Type::Parameter(b))
            | (Type::Session(a), Type::Session(b))
                if a == b => {}
            (Type::Number(a), Type::Number(b)) if a == b => {}
            _ => return false,
        }
    }
    true
}

/// Borrowed memo traversal, shared by trusted data construction and VM admission.
pub(crate) fn data_only<E>(
    value: &Value,
    pay: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    scan_data(value, pay, false)
}

/// Logical Host data-admission work does not depend on private Pack evidence.
pub(crate) fn logical_data_only<E>(
    value: &Value,
    pay: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<bool, E> {
    scan_data(value, pay, true)
}

fn scan_data<E>(
    value: &Value,
    pay: &mut impl FnMut(u64) -> Result<(), E>,
    logical: bool,
) -> Result<bool, E> {
    let mut pending = alloc::vec![value];
    let mut seen = alloc::collections::BTreeSet::new();
    while let Some(value) = pending.pop() {
        pay(1)?;
        if matches!(value, Value::Resource(_)) {
            return Ok(false);
        }
        let identity = match value {
            Value::Seq(x) => Some((0, Rc::as_ptr(x) as usize)),
            Value::Tuple(x) => Some((1, Rc::as_ptr(x) as usize)),
            Value::Record(x) => Some((2, Rc::as_ptr(x) as usize)),
            Value::Packed(x) => Some((3, Rc::as_ptr(x) as usize)),
            Value::Function(x) => Some((4, Rc::as_ptr(x) as usize)),
            _ => None,
        };
        if identity.is_some_and(|key| !seen.insert(key)) {
            continue;
        }
        match value {
            Value::Seq(x) | Value::Tuple(x) => {
                pay(u64::try_from(x.len()).unwrap_or(u64::MAX))?;
                pending.extend(x.iter());
            }
            Value::Record(x) => {
                pay(u64::try_from(x.fields.len()).unwrap_or(u64::MAX))?;
                pending.extend(&x.fields);
            }
            Value::Packed(x) if logical || x.association().is_some() => {
                pending.push(x.value());
            }
            Value::Function(x) => {
                pay(u64::try_from(x.captures.len() + x.filled.len())
                    .unwrap_or(u64::MAX))?;
                pending.extend(&x.captures);
                pending.extend(&x.filled);
            }
            _ => {}
        }
    }
    Ok(true)
}

#[cfg(test)]
mod logical_data_tests {
    use super::*;
    use crate::{ir::Type, value::Packed};

    fn counted(value: &Value, logical: bool) -> u64 {
        let mut count = 0;
        let mut pay = |units| {
            count += units;
            Ok::<(), core::convert::Infallible>(())
        };
        assert!(scan_data(value, &mut pay, logical).unwrap());
        count
    }

    #[test]
    fn logical_scan_ignores_pack_evidence_but_construction_keeps_its_fast_path()
    {
        let payload = Value::Tuple(Rc::new(alloc::vec![Value::Int(7)]));
        let checked = Value::Packed(Rc::new(Packed::checked(
            Type::Unit,
            Type::Unit,
            payload.clone(),
            alloc::vec![],
        )));
        let descriptive = Value::Packed(Rc::new(
            Packed::from_data(Type::Unit, Type::Unit, &payload).unwrap(),
        ));
        assert_eq!(checked, descriptive);
        assert_eq!(counted(&checked, true), 4);
        assert_eq!(counted(&descriptive, true), 4);
        assert_eq!(counted(&descriptive, false), 1);
        let mut nested = descriptive;
        for _ in 0..96 {
            nested = Value::Packed(Rc::new(
                Packed::from_data(Type::Unit, Type::Unit, &nested).unwrap(),
            ));
        }
        assert_eq!(counted(&nested, false), 1);
        assert_eq!(counted(&nested, true), 100);
    }

    #[test]
    fn logical_scan_keeps_shared_dags_bounded_and_pays_before_pushing() {
        let mut value = Value::Int(7);
        for _ in 0..48 {
            value = Value::Tuple(Rc::new(alloc::vec![value.clone(), value]));
        }
        let whole = counted(&value, true);
        assert_eq!(whole, 193);
        for allowance in [0, whole - 1, whole] {
            let mut left = allowance;
            let result = logical_data_only(&value, &mut |units| {
                if left < units {
                    left = 0;
                    return Err(());
                }
                left -= units;
                Ok(())
            });
            assert_eq!(
                result,
                if allowance == whole {
                    Ok(true)
                } else {
                    Err(())
                }
            );
            assert_eq!(left, 0);
        }
    }
}

/// Owned predicate syntax, always destroyed without recursive tree drops.
#[derive(Default)]
pub(crate) struct Predicates(pub(crate) Vec<crate::ir::Predicate>);
impl fmt::Debug for Predicates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PredicateForest")
    }
}
impl Drop for Predicates {
    fn drop(&mut self) {
        use crate::ir::Predicate;
        while let Some(predicate) = self.0.pop() {
            match predicate {
                Predicate::All(parts) | Predicate::Any(parts) => {
                    self.0.extend(parts);
                }
                Predicate::Function { parameter, result } => {
                    self.0.push(*parameter);
                    self.0.push(*result);
                }
                Predicate::Not(inner) => self.0.push(*inner),
                Predicate::Test { meanings, .. } => drop(Forest(meanings)),
                Predicate::Nonzero { .. } => (),
            }
        }
    }
}
