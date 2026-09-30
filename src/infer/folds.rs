//! Exact named spellings of a scheme's member obligations.
use super::{
    lower::{SelfKind, Vars},
    Infer,
};
use crate::{
    ast::{Entry, ItemKind, Name, TypeParameter},
    resolve::Origin,
    token::Span,
    types::{Obligation, Scheme, Type},
};
use alloc::{vec, vec::Vec};

impl Infer<'_> {
    pub(super) fn fold_names(
        &mut self,
        scheme: &Scheme,
    ) -> Vec<(Type, Vec<Obligation>)> {
        if scheme.obligations.is_empty() {
            return Vec::new();
        }
        let mut origins = vec![Origin::Prelude, Origin::This, Origin::Commands];
        origins.extend(
            self.declarations
                .modules
                .iter()
                .map(|m| Origin::Host(m.name.clone())),
        );
        origins.extend(
            self.files
                .iter()
                .map(|(name, _)| Origin::File(name.clone())),
        );
        if let Some(frame) = self.frame {
            origins.extend((0..frame.lines.len()).map(Origin::Line));
        }
        let mut heads = Vec::new();
        for obligation in &scheme.obligations {
            if !heads.contains(obligation.subject()) {
                heads.push(obligation.subject().clone());
            }
        }
        let mut folds = Vec::new();
        for origin in origins {
            let Some(module) = self.owner(&origin) else {
                continue;
            };
            for entry in &module.entries {
                let Entry::Item(item) = entry else { continue };
                let ItemKind::Type {
                    name,
                    parameters,
                    members: None,
                    ..
                } = &item.kind
                else {
                    continue;
                };
                for head in &heads {
                    if let Some(fold) =
                        self.fold_name(scheme, head, &origin, name, parameters)
                    {
                        folds.push(fold);
                    }
                }
            }
        }
        folds.sort_by_key(|(_, members)| core::cmp::Reverse(members.len()));
        folds
    }
    fn fold_name(
        &mut self,
        scheme: &Scheme,
        head: &Type,
        origin: &Origin,
        name: &Name,
        parameters: &[TypeParameter],
    ) -> Option<(Type, Vec<Obligation>)> {
        let before = self.state.clone();
        let scope = self.scope(origin)?;
        let mut folded = None;
        let arguments: Vec<_> =
            parameters.iter().map(|_| self.fresh()).collect();
        let named =
            Type::named(origin.clone(), &super::last(name), arguments.clone());
        let members =
            self.member_limits(&named, head, Span::default(), &mut Vec::new());
        let matches = !members.is_empty()
            && members.iter().all(|member| {
                let Obligation::Member { name, ty, .. } = member;
                scheme.obligations.iter().any(|other| {
                    let Obligation::Member {
                        head: h,
                        name: n,
                        ty: t,
                        ..
                    } = other;
                    if h != head || n != name {
                        return false;
                    }
                    let saved = self.state.subst.clone();
                    let matched = self.state.subst.unify(ty, t).is_ok()
                        && scheme.variables.iter().all(|var| {
                            self.apply(&Type::Var(*var)) == Type::Var(*var)
                        });
                    if !matched {
                        self.state.subst = saved;
                    }
                    matched
                })
            });
        if matches {
            let mut vars = Vars::new(SelfKind::Type(head.clone()));
            let mut defaults = Vec::new();
            let mut arguments: Vec<_> =
                arguments.iter().map(|a| self.apply(a)).collect();
            for (parameter, argument) in parameters.iter().zip(&arguments) {
                defaults.push(
                    parameter.default.map(|d| self.lower(&scope, d, &mut vars)),
                );
                vars.names
                    .push((super::last(&parameter.name), argument.clone()));
            }
            while arguments
                .last()
                .zip(defaults.last())
                .is_some_and(|(a, d)| d.as_ref() == Some(a))
            {
                arguments.pop();
                defaults.pop();
            }
            let label =
                Type::named(origin.clone(), &super::last(name), arguments);
            let members: Vec<_> =
                members.iter().map(|o| o.map(|t| self.apply(t))).collect();
            folded = Some((label, members));
        }
        self.state = before;
        folded
    }
}
