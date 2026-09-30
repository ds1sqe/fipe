use super::{signature, Error, Kind, Reason};
use crate::{
    ast::{Entry, ImplMember, ItemKind, Module, TypeId},
    token::Span,
};
use alloc::{string::String, vec::Vec};

pub(super) fn check(module: &Module, errors: &mut Vec<Error>) {
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Declare { name, ty } => {
                spots(module, item.doc.as_deref(), *ty, name.span, errors);
            }
            ItemKind::Impl { members, .. } => {
                for member in members {
                    if let ImplMember::Named {
                        doc,
                        name,
                        annotation: Some(ty),
                        value: None,
                        ..
                    } = member
                    {
                        spots(module, doc.as_deref(), *ty, name.span, errors);
                    }
                }
            }
            ItemKind::Type { .. } | ItemKind::Use { .. } => {}
        }
    }
}

fn spots(
    module: &Module,
    doc: Option<&str>,
    ty: TypeId,
    span: Span,
    errors: &mut Vec<Error>,
) {
    let Some(doc) = doc else { return };
    let kind = signature(module, ty);
    for name in doc.lines().filter_map(spot_head) {
        let exists = matches!(&kind, Kind::Function { spots, .. }
            if spots.iter().any(|spot| spot.name.as_deref() == Some(name)));
        if !exists {
            errors.push(Error {
                span,
                reason: Reason::UnknownDocSpot {
                    name: String::from(name),
                },
            });
        }
    }
}

/// Only the ruled Markdown head is meaningful; the rest remains opaque.
fn spot_head(line: &str) -> Option<&str> {
    let (name, _) = line.trim_start().strip_prefix("- ")?.split_once(':')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    ((first.is_ascii_lowercase() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
    .then_some(name)
}
