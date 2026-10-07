use super::{signature, Error, Kind, Reason};
use crate::ast::{
    Entry, ExprKind, ImplMember, ItemKind, Module, TypeId, TypeKind,
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

/// Declaration-time spot rules, shared with program checking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Duplicate(String),
    AmbiguousFlag(char),
    SelfNotLast,
}

pub fn check(module: &Module) -> Vec<Error> {
    let mut errors = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Declare { ty, .. } => {
                signature_spots(module, *ty, false, &mut errors);
            }
            ItemKind::Impl { members, .. } => {
                for member in members {
                    if let ImplMember::Named {
                        annotation: Some(ty),
                        value,
                        ..
                    } = member
                    {
                        signature_spots(
                            module,
                            *ty,
                            value.is_none(),
                            &mut errors,
                        );
                    }
                }
            }
            ItemKind::Type {
                members: Some(members),
                ..
            } => {
                let fields: Vec<_> = members
                    .iter()
                    .filter_map(|member| match member {
                        crate::ast::Member::Field(field) => Some(field),
                        crate::ast::Member::Constructor { fields, .. } => {
                            errors
                                .extend(validate_fields(module, fields.iter()));
                            None
                        }
                    })
                    .collect();
                errors.extend(validate_fields(module, fields.into_iter()));
            }
            _ => {}
        }
    }
    for expression in &module.expressions {
        if let ExprKind::Bind {
            annotation: Some(ty),
            ..
        } = expression.kind
        {
            signature_spots(module, ty, false, &mut errors);
        }
    }
    errors
}

fn validate_fields<'a>(
    module: &Module,
    fields: impl Iterator<Item = &'a crate::ast::Field>,
) -> Vec<Error> {
    let spots: Vec<_> = fields
        .map(|field| {
            (
                field.name.segments.last().cloned(),
                is_bool(module, field.ty, 0),
                field.name.span,
            )
        })
        .collect();
    validate(&spots, false)
}

fn signature_spots(
    module: &Module,
    ty: TypeId,
    supplied: bool,
    errors: &mut Vec<Error>,
) {
    let Kind::Function { spots, .. } = signature(module, ty) else {
        return;
    };
    let spots: Vec<_> = spots
        .iter()
        .map(|spot| (spot.name.clone(), is_bool(module, spot.ty, 0), spot.span))
        .collect();
    errors.extend(validate(&spots, supplied));
}

pub fn validate(
    spots: &[(Option<String>, bool, crate::token::Span)],
    supplied: bool,
) -> Vec<Error> {
    let mut errors = Vec::new();
    let mut names = BTreeMap::new();
    let mut letters = BTreeMap::new();
    for (index, (name, is_bool, span)) in spots.iter().enumerate() {
        let Some(name) = name else { continue };
        let problem = if names.insert(name, index).is_some() {
            Some(Problem::Duplicate(name.clone()))
        } else if supplied && name == "self" && index + 1 != spots.len() {
            Some(Problem::SelfNotLast)
        } else if *is_bool {
            name.chars().next().and_then(|letter| {
                letters
                    .insert(letter, index)
                    .map(|_| Problem::AmbiguousFlag(letter))
            })
        } else {
            None
        };
        if let Some(problem) = problem {
            errors.push(Error {
                span: *span,
                reason: Reason::Spots { problem },
            });
        }
    }
    errors
}

fn is_bool(module: &Module, ty: TypeId, depth: usize) -> bool {
    if depth > crate::parse::MAX_NESTING {
        return false;
    }
    match &module.types[ty.0].kind {
        TypeKind::Named { name, .. } => {
            if name.segments.len() != 1 {
                return false;
            }
            let own = module.entries.iter().find_map(|entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        name: declared,
                        base,
                        ..
                    } if declared.segments == name.segments => Some(*base),
                    _ => None,
                },
                Entry::Line(_) => None,
            });
            own.map_or(name.segments[0] == "Bool", |base| {
                base.is_some_and(|base| is_bool(module, base, depth + 1))
            })
        }
        TypeKind::Labeled { ty, .. } => is_bool(module, *ty, depth + 1),
        TypeKind::Both(parts) => {
            parts.iter().any(|ty| is_bool(module, *ty, depth + 1))
        }
        _ => false,
    }
}
