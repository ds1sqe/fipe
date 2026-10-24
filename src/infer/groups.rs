//! The members of one impl ([types.md] Generalization): they see each
//! other in any order, so they are typed as one group, monomorphically,
//! and generalized at its boundary. A use inside the group takes a copy
//! of the member's type, unified with it at the group's end, so a use
//! at a second type names both uses: there is no polymorphic recursion.
//! An arm binding calling itself is the one-binding case, in `bind.rs`.
//!
//! An impl is order-free, but the bindings its members name are
//! sequential. The group is typed at its first use when every
//! file-level binding it names is typed by then, so an early use sees
//! its scheme; otherwise at the impl's own entry, where they all are,
//! and a use before that holds a copy that stays monomorphic until the
//! group is generalized, then is unified with an instance of its scheme.
//! Typing the group early would read those bindings as unknown, and
//! typing them out of order would break their sequence.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/03-types.md

use super::{bind::Part, Body, Callee, Expected, Infer, Status, Use};
use crate::{
    token::Span,
    types::{Scheme, Subst, Type},
};
use alloc::vec::Vec;

impl Infer<'_> {
    /// A value of this module's impls: an instance of the member's
    /// scheme, typing its group first when it can be; a copy inside the
    /// group or before it can be typed.
    pub(super) fn item_callee(&mut self, index: usize, span: Span) -> Callee {
        let item = &self.items[index];
        let (ty, heads, group) =
            (item.ty.clone(), item.heads.clone(), item.group);
        let owner = self.groups[group].self_type.clone();
        let (ty, meanings, owner) = match self.state.status[index].clone() {
            Status::Pending if self.ready(group) => {
                self.type_group(group);
                return self.item_callee(index, span);
            }
            Status::Pending | Status::Typing => {
                let pending =
                    matches!(self.state.status[index], Status::Pending);
                let (ty, owner) =
                    self.copy_member(&ty, owner.as_ref(), pending);
                let used = Use {
                    lexical_owner: self.state.lexical_owner.clone(),
                    item: index,
                    ty: ty.clone(),
                    owner: owner.clone(),
                    span,
                    checked: false,
                };
                if pending {
                    self.state.deferred.push(used);
                } else {
                    self.state.uses.push(used);
                }
                (ty, Vec::new(), owner)
            }
            Status::Done(member) => self.instantiate_member(
                &member.scheme,
                member.owner.as_ref(),
                span,
            ),
        };
        let mut callee = self.with_heads(ty, heads.as_deref());
        callee.owner = owner;
        callee.meanings = meanings;
        callee.selected = Some(crate::typed::Selected::Value {
            origin: crate::resolve::Origin::This,
            site: self.items[index].site,
        });
        if let Some(home) = self.declared_home.clone() {
            let site = self.items[index].site;
            if let Some(annotation) = self.declared_item_annotation(site) {
                let name = match &self.items[index].key {
                    super::Key::Binding(name)
                    | super::Key::Member(_, name)
                    | super::Key::Constructor(name) => name.clone(),
                };
                self.overlay_declared_signature(
                    &mut callee,
                    self.module,
                    annotation,
                    &home,
                    (&name, site),
                    span,
                );
            }
        }
        callee
    }

    fn copy_member(
        &mut self,
        ty: &Type,
        owner: Option<&Type>,
        pending: bool,
    ) -> (Type, Option<Type>) {
        let mut parts = alloc::vec![self.apply(ty)];
        if let Some(owner) = owner {
            parts.push(self.apply(owner));
        }
        let root = Type::Tuple(parts);
        let mut scheme = Scheme::generalize(&root, &[], &[], &Subst::default());
        if pending {
            if let Some(owner) = owner {
                let mut names = Vec::new();
                super::bind::rigids(&self.apply(owner), &mut names);
                let fresh: Vec<_> = names
                    .into_iter()
                    .map(|name| (name, self.state.subst.fresh()))
                    .collect();
                scheme.ty = super::bind::unrigid(&scheme.ty, &fresh);
            }
        }
        let Type::Tuple(mut parts) =
            scheme.instantiate(&mut self.state.subst).0
        else {
            unreachable!("member pair")
        };
        let owner = (parts.len() == 2).then(|| parts.pop().expect("owner"));
        (parts.pop().expect("member"), owner)
    }

    /// Whether every file-level binding the group names is typed.
    fn ready(&self, group: usize) -> bool {
        self.groups[group].names.iter().all(|site| {
            let key = (site.start, site.end);
            (if self.state.lexical_owner.is_none() {
                self.state.locals.contains_key(&key)
            } else {
                self.state
                    .lexical_frames
                    .iter()
                    .rev()
                    .find(|frame| frame.owner.is_none())
                    .is_some_and(|frame| frame.facts.locals.contains_key(&key))
            }) || self.frame.is_some_and(|f| f.locals.contains_key(&key))
        })
    }

    /// Type one impl's members, then generalize them together.
    pub(super) fn type_group(&mut self, group: usize) {
        self.with_lexical_owner(&None, |inner| inner.type_group_owned(group));
    }

    fn type_group_owned(&mut self, group: usize) {
        let members = self.groups[group].members.clone();
        let outer = core::mem::replace(
            &mut self.self_type,
            self.groups[group].self_type.clone(),
        );
        for member in &members {
            self.set_status(*member, Status::Typing);
        }
        let mark = self.mark();
        let mut parts = Vec::new();
        for member in &members {
            parts.push(self.member_value(*member));
        }
        self.flush_uses(&members);
        let schemes = self.generalize(&parts, mark, &members);
        for (member, scheme) in members.iter().zip(schemes) {
            self.set_status(
                *member,
                Status::Done(alloc::boxed::Box::new(scheme)),
            );
        }
        self.self_type = outer;
        self.flush_deferred(&members);
    }

    /// Unify each use made before the group was typed with an instance
    /// of its member's scheme, at that use.
    fn flush_deferred(&mut self, members: &[usize]) {
        self.journal_list(|s| &mut s.deferred);
        let (mine, rest): (Vec<Use>, Vec<Use>) =
            core::mem::take(&mut self.state.deferred)
                .into_iter()
                .partition(|u| members.contains(&u.item));
        self.state.deferred = rest;
        for used in mine {
            let lexical_owner = used.lexical_owner.clone();
            self.with_lexical_owner(&lexical_owner, |inner| {
                inner.flush_deferred_use(used);
            });
        }
    }

    fn flush_deferred_use(&mut self, used: Use) {
        let Status::Done(scheme) = self.state.status[used.item].clone() else {
            return;
        };
        let (ty, meanings, owner) = self.instantiate_member(
            &scheme.scheme,
            scheme.owner.as_ref(),
            used.span,
        );
        if let (Some(owner), Some(actual)) = (owner, used.owner) {
            self.unify(&owner, &actual, used.span, self.items[used.item].site);
        }
        self.set_meaning_arguments((used.span.start, used.span.end), meanings);
        let site = self.items[used.item].site;
        self.unify(&ty, &used.ty, used.span, site);
    }

    /// A member's value, checked against its item's type.
    fn member_value(&mut self, index: usize) -> Part {
        let head = self.self_type.clone();
        self.with_lexical_owner(&None, |inner| {
            let prior = core::mem::replace(&mut inner.self_type, head);
            let part = inner.member_value_owned(index);
            inner.self_type = prior;
            part
        })
    }

    fn member_value_owned(&mut self, index: usize) -> Part {
        let item = &self.items[index];
        let part = Part {
            owner: self.groups[item.group].self_type.clone(),
            ty: item.ty.clone(),
            site: item.site,
            annotated: item.annotated || self.groups[item.group].self_type.as_ref().is_some_and(|ty| matches!(ty, Type::Named { arguments, .. } if !arguments.is_empty())),
        };
        let mut expected = Expected::new(part.ty.clone(), part.site);
        expected.cast = item.annotated;
        match item.body.clone() {
            Some(Body::Value(value)) => {
                self.expr(value, Some(&expected));
            }
            Some(Body::Arms(arms)) => {
                let ty = self.arms(&arms, Some(&expected));
                let mut result = &ty;
                for _ in &arms[0].patterns {
                    let Type::Function { result: next, .. } = result else {
                        break;
                    };
                    result = next;
                }
                if let Some(head) = self.self_type.clone() {
                    self.unify(&head, result, arms[0].span, part.site);
                }
            }
            None => {}
        }
        part
    }

    /// Unify each use inside the group with its member's type, in
    /// order; a failure names the use and the one before it.
    fn flush_uses(&mut self, members: &[usize]) {
        self.journal_list(|s| &mut s.uses);
        let mine: Vec<_> = self
            .state
            .uses
            .iter_mut()
            .filter(|u| members.contains(&u.item) && !u.checked)
            .map(|u| {
                u.checked = true;
                u.clone()
            })
            .collect();
        let mut last: Vec<(usize, Span)> = Vec::new();
        for used in mine {
            let other = last
                .iter()
                .rev()
                .find(|(item, _)| *item == used.item)
                .map_or(self.items[used.item].site, |(_, span)| *span);
            let ty = self.items[used.item].ty.clone();
            self.unify(&ty, &used.ty, used.span, other);
            let owner =
                self.groups[self.items[used.item].group].self_type.clone();
            if let (Some(owner), Some(actual)) = (owner, used.owner) {
                self.unify(&owner, &actual, used.span, other);
            }
            last.push((used.item, used.span));
        }
    }
}
