use super::{Report, Session};
use crate::{
    ast::{
        Entry, ExprId, ExprKind, ImplMember, ItemKind, Member, Module,
        PatternKind, TypeKind,
    },
    resolve::{Origin, Target},
    token::Span,
};

impl Session {
    /// Documentation on a binding or resolved name in this session's report.
    /// Reads only the owner's syntax; documentation has no typing effect.
    #[must_use]
    pub fn documentation<'a>(
        &'a self,
        report: &'a Report,
        id: ExprId,
    ) -> Option<&'a str> {
        let module = report.module.as_ref()?;
        if let ExprKind::Bind { doc, .. } = &module.expr(id)?.kind {
            return doc.as_deref();
        }
        let target = report.resolved.as_ref()?.expr(id)?;
        self.target_doc(module, target)
    }

    fn doc_module<'a>(
        &'a self,
        current: &'a Module,
        origin: &Origin,
    ) -> Option<&'a Module> {
        match origin {
            Origin::This => Some(current),
            Origin::Prelude => Some(&self.declarations.prelude.module),
            Origin::Commands => {
                self.declarations.commands.as_ref().map(|ns| &ns.module)
            }
            Origin::Host(name) => {
                self.declarations.get_module(name).map(|ns| &ns.module)
            }
            Origin::File(name) => self
                .files
                .iter()
                .find(|(stem, _)| stem == name)
                .map(|(_, module)| module),
            Origin::Line(_) => self.frame.module(origin),
        }
    }

    fn target_doc<'a>(
        &'a self,
        current: &'a Module,
        target: &Target,
    ) -> Option<&'a str> {
        match target {
            Target::Local { site } => local_doc(current, *site).or_else(|| {
                self.frame
                    .lines
                    .iter()
                    .find_map(|line| local_doc(&line.module, *site))
            }),
            Target::Module(origin) => {
                self.doc_module(current, origin)?.doc.as_deref()
            }
            Target::Value { origin, name } | Target::Type { origin, name } => {
                named_doc(self.doc_module(current, origin)?, name)
            }
            Target::Constructor { origin, ty, name } => {
                let module = self.doc_module(current, origin)?;
                for entry in &module.entries {
                    if let Entry::Item(item) = entry {
                        if let ItemKind::Type {
                            name: owner,
                            members: Some(members),
                            ..
                        } = &item.kind
                        {
                            if owner.segments.last()? != ty {
                                continue;
                            }
                            if name == ty {
                                return item.doc.as_deref();
                            }
                            for member in members {
                                if let Member::Constructor {
                                    name: owner,
                                    doc,
                                    ..
                                } = member
                                {
                                    if owner.segments.last()? == name {
                                        return doc.as_deref();
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            Target::TypeMember { ty, member } => {
                let Target::Type { origin, name } = ty.as_ref() else {
                    return None;
                };
                let module = self.doc_module(current, origin)?;
                if let Origin::Line(index) = origin {
                    let owner = (*index, name.clone());
                    if let Some(lines) = self.frame.impls.get(&owner) {
                        return lines.iter().rev().find_map(|index| {
                            member_doc(
                                &self.frame.lines[*index].module,
                                name,
                                member,
                            )
                        });
                    }
                }
                member_doc(module, name, member)
            }
            _ => None,
        }
    }
}

fn local_doc(module: &Module, site: Span) -> Option<&str> {
    module.expressions.iter().find_map(|expr| {
        let ExprKind::Bind { doc, pattern, .. } = &expr.kind else {
            return None;
        };
        let span = module.pattern(*pattern)?.span;
        (span.start <= site.start && site.end <= span.end)
            .then_some(doc.as_deref())
            .flatten()
    })
}

fn named_doc<'a>(module: &'a Module, name: &str) -> Option<&'a str> {
    module
        .entries
        .iter()
        .rev()
        .find_map(|entry| {
            let (owner, doc) = match entry {
                Entry::Item(item) => {
                    let (ItemKind::Type { name: owner, .. }
                    | ItemKind::Declare { name: owner, .. }) = &item.kind
                    else {
                        return None;
                    };
                    (owner, &item.doc)
                }
                Entry::Line(id) => {
                    let ExprKind::Bind { doc, pattern, .. } =
                        &module.expr(*id)?.kind
                    else {
                        return None;
                    };
                    let PatternKind::Name(owner) =
                        &module.pattern(*pattern)?.kind
                    else {
                        return None;
                    };
                    (owner, doc)
                }
            };
            owner
                .segments
                .last()
                .is_some_and(|part| part == name)
                .then_some(doc.as_deref())
        })
        .flatten()
}

fn member_doc<'a>(
    module: &'a Module,
    name: &str,
    member: &str,
) -> Option<&'a str> {
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Impl {
            target, members, ..
        } = &item.kind
        else {
            continue;
        };
        let TypeKind::Named { name: owner, .. } = &module.ty(*target)?.kind
        else {
            continue;
        };
        if owner.segments.last().is_none_or(|part| part != name) {
            continue;
        }
        for entry in members {
            if let ImplMember::Named { name, doc, .. } = entry {
                if name.segments.last().is_some_and(|part| part == member) {
                    return doc.as_deref();
                }
            }
        }
    }
    None
}
