//! The members of one impl ([types.md] Generalization): they see each
//! other in any order, so they are typed as one group, monomorphically,
//! and generalized at its boundary, when the impl's entry or a first
//! outside use comes. A use inside the group takes a copy of the
//! member's type, unified with it at the group's end, so a use at a
//! second type names both uses: there is no polymorphic recursion. An
//! arm binding calling itself is the one-binding case, in `bind.rs`.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/types.md

use super::{bind::Part, Body, Callee, Expected, Infer, Status, Use};
use crate::{
    token::Span,
    types::{Scheme, Subst},
};
use alloc::vec::Vec;

impl Infer<'_> {
    /// A value of this module's impls: an instance of the member's
    /// scheme, typing its group first; a copy inside the group.
    pub(super) fn item_callee(&mut self, index: usize, span: Span) -> Callee {
        let (ty, heads) = {
            let item = &self.items[index];
            (item.ty.clone(), item.heads.clone())
        };
        let ty = match self.state.status[index].clone() {
            Status::Pending => {
                self.type_group(self.items[index].group);
                return self.item_callee(index, span);
            }
            Status::Typing => {
                let applied = self.apply(&ty);
                let whole =
                    Scheme::generalize(&applied, &[], &[], &Subst::default());
                let (copy, _) = whole.instantiate(&mut self.state.subst);
                self.state.uses.push(Use {
                    item: index,
                    ty: copy.clone(),
                    span,
                });
                copy
            }
            Status::Done(scheme) => self.instantiate(&scheme, span),
        };
        self.with_heads(ty, heads.as_deref())
    }

    /// Type one impl's members, then generalize them together.
    pub(super) fn type_group(&mut self, group: usize) {
        let members = self.groups[group].members.clone();
        let outer = core::mem::replace(
            &mut self.self_type,
            self.groups[group].self_type.clone(),
        );
        for member in &members {
            self.state.status[*member] = Status::Typing;
        }
        let mark = self.mark();
        let mut parts = Vec::new();
        for member in &members {
            parts.push(self.member_value(*member));
        }
        self.flush_uses(&members);
        let schemes = self.generalize(&parts, mark, &members);
        for (member, scheme) in members.iter().zip(schemes) {
            self.state.status[*member] = Status::Done(scheme);
        }
        self.self_type = outer;
    }

    /// A member's value, checked against its item's type.
    fn member_value(&mut self, index: usize) -> Part {
        let item = &self.items[index];
        let part = Part {
            ty: item.ty.clone(),
            site: item.site,
            annotated: item.annotated,
            public: item.public,
        };
        let expected = Expected::new(part.ty.clone(), part.site);
        match item.body.clone() {
            Some(Body::Value(value)) => {
                self.expr(value, Some(&expected));
            }
            Some(Body::Arms(arms)) => {
                self.arms(&arms, Some(&expected));
            }
            None => {}
        }
        part
    }

    /// Unify each use inside the group with its member's type, in
    /// order; a failure names the use and the one before it.
    fn flush_uses(&mut self, members: &[usize]) {
        let (mine, rest): (Vec<Use>, Vec<Use>) =
            core::mem::take(&mut self.state.uses)
                .into_iter()
                .partition(|u| members.contains(&u.item));
        self.state.uses = rest;
        let mut last: Vec<(usize, Span)> = Vec::new();
        for used in mine {
            let other = last
                .iter()
                .rev()
                .find(|(item, _)| *item == used.item)
                .map_or(self.items[used.item].site, |(_, span)| *span);
            let ty = self.items[used.item].ty.clone();
            self.unify(&ty, &used.ty, used.span, other);
            last.push((used.item, used.span));
        }
    }
}
