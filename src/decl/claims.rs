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
        let context = owner(module, *target);
        let key = (
            context.identity.clone(),
            contextual_key(module, *claim, &context.names),
        );
        check_listed(module, others, &context, &key.1, item.span, errors);
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
            if key.0 != owner(module, *target).identity {
                continue;
            }
            if members.iter().any(|m| matches!(m,ImplMember::Named { name, .. } if required.contains(name.segments.last().unwrap()))) {
                error(errors,item.span,"member occurs in a plain impl and a claim");
            }
        }
    }
}

fn check_listed(
    module: &Module,
    others: &[(&str, &Module)],
    context: &Owner,
    claim: &str,
    span: Span,
    errors: &mut Vec<Error>,
) {
    for declaration in &module.entries {
        let Entry::Item(declaration) = declaration else {
            continue;
        };
        if let ItemKind::Type {
            name,
            base,
            parameters,
            members: Some(_),
            ..
        } = &declaration.kind
        {
            if name.segments.join(".") == context.path
                && declaration_names(parameters, context).is_some_and(|names| {
                    !base.is_some_and(|base| {
                        listed_in(
                            module,
                            base,
                            claim,
                            &names,
                            others,
                            &mut Vec::new(),
                        )
                    })
                })
            {
                error(errors, span, "claim is not listed on the type");
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
    let context = owner(module, target);
    let wanted = contextual_key(module, claim, &context.names);
    module.entries.iter().filter_map(|entry|{
        let Entry::Item(crate::ast::Item{kind:ItemKind::Impl{target,claim:Some(other),members},..})=entry else{return None;};
        let other_context=owner(module,*target);
        (context.identity==other_context.identity&&(
            listed_in(module,claim,&contextual_key(module,*other,&other_context.names),&context.names,others,&mut Vec::new())
            ||listed_in(module,*other,&wanted,&other_context.names,others,&mut Vec::new())
        )).then_some(members)
    }).flatten().filter(|m|matches!(m,ImplMember::Named{name:n,..}if n.segments.last().is_some_and(|n|n==name))).count()
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
            parameters,
            members: Some(_),
            ..
        } = &item.kind
        else {
            continue;
        };
        let declaration = declared_owner(name, parameters);
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
            let wanted = contextual_key(module, part, &declaration.names);
            let present = module.entries.iter().any(|e| match e {
                Entry::Item(crate::ast::Item {
                    kind:
                        ItemKind::Impl {
                            target,
                            claim: Some(claim),
                            ..
                        },
                    ..
                }) if owner(module, *target).path == declaration.path
                    && owner(module, *target).blanket =>
                {
                    let context = owner(module, *target);
                    listed_in(
                        module,
                        *claim,
                        &wanted,
                        &context.names,
                        others,
                        &mut Vec::new(),
                    )
                }
                _ => false,
            });
            if !present && !names.iter().all(|n| {
                derived_member(module, part, n, others) || module.entries.iter().any(|e| match e {
                    Entry::Item(crate::ast::Item { kind: ItemKind::Impl { target, claim: Some(claim), members }, .. })
                        if owner(module,*target).path==declaration.path && owner(module,*target).blanket
                            && listed_in(module,part,&contextual_key(module,*claim,&owner(module,*target).names),&declaration.names,others,&mut Vec::new()) =>
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

// Compare owners in the same implicit binder scope used by walk_impl.
struct Owner {
    path: String,
    identity: String,
    names: Vec<(String, String)>,
    actual: Vec<String>,
    blanket: bool,
}
fn direct_binder(module: &Module, id: TypeId) -> Option<String> {
    match &module.types[id.0].kind {
        TypeKind::Named { name, arguments }
            if arguments.is_empty()
                && name.segments.len() == 1
                && name.segments[0] != "Self" =>
        {
            Some(name.segments[0].clone())
        }
        _ => None,
    }
}
fn owner(module: &Module, id: TypeId) -> Owner {
    let TypeKind::Named { name, arguments } = &module.types[id.0].kind else {
        return Owner {
            path: String::new(),
            identity: key(module, id),
            names: Vec::new(),
            actual: Vec::new(),
            blanket: false,
        };
    };
    let parameters = module.entries.iter().find_map(|entry| match entry {
        Entry::Item(crate::ast::Item {
            kind:
                ItemKind::Type {
                    name: n,
                    parameters,
                    ..
                },
            ..
        }) if n.segments == name.segments => Some(parameters.as_slice()),
        _ => None,
    });
    let mut names = Vec::new();
    for argument in arguments {
        if let Some(name) = direct_binder(module, *argument) {
            if !names.iter().any(|(known, _)| known == &name) {
                let position = names.len();
                names.push((name, alloc::format!("?{position}")));
            }
        }
    }
    let implicit =
        arguments.is_empty() && parameters.is_some_and(|p| !p.is_empty());
    let actual = if implicit {
        parameters
            .unwrap()
            .iter()
            .enumerate()
            .map(|(position, p)| {
                let value = alloc::format!("?{position}");
                names.push((p.name.segments.join("."), value.clone()));
                value
            })
            .collect()
    } else {
        arguments
            .iter()
            .map(|id| contextual_key(module, *id, &names))
            .collect::<Vec<_>>()
    };
    let path = name.segments.join(".");
    let identity = if actual.is_empty() {
        path.clone()
    } else {
        alloc::format!("{path}<{}>", actual.join(","))
    };
    let blanket = parameters.is_some_and(|p| {
        implicit
            || (arguments.len() == p.len()
                && names.len() == p.len()
                && arguments
                    .iter()
                    .all(|id| direct_binder(module, *id).is_some()))
    });
    names.push((String::from("Self"), identity.clone()));
    Owner {
        path,
        identity,
        names,
        actual,
        blanket,
    }
}
fn contextual_key(
    module: &Module,
    id: TypeId,
    names: &[(String, String)],
) -> String {
    match &module.types[id.0].kind {
        TypeKind::Named { name, arguments } => {
            if arguments.is_empty() && name.segments.len() == 1 {
                if let Some((_, value)) =
                    names.iter().find(|(n, _)| n == &name.segments[0])
                {
                    return value.clone();
                }
            }
            let name = name.segments.join(".");
            let arguments = arguments
                .iter()
                .map(|id| contextual_key(module, *id, names))
                .collect::<Vec<_>>();
            if arguments.is_empty() {
                name
            } else {
                alloc::format!("{name}<{}>", arguments.join(","))
            }
        }
        TypeKind::Array { element, length } => alloc::format!(
            "[{};{}]",
            contextual_key(module, *element, names),
            contextual_key(module, *length, names)
        ),
        TypeKind::Number(n) => alloc::format!("{n}"),
        TypeKind::Unit => String::from("()"),
        TypeKind::Tuple(parts)
        | TypeKind::Both(parts)
        | TypeKind::Either(parts)
        | TypeKind::Sum(parts) => {
            let separator = match &module.types[id.0].kind {
                TypeKind::Tuple(_) => "*",
                TypeKind::Both(_) => "&",
                TypeKind::Either(_) => "|",
                TypeKind::Sum(_) => "+",
                _ => unreachable!(),
            };
            let parts = parts
                .iter()
                .map(|part| contextual_key(module, *part, names))
                .collect::<Vec<_>>();
            alloc::format!("({})", parts.join(separator))
        }
        TypeKind::Function { parameter, result } => alloc::format!(
            "({}->{})",
            contextual_key(module, *parameter, names),
            contextual_key(module, *result, names)
        ),
        TypeKind::Not(inner) => {
            alloc::format!("!({})", contextual_key(module, *inner, names))
        }
        // These forms retain obligations/defaults elsewhere; their underlying
        // type is the identity used by inference's lower() as well.
        TypeKind::Qualified { ty, .. } | TypeKind::Labeled { ty, .. } => {
            contextual_key(module, *ty, names)
        }
    }
}
fn declaration_names(
    parameters: &[crate::ast::TypeParameter],
    owner: &Owner,
) -> Option<Vec<(String, String)>> {
    if parameters.len() != owner.actual.len() {
        return None;
    }
    let mut names = parameters
        .iter()
        .zip(&owner.actual)
        .map(|(p, value)| (p.name.segments.join("."), value.clone()))
        .collect::<Vec<_>>();
    names.push((String::from("Self"), owner.identity.clone()));
    Some(names)
}
fn declared_owner(
    name: &crate::ast::Name,
    parameters: &[crate::ast::TypeParameter],
) -> Owner {
    let path = name.segments.join(".");
    let actual: Vec<_> = (0..parameters.len())
        .map(|i| alloc::format!("?{i}"))
        .collect();
    let identity = if parameters.is_empty() {
        path.clone()
    } else {
        alloc::format!("{path}<{}>", actual.join(","))
    };
    let mut result = Owner {
        path,
        identity,
        names: Vec::new(),
        actual,
        blanket: true,
    };
    result.names = declaration_names(parameters, &result).unwrap();
    result
}

// Class identity, not an expanding instantiated key, bounds invalid cycles.
fn listed_in(
    module: &Module,
    id: TypeId,
    wanted: &str,
    names: &[(String, String)],
    others: &[(&str, &Module)],
    visiting: &mut Vec<(*const Module, Span)>,
) -> bool {
    if contextual_key(module, id, names) == wanted {
        return true;
    }
    match &module.types[id.0].kind {
        TypeKind::Both(parts) => parts
            .iter()
            .any(|p| listed_in(module, *p, wanted, names, others, visiting)),
        TypeKind::Named { name, arguments } => core::iter::once(module)
            .chain(others.iter().map(|(_, m)| *m))
            .any(|m| {
                m.entries.iter().any(|entry| {
                    let Entry::Item(item) = entry else {
                        return false;
                    };
                    let ItemKind::Type {
                        name: n,
                        parameters,
                        base: Some(base),
                        members: None,
                        ..
                    } = &item.kind
                    else {
                        return false;
                    };
                    if n.segments.last() != name.segments.last() {
                        return false;
                    }
                    let identity = (core::ptr::from_ref(m), item.span);
                    if visiting.contains(&identity) {
                        return false;
                    }
                    // Do not capture an identically spelled owner formal in this class.
                    let mut inherited: Vec<_> = parameters
                        .iter()
                        .zip(arguments)
                        .map(|(p, id)| {
                            (
                                p.name.segments.join("."),
                                contextual_key(module, *id, names),
                            )
                        })
                        .collect();
                    if let Some((_, value)) =
                        names.iter().find(|(name, _)| name == "Self")
                    {
                        inherited.push((String::from("Self"), value.clone()));
                    }
                    visiting.push(identity);
                    let found = listed_in(
                        m, *base, wanted, &inherited, others, visiting,
                    );
                    visiting.pop();
                    found
                })
            }),
        _ => false,
    }
}
