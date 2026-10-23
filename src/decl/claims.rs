//! Declaration blocks must supply exactly the named limit's members.
use super::{Error, Reason};
use crate::{
    ast::{Entry, ImplMember, ItemKind, Limit, Module, TypeId, TypeKind},
    token::Span,
};
use alloc::{string::String, vec::Vec};

pub(super) fn check(
    module: &Module,
    others: &[(&str, &Module)],
    errors: &mut Vec<Error>,
) {
    check_missing(module, others, errors);
    let mut blocks = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Impl {
            target,
            claim: Some(claim),
            members,
        } = &item.kind
        else {
            continue;
        };
        let Some(shaped) = target_shape(module, *target, others) else {
            error(errors, module.types[target.0].span, "unknown impl target");
            continue;
        };
        let mut required = Vec::new();
        if !asked(module, *claim, others, &mut Vec::new(), &mut required)
            || required.is_empty()
        {
            error(errors, item.span, "unknown member limit");
            continue;
        }
        let key = (key(module, *target), key(module, *claim));
        for declaration in &module.entries {
            let Entry::Item(declaration) = declaration else {
                continue;
            };
            if let ItemKind::Type {
                name,
                base,
                members: Some(_),
                ..
            } = &declaration.kind
            {
                if name.segments.join(".") == key.0
                    && !base.is_some_and(|base| {
                        listed(module, base, &key.1, others, &mut Vec::new())
                    })
                {
                    error(errors, item.span, "claim is not listed on the type");
                }
            }
        }
        if blocks.contains(&key) {
            error(errors, item.span, "second block for this claim");
        }
        blocks.push(key.clone());
        let mut supplied = Vec::new();
        for member in members {
            let ImplMember::Named { name, .. } = member else {
                error(
                    errors,
                    item.span,
                    "a claim does not ask for a constructor",
                );
                continue;
            };
            let name = name.segments.last().unwrap().clone();
            if !required.contains(&name) {
                error(errors, item.span, "member not asked by the claim");
            }
            if supplied.contains(&name) {
                error(errors, item.span, "member supplied twice");
            }
            supplied.push(name);
        }
        for name in &required {
            let count = member_count(module, *target, *claim, name, others);
            if count > 1 {
                error(errors, item.span, "member supplied twice");
            } else if count == 0
                && !(shaped && derived_member(module, *claim, name, others))
            {
                error(
                    errors,
                    item.span,
                    &alloc::format!("missing claim member {name}"),
                );
            }
        }
        for other in &module.entries {
            let Entry::Item(other) = other else { continue };
            let ItemKind::Impl {
                target,
                claim: None,
                members,
            } = &other.kind
            else {
                continue;
            };
            if key.0 != self::key(module, *target) {
                continue;
            }
            if members.iter().any(|m| matches!(m,ImplMember::Named { name, .. } if required.contains(name.segments.last().unwrap()))) {
                error(errors,item.span,"member occurs in a plain impl and a claim");
            }
        }
    }
}

fn member_count(
    module: &Module,
    target: TypeId,
    claim: TypeId,
    name: &str,
    others: &[(&str, &Module)],
) -> usize {
    module.entries.iter().filter_map(|e| match e {
        Entry::Item(crate::ast::Item { kind: ItemKind::Impl { target: t, claim: Some(c), members }, .. })
            if key(module, *t) == key(module, target)
                && [ (claim, *c), (*c, claim) ].iter().any(|(whole, part)| listed(module, *whole, &key(module, *part), others, &mut Vec::new())) => Some(members),
        _ => None,
    }).flatten().filter(|m| matches!(m, ImplMember::Named { name: n, .. } if n.segments.last().is_some_and(|n| n == name))).count()
}

fn derived_member(
    module: &Module,
    claim: TypeId,
    name: &str,
    others: &[(&str, &Module)],
) -> bool {
    let class = match name {
        "eq" => "Eq",
        "cmp" => "Ord",
        "show" => "Show",
        "default" => "Default",
        _ => return false,
    };
    listed(module, claim, class, others, &mut Vec::new())
}

fn error(errors: &mut Vec<Error>, span: Span, problem: &str) {
    errors.push(Error {
        span,
        reason: Reason::ClaimBlock {
            problem: String::from(problem),
        },
    });
}

fn key(module: &Module, id: TypeId) -> String {
    match &module.types[id.0].kind {
        TypeKind::Named { name, arguments } => {
            let mut text = name.segments.join(".");
            if !arguments.is_empty() {
                text.push('<');
                text.push_str(
                    &arguments
                        .iter()
                        .map(|a| key(module, *a))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                text.push('>');
            }
            text
        }
        TypeKind::Array { element, length } => alloc::format!(
            "[{};{}]",
            key(module, *element),
            key(module, *length)
        ),
        TypeKind::Number(n) => alloc::format!("{n}"),
        _ => String::new(),
    }
}

fn asked(
    module: &Module,
    id: TypeId,
    others: &[(&str, &Module)],
    visiting: &mut Vec<String>,
    names: &mut Vec<String>,
) -> bool {
    if let TypeKind::Both(parts) = &module.types[id.0].kind {
        return parts
            .iter()
            .all(|p| asked(module, *p, others, visiting, names));
    }
    let TypeKind::Named { name, .. } = &module.types[id.0].kind else {
        return false;
    };
    let text = name.segments.join(".");
    if visiting.contains(&text) {
        return false;
    }
    visiting.push(text);
    let declaration = core::iter::once(("", module))
        .chain(others.iter().copied())
        .filter(|(prefix, _)| {
            name.segments.len() == 1
                || *prefix == name.segments[..name.segments.len() - 1].join(".")
        })
        .map(|(_, m)| m)
        .find_map(|m| {
            m.entries.iter().find_map(|e| match e {
                Entry::Item(crate::ast::Item {
                    kind:
                        ItemKind::Type {
                            name: n,
                            base,
                            limits,
                            members: None,
                            ..
                        },
                    ..
                }) if n.segments.last() == name.segments.last() => {
                    Some((m, base, limits))
                }
                _ => None,
            })
        });
    let result = declaration.is_some_and(|(m, base, limits)| {
        let bases = base.is_none_or(|b| asked(m, b, others, visiting, names));
        let mut valid = bases;
        for limit in limits {
            if let Limit::Member { name, .. } = limit {
                let name = name.segments.last().unwrap().clone();
                if !names.contains(&name) {
                    names.push(name);
                }
            } else {
                valid = false;
            }
        }
        valid
    });
    visiting.pop();
    result
}

fn listed(
    module: &Module,
    id: TypeId,
    wanted: &str,
    others: &[(&str, &Module)],
    visiting: &mut Vec<String>,
) -> bool {
    let text = key(module, id);
    if text == wanted {
        return true;
    }
    match &module.types[id.0].kind {
        TypeKind::Both(parts) => parts
            .iter()
            .any(|p| listed(module, *p, wanted, others, visiting)),
        TypeKind::Named { name, .. } => {
            if visiting.contains(&text) {
                return false;
            }
            visiting.push(text);
            let result = core::iter::once(module)
                .chain(others.iter().map(|(_, m)| *m))
                .any(|m| {
                    m.entries.iter().any(|e| match e {
                        Entry::Item(crate::ast::Item {
                            kind:
                                ItemKind::Type {
                                    name: n,
                                    base: Some(base),
                                    members: None,
                                    ..
                                },
                            ..
                        }) if n.segments.last() == name.segments.last() => {
                            listed(m, *base, wanted, others, visiting)
                        }
                        _ => false,
                    })
                });
            visiting.pop();
            result
        }
        _ => false,
    }
}

fn check_missing(
    module: &Module,
    others: &[(&str, &Module)],
    errors: &mut Vec<Error>,
) {
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Type {
            name,
            base: Some(base),
            members: Some(_),
            ..
        } = &item.kind
        else {
            continue;
        };
        let parts = match &module.types[base.0].kind {
            TypeKind::Both(parts) => parts.clone(),
            _ => alloc::vec![*base],
        };
        for part in parts {
            let mut names = Vec::new();
            if !asked(module, part, others, &mut Vec::new(), &mut names)
                || names.is_empty()
            {
                continue;
            }
            if derives(module, part, others, &mut Vec::new()) {
                continue;
            }
            let wanted = key(module, part);
            let present = module.entries.iter().any(|e| match e {
                Entry::Item(crate::ast::Item {
                    kind:
                        ItemKind::Impl {
                            target,
                            claim: Some(claim),
                            ..
                        },
                    ..
                }) if key(module, *target) == name.segments.join(".") => {
                    listed(module, *claim, &wanted, others, &mut Vec::new())
                }
                _ => false,
            });
            if !present && !names.iter().all(|n| {
                derived_member(module, part, n, others) || module.entries.iter().any(|e| match e {
                    Entry::Item(crate::ast::Item { kind: ItemKind::Impl { target, claim: Some(claim), members }, .. })
                        if key(module, *target) == name.segments.join(".")
                            && listed(module, part, &key(module, *claim), others, &mut Vec::new()) =>
                        members.iter().any(|m| matches!(m, ImplMember::Named { name, .. } if name.segments.last() == Some(n))),
                    _ => false,
                })
            }) {
                error(
                    errors,
                    module.types[part.0].span,
                    "listed claim has no block or derived member",
                );
            }
        }
    }
}

fn derives(
    module: &Module,
    id: TypeId,
    others: &[(&str, &Module)],
    visiting: &mut Vec<String>,
) -> bool {
    match &module.types[id.0].kind {
        TypeKind::Both(parts) => {
            parts.iter().all(|p| derives(module, *p, others, visiting))
        }
        TypeKind::Named { name, .. } => {
            let text = key(module, id);
            if visiting.contains(&text) {
                return false;
            }
            visiting.push(text);
            let own = module.entries.iter().find_map(|e| match e {
                Entry::Item(crate::ast::Item {
                    kind:
                        ItemKind::Type {
                            name: n,
                            base,
                            limits,
                            ..
                        },
                    ..
                }) if n.segments == name.segments => Some((base, limits)),
                _ => None,
            });
            let builtin = others.is_empty() || own.is_none();
            let result = (builtin
                && name.segments.len() == 1
                && matches!(
                    name.segments[0].as_str(),
                    "Eq" | "Ord" | "Show" | "Default"
                ))
                || own.is_some_and(|(base, limits)| {
                    limits.is_empty()
                        && base.is_some_and(|base| {
                            derives(module, base, others, visiting)
                        })
                });
            visiting.pop();
            result
        }
        _ => false,
    }
}

fn target_shape(
    module: &Module,
    target: TypeId,
    others: &[(&str, &Module)],
) -> Option<bool> {
    if matches!(module.types[target.0].kind, TypeKind::Array { .. }) {
        return Some(false);
    }
    let TypeKind::Named { name, .. } = &module.types[target.0].kind else {
        return None;
    };
    core::iter::once(module)
        .chain(
            others
                .iter()
                .filter(|(n, _)| {
                    name.segments.len() == 1
                        || *n
                            == name.segments[..name.segments.len() - 1]
                                .join(".")
                })
                .map(|(_, m)| *m),
        )
        .find_map(|m| {
            m.entries.iter().find_map(|e| match e {
                Entry::Item(crate::ast::Item {
                    kind:
                        ItemKind::Type {
                            name: n, members, ..
                        },
                    ..
                }) if n.segments.last() == name.segments.last() => {
                    Some(members.is_some())
                }
                _ => None,
            })
        })
}
