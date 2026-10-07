use super::{
    End, Error, Known, Module, Origin, Pass, Span, Type, TypeId, TypeKind,
    TypeName, TypeReason,
};
use crate::{
    ast::{Entry, ItemKind, Member},
    token::Magnitude,
};
use alloc::{string::String, vec::Vec};

/// A checked storage width, separate from a value's meaning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub bits: Magnitude,
    pub needed: Magnitude,
}
impl core::fmt::Display for Holding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (bytes, remainder) = self
            .bits
            .div_rem(&Magnitude::from(8))
            .expect("nonzero divisor");
        if remainder.is_zero() {
            write!(f, "held Size<{bytes}>")?;
        } else {
            write!(f, "held Bits<{}>", self.bits)?;
        }
        if self.needed < self.bits {
            write!(f, ", {} of {} bits", self.needed, self.bits)?;
        }
        Ok(())
    }
}
impl Pass<'_> {
    pub(super) fn holdings(&mut self) {
        for entry in &self.module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Type {
                    name,
                    base: Some(base),
                    parameters,
                    ..
                } = &item.kind
                {
                    let ty = Type::named(
                        Origin::This,
                        name.segments.last().map_or("", String::as_str),
                        parameters
                            .iter()
                            .map(|p| {
                                Type::Rigid(
                                    p.name
                                        .segments
                                        .last()
                                        .cloned()
                                        .unwrap_or_default(),
                                )
                            })
                            .collect(),
                    );
                    self.check_holding(&ty, self.module, *base);
                }
            }
        }
        let module = self.module;
        for (index, syntax) in module.types.iter().enumerate() {
            // A whole intersection supplies the range for each written holding.
            if !matches!(syntax.kind, TypeKind::Both(_)) {
                continue;
            }
            if module.entries.iter().any(|entry| matches!(entry, Entry::Item(item) if matches!(item.kind, ItemKind::Type { base: Some(base), .. } if base.0 == index))) { continue; }
            self.check_holding(
                &self.syntax_type(module, &Origin::This, TypeId(index)),
                module,
                TypeId(index),
            );
        }
    }

    fn check_holding(&mut self, ty: &Type, module: &Module, syntax: TypeId) {
        let mut parts = Vec::new();
        holding_parts(module, syntax, &mut parts);
        let needed = self.needed(ty, &mut Vec::new());
        let mut first = None;
        for part in parts {
            let part_ty = self.syntax_bound(
                module,
                &Origin::This,
                part,
                &self.holding_bindings(ty),
            );
            let mut widths = Vec::new();
            self.widths(
                &part_ty,
                module.types[part.0].span,
                &mut Vec::new(),
                &mut widths,
            );
            for (bits, _, _) in widths {
                let message =
                    if first.as_ref().is_some_and(|first| first != &bits) {
                        Some("the holdings disagree")
                    } else if super::modifier(&part_ty)
                        && !has_parameters(ty)
                        && needed.as_ref().is_none_or(|n| n > &bits)
                    {
                        Some("does not fit its holding")
                    } else {
                        None
                    };
                first.get_or_insert(bits);
                if let Some(message) = message {
                    let span = module.types[part.0].span;
                    if !self.errors.iter().any(|e| e.span == span) {
                        self.errors.push(Error {
                            span,
                            reason: TypeReason::Limit {
                                message: String::from(message),
                            },
                        });
                    }
                }
            }
        }
    }

    fn widths(
        &self,
        ty: &Type,
        span: Span,
        visited: &mut Vec<TypeName>,
        out: &mut Vec<(Magnitude, Span, Origin)>,
    ) {
        match ty {
            Type::Limited { name, .. } => self.widths(name, span, visited, out),
            Type::Both(parts) => {
                for part in parts {
                    self.widths(part, span, visited, out);
                }
            }
            Type::Named { name, arguments } => {
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "Bits" | "Size")
                {
                    if let [Type::Number(n)] = arguments.as_slice() {
                        out.push((
                            if name.name == "Size" {
                                n.multiply(&Magnitude::from(8))
                            } else {
                                n.clone()
                            },
                            span,
                            Origin::This,
                        ));
                    }
                    return;
                }
                if visited.contains(name) {
                    return;
                }
                visited.push(name.clone());
                if let Some((module, Some(base), _)) = self.declaration(name) {
                    let base = self.syntax_bound(
                        module,
                        &name.origin,
                        base,
                        &self.holding_bindings(ty),
                    );
                    self.widths(&base, span, visited, out);
                }
                visited.pop();
            }
            _ => (),
        }
    }
    pub(super) fn holding(
        &self,
        ty: &Type,
        visited: &mut Vec<TypeName>,
    ) -> Option<Holding> {
        let mut widths = Vec::new();
        self.widths(ty, Span::default(), &mut Vec::new(), &mut widths);
        if let Some((bits, _, _)) = widths.first() {
            return Some(Holding {
                bits: bits.clone(),
                needed: self.needed(ty, visited)?,
            });
        }
        match ty {
            Type::Tuple(parts) => {
                let bits = parts.iter().try_fold(
                    Magnitude::default(),
                    |sum, part| {
                        Some(sum.add(&self.holding(part, visited)?.bits))
                    },
                )?;
                Some(Holding {
                    needed: bits.clone(),
                    bits,
                })
            }
            Type::Array { element, length } => {
                let element = self.holding(element, visited)?;
                let n = length.number()?;
                Some(Holding {
                    bits: element.bits.multiply(n),
                    needed: element.bits.multiply(n),
                })
            }
            Type::Named { name, .. }
                if name.origin == Origin::Prelude
                    && matches!(name.name.as_str(), "F32" | "F64") =>
            {
                let bits =
                    Magnitude::from(if name.name == "F32" { 32 } else { 64 });
                Some(Holding {
                    needed: bits.clone(),
                    bits,
                })
            }
            _ => None,
        }
    }
    fn needed(
        &self,
        ty: &Type,
        visited: &mut Vec<TypeName>,
    ) -> Option<Magnitude> {
        let mut facts = Known::default();
        self.limit_facts(ty, &mut facts);
        if let Some(range) = facts.range {
            let (End::Finite(lo), End::Finite(hi)) = (range.lo, range.hi)
            else {
                return None;
            };
            let bits = if lo.negative {
                let low =
                    lo.magnitude.checked_sub(&Magnitude::from(1))?.bit_length()
                        + 1;
                let high = if hi.negative {
                    0
                } else {
                    hi.magnitude.bit_length() + 1
                };
                low.max(high)
            } else {
                hi.magnitude.bit_length()
            };
            return Some(Magnitude::from(bits as u64));
        }
        match ty {
            Type::Tuple(parts) => {
                parts.iter().try_fold(Magnitude::default(), |sum, part| {
                    Some(sum.add(&self.holding(part, visited)?.bits))
                })
            }
            Type::Unit => Some(Magnitude::default()),
            Type::Limited { name, .. } => self.needed(name, visited),
            Type::Both(parts) => parts
                .iter()
                .find(|p| !super::modifier(p))
                .and_then(|p| self.needed(p, visited)),
            Type::Array { element, length } => Some(
                self.holding(element, visited)?
                    .bits
                    .multiply(length.number()?),
            ),
            Type::Named { name, .. } => {
                if name.origin == Origin::Prelude {
                    match name.name.as_str() {
                        "F32" => return Some(Magnitude::from(32)),
                        "F64" => return Some(Magnitude::from(64)),
                        "Integer" | "Vec" | "Deque" => return None,
                        _ => (),
                    }
                }
                if visited.contains(name) {
                    return None;
                }
                visited.push(name.clone());
                let (module, base, _) = self.declaration(name)?;
                let members = module.entries.iter().find_map(|e| match e {
                    Entry::Item(item) => match &item.kind {
                        ItemKind::Type {
                            name: n, members, ..
                        } if n.segments.last() == Some(&name.name) => {
                            members.as_ref()
                        }
                        _ => None,
                    },
                    Entry::Line(_) => None,
                });
                let result = if let Some(members) = members {
                    self.parts_needed(module, ty, members, visited)
                } else {
                    base.and_then(|base| {
                        self.needed(
                            &self.syntax_bound(
                                module,
                                &name.origin,
                                base,
                                &self.holding_bindings(ty),
                            ),
                            visited,
                        )
                    })
                };
                visited.pop();
                result
            }
            _ => None,
        }
    }
    fn holding_bindings(&self, ty: &Type) -> Vec<(String, Type)> {
        let Type::Named { name, arguments } = ty else {
            return Vec::new();
        };
        let Some((module, _, _)) = self.declaration(name) else {
            return Vec::new();
        };
        module
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        name: n,
                        parameters,
                        ..
                    } if n.segments.last() == Some(&name.name) => Some(
                        parameters
                            .iter()
                            .zip(arguments)
                            .map(|(p, a)| {
                                (
                                    p.name
                                        .segments
                                        .last()
                                        .cloned()
                                        .unwrap_or_default(),
                                    a.clone(),
                                )
                            })
                            .collect(),
                    ),
                    _ => None,
                },
                Entry::Line(_) => None,
            })
            .unwrap_or_default()
    }

    pub(super) fn instantiated_holding(&mut self, ty: &Type, span: Span) {
        let Type::Named { name, arguments } = ty else {
            return;
        };
        if arguments.is_empty() || has_parameters(ty) {
            return;
        }
        let mut widths = Vec::new();
        self.widths(ty, span, &mut Vec::new(), &mut widths);
        let needed = self.needed(ty, &mut Vec::new());
        if widths.first().is_some_and(|(bits, _, _)| {
            needed.as_ref().is_none_or(|n| n > bits)
                || widths.iter().any(|(other, _, _)| other != bits)
        }) {
            self.errors.push(Error {
                span,
                reason: TypeReason::Limit {
                    message: alloc::format!(
                        "{} does not fit its holding",
                        name.name
                    ),
                },
            });
        }
    }

    fn parts_needed(
        &self,
        module: &Module,
        head: &Type,
        members: &[Member],
        visited: &mut Vec<TypeName>,
    ) -> Option<Magnitude> {
        let Type::Named { name, .. } = head else {
            return None;
        };
        let bindings = self.holding_bindings(head);
        let constructors: Vec<_> = members
            .iter()
            .filter_map(|m| {
                if let Member::Constructor { fields, .. } = m {
                    Some(fields)
                } else {
                    None
                }
            })
            .collect();
        if constructors.is_empty() {
            members
                .iter()
                .try_fold(Magnitude::default(), |sum, member| {
                    if let Member::Field(field) = member {
                        let ty = self.syntax_bound(
                            module,
                            &name.origin,
                            field.ty,
                            &bindings,
                        );
                        let bits = self
                            .holding(&ty, visited)
                            .map(|h| h.bits)
                            .or_else(|| self.needed(&ty, visited))?;
                        Some(sum.add(&bits))
                    } else {
                        Some(sum)
                    }
                })
        } else if constructors.iter().any(|f| !f.is_empty()) {
            None
        } else {
            Some(Magnitude::from(
                Magnitude::from((constructors.len() - 1) as u64).bit_length()
                    as u64,
            ))
        }
    }
}

fn holding_parts(module: &Module, id: TypeId, parts: &mut Vec<TypeId>) {
    if let TypeKind::Both(children) = &module.types[id.0].kind {
        for child in children {
            holding_parts(module, *child, parts);
        }
    } else {
        parts.push(id);
    }
}

fn has_parameters(ty: &Type) -> bool {
    match ty {
        Type::Var(_) | Type::Rigid(_) => true,
        Type::Named { arguments, .. }
        | Type::Sum(arguments)
        | Type::Tuple(arguments)
        | Type::Both(arguments)
        | Type::Either(arguments) => arguments.iter().any(has_parameters),
        Type::Array { element, length } => {
            has_parameters(element) || has_parameters(length)
        }
        Type::Limited { name, meaning } => {
            has_parameters(name) || has_parameters(meaning)
        }
        Type::Function { parameter, result } => {
            has_parameters(parameter) || has_parameters(result)
        }
        Type::Not(inner) => has_parameters(inner),
        Type::Unit | Type::Number(_) => false,
    }
}
