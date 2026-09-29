//! Dots (types.md §Fields; host.md §Name lookup): a field of the head's
//! type, else a function in scope with a `self` spot, filled with the
//! head. A dot on a head still unknown is a member obligation, solved
//! the same way once the head is known; a `.name` selector is the same
//! dot on its parameter.

use super::{
    arms::heads_in, lower::Scope, lower::SelfKind, lower::Vars, Callee,
    Expected, Infer, Key, Owed, Reason,
};
use crate::{
    ast::{Entry, ExprKind, ImplMember, Module, Name, PatternKind},
    resolve::{resolve_partial, Origin, Target},
    token::Span,
    types::{Obligation, Scheme, Subst, Type},
};
use alloc::{string::String, vec::Vec};
use core::ops::ControlFlow;

impl<'a> Infer<'a> {
    /// `head.a.b`: each member of the value before it.
    pub(super) fn members(
        &mut self,
        head: Type,
        members: &[String],
        span: Span,
    ) -> Callee {
        let mut current = head;
        for (index, member) in members.iter().enumerate() {
            let applied = self.apply(&current);
            if matches!(applied, Type::Var(_)) {
                current = self.fresh();
                let obligation = Obligation::Member {
                    head: applied,
                    name: member.clone(),
                    ty: current.clone(),
                    dot: span,
                };
                self.state.obligations.push(Owed::new(obligation, span));
                continue;
            }
            if let Some(ty) = self.field(&applied, member) {
                current = ty;
                continue;
            }
            let Some(mut callee) = self.self_function(member, &applied, span)
            else {
                let reason = Reason::NoField {
                    ty: applied,
                    other: span,
                };
                self.error(span, reason);
                return Callee::plain(self.fresh());
            };
            let slot = callee.self_slot.unwrap_or_default();
            callee.supplied = Some((slot, applied, span));
            if index + 1 == members.len() {
                return callee;
            }
            current = self.apply_callee(callee, &[], None, span, None);
        }
        Callee::plain(current)
    }

    /// Resolve the member obligations whose head is known.
    pub(super) fn dots(&mut self) {
        loop {
            let ready: Vec<usize> = (0..self.state.obligations.len())
                .filter(|index| {
                    let owed = &self.state.obligations[*index];
                    matches!(owed.obligation, Obligation::Member { .. })
                        && !owed.solved
                        && !matches!(
                            self.apply(owed.obligation.subject()),
                            Type::Var(_)
                        )
                })
                .collect();
            if ready.is_empty() {
                return;
            }
            for index in ready {
                // Solving applies callees, which may solve this one first.
                let owed = &mut self.state.obligations[index];
                if !owed.solved {
                    owed.solved = true;
                    let owed = owed.clone();
                    self.member(&owed);
                }
            }
        }
    }

    /// `head.name: ty` on a known head: its field, else the remaining
    /// application of a `self`-spot function to it.
    fn member(&mut self, owed: &Owed) {
        let Obligation::Member {
            head,
            name,
            ty,
            dot,
        } = &owed.obligation
        else {
            return;
        };
        let (head, dot) = (self.apply(head), *dot);
        let value = if let Some(field) = self.field(&head, name) {
            field
        } else if let Some(mut callee) = self.self_function(name, &head, dot) {
            let slot = callee.self_slot.unwrap_or_default();
            callee.supplied = Some((slot, head, dot));
            self.apply_callee(callee, &[], None, dot, None)
        } else {
            let other = owed.span;
            self.error(dot, Reason::NoField { ty: head, other });
            return;
        };
        self.unify(ty, &value, dot, owed.span);
    }

    /// A readable field of a record type.
    fn field(&mut self, ty: &Type, name: &str) -> Option<Type> {
        self.fields(ty)?
            .into_iter()
            .find(|field| field.name == name && field.public)
            .map(|field| field.ty)
    }

    /// A function `name` in scope with a `self` spot: this module's
    /// items, the head type's own members, then the prelude and the
    /// commands.
    fn self_function(
        &mut self,
        name: &str,
        ty: &Type,
        span: Span,
    ) -> Option<Callee> {
        let mut keys = alloc::vec![Key::Binding(String::from(name))];
        if let Type::Named { name: owner, .. } = ty {
            if matches!(owner.origin, Origin::This | Origin::Line(_)) {
                keys.push(Key::Member(owner.name.clone(), String::from(name)));
            }
        }
        for key in keys {
            let Some(index) = self.item_index(&key) else {
                continue;
            };
            let has_self = self.items[index]
                .heads
                .iter()
                .flatten()
                .any(|head| head.name.as_deref() == Some("self"));
            if has_self {
                return Some(self.item_callee(index, span));
            }
        }
        // An accepted line's function, then its member of the type.
        let value = match self.frame.and_then(|frame| frame.values.get(name)) {
            Some(Target::Local { site }) => Some(*site),
            _ => None,
        };
        let member = match ty {
            Type::Named { name: owner, .. } => self.frame_member(owner, name),
            _ => None,
        };
        for site in [value, member].into_iter().flatten() {
            let has_self = self
                .frame
                .and_then(|frame| frame.locals.get(&(site.start, site.end)))
                .and_then(|local| local.heads.as_ref())
                .is_some_and(|heads| {
                    heads.iter().any(|h| h.name.as_deref() == Some("self"))
                });
            if has_self {
                return Some(self.local(site, span));
            }
        }
        for origin in [Origin::Prelude, Origin::Commands] {
            if self
                .namespace(&origin)
                .and_then(|n| n.value(name))
                .is_none()
            {
                continue;
            }
            let obligations = self.state.obligations.len();
            let callee = self.declared_value(&origin, name, span);
            if callee.self_slot.is_some() {
                return Some(callee);
            }
            self.state.obligations.truncate(obligations);
        }
        None
    }

    /// `.name` as a value: `a -> b` with the member obligation
    /// `a.name: b`, solved now when the expected type knows `a`.
    pub(super) fn selector(
        &mut self,
        name: &Name,
        expected: Option<&Expected>,
        span: Span,
    ) -> Type {
        let record = self.fresh();
        let ty = Type::function(record.clone(), self.fresh());
        if let Some(expected) = expected {
            self.hint(&expected.ty, &ty);
        }
        let callee = self.members(record, &name.segments, span);
        let value = self.apply_callee(callee, &[], None, span, None);
        let Type::Function { result, .. } = &ty else {
            unreachable!()
        };
        self.unify(result, &value, span, span);
        ty
    }

    /// `T.member`: an impl member of `T`, or the trait method an impl of
    /// `T` implements, with `Self` as `T`.
    pub(super) fn type_member(
        &mut self,
        ty: &Target,
        member: &str,
        span: Span,
    ) -> Callee {
        let named = match ty {
            Target::Type { origin, name } => {
                let count = self
                    .owner(origin)
                    .and_then(|m| super::lower::type_item(m, name))
                    .map_or(0, |(parameters, _)| parameters.len());
                let arguments = (0..count).map(|_| self.fresh()).collect();
                Some(Type::named(origin.clone(), name, arguments))
            }
            Target::SelfType => self.self_type.clone(),
            _ => None,
        };
        let Some(named @ Type::Named { .. }) = named else {
            self.error(span, Reason::NotAFunction);
            return Callee::plain(self.fresh());
        };
        let Type::Named { name: owner, .. } = &named else {
            unreachable!()
        };
        let owner = owner.clone();
        if matches!(owner.origin, Origin::This | Origin::Line(_)) {
            let key = Key::Member(owner.name.clone(), String::from(member));
            if let Some(index) = self.item_index(&key) {
                return self.item_callee(index, span);
            }
        }
        if let Some(site) = self.frame_member(&owner, member) {
            return self.local(site, span);
        }
        self.trait_member(&owner.origin, &owner.name, &named, member, span)
            .unwrap_or_else(|| {
                self.error(span, Reason::NotAFunction);
                Callee::plain(self.fresh())
            })
    }

    fn trait_member(
        &mut self,
        origin: &Origin,
        ty: &str,
        named: &Type,
        member: &str,
        span: Span,
    ) -> Option<Callee> {
        for (module, owner_scope) in self.impl_scopes(origin, ty) {
            match self.trait_member_in(
                module,
                &owner_scope,
                ty,
                named,
                member,
                span,
            ) {
                ControlFlow::Break(callee) => return callee,
                ControlFlow::Continue(()) => {}
            }
        }
        None
    }

    /// The modules that may hold impls of type `ty`: its own, or for an
    /// accepted line's type, this module and the lines with its impls.
    fn impl_scopes(
        &self,
        origin: &Origin,
        ty: &str,
    ) -> Vec<(&'a Module, Scope<'a>)> {
        let Origin::Line(line) = origin else {
            return self
                .owner(origin)
                .zip(self.scope(origin))
                .into_iter()
                .collect();
        };
        let mut scopes = alloc::vec![(self.module, Scope::This)];
        let lines = self
            .frame
            .and_then(|frame| frame.impls.get(&(*line, String::from(ty))));
        for index in lines.into_iter().flatten() {
            let origin = Origin::Line(*index);
            if let (Some(module), Some(scope)) =
                (self.owner(&origin), self.scope(&origin))
            {
                scopes.push((module, scope));
            }
        }
        scopes
    }

    /// [`Self::trait_member`] among the impls of one module: continue to
    /// look further, break with `None` when `member` is an associated
    /// type.
    fn trait_member_in(
        &mut self,
        module: &'a Module,
        owner_scope: &Scope<'a>,
        ty: &str,
        named: &Type,
        member: &str,
        span: Span,
    ) -> ControlFlow<Option<Callee>> {
        for (implements, members) in super::lower::impls(module, ty) {
            let associated = members.iter().any(|m| {
                matches!(m, ImplMember::Type { name, .. } if super::last(name) == member)
            });
            if associated {
                return ControlFlow::Break(None);
            }
            let Some(implements) = implements else {
                continue;
            };
            let Some(class) = super::lower::named(module, implements) else {
                continue;
            };
            let Some(trait_scope) = self.trait_scope(&class, owner_scope)
            else {
                continue;
            };
            let declares = {
                let trait_module = trait_scope.module(self.module);
                super::lower::trait_item(trait_module, &class).is_some_and(
                    |(_, members)| {
                        members.iter().any(|m| super::last(&m.name) == member)
                    },
                )
            };
            if !declares {
                continue;
            }
            let arguments = match module.ty(implements).map(|n| &n.kind) {
                Some(crate::ast::TypeKind::Named { arguments, .. }) => {
                    arguments.clone()
                }
                _ => Vec::new(),
            };
            let mut vars = Vars::new(SelfKind::Type(named.clone()));
            let arguments: Vec<Type> = arguments
                .iter()
                .map(|a| self.lower(owner_scope, *a, &mut vars))
                .collect();
            let self_kind = SelfKind::Type(named.clone());
            return ControlFlow::Break(Some(self.method(
                &trait_scope,
                &class,
                member,
                &self_kind,
                Some(&arguments),
                span,
            )));
        }
        ControlFlow::Continue(())
    }

    /// The site of an accepted line's inherent member of its type.
    fn frame_member(
        &self,
        owner: &crate::types::TypeName,
        member: &str,
    ) -> Option<Span> {
        let Origin::Line(line) = owner.origin else {
            return None;
        };
        let key = ((line, owner.name.clone()), String::from(member));
        let (start, end) = *self.frame?.members.get(&key)?;
        Some(Span { start, end })
    }

    /// Where trait `class` is declared: the type's module, this module,
    /// the prelude or the commands.
    fn trait_scope(&self, class: &str, near: &Scope<'a>) -> Option<Scope<'a>> {
        let near_module = near.module(self.module);
        if super::lower::trait_item(near_module, class).is_some() {
            return Some(near.clone());
        }
        if super::lower::trait_item(self.module, class).is_some() {
            return Some(Scope::This);
        }
        if let Some(Target::Trait { origin, .. }) =
            self.frame.and_then(|frame| frame.types.get(class))
        {
            return self.scope(origin);
        }
        [Origin::Prelude, Origin::Commands]
            .into_iter()
            .find_map(|origin| {
                let module = self.owner(&origin)?;
                super::lower::trait_item(module, class)?;
                Some(Scope::Declared { module, origin })
            })
    }

    /// A binding of a file module: that module typed on its own, its
    /// binding's type instantiated here.
    pub(super) fn file_value(
        &mut self,
        stem: &str,
        name: &str,
        span: Span,
    ) -> Callee {
        let origin = Origin::File(String::from(stem));
        let Some(module) = self.owner(&origin) else {
            return Callee::plain(self.fresh());
        };
        if self.visiting.iter().any(|s| s == stem) {
            self.error(span, Reason::ImportCycle);
            return Callee::plain(self.fresh());
        }
        // A file whose other lines fail still types the ones that resolve;
        // the session reports its errors where it loads it.
        let (resolved, _) =
            resolve_partial(module, self.declarations, self.files);
        let mut visiting = self.visiting.clone();
        visiting.push(String::from(stem));
        let (typed, errors) = super::run(
            module,
            &resolved,
            self.declarations,
            self.files,
            None,
            &visiting,
            usize::MAX,
        );
        // A cycle below leaves the module's types unknown, not general.
        if errors.iter().any(|e| e.reason == Reason::ImportCycle) {
            self.error(span, Reason::ImportCycle);
            return Callee::plain(self.fresh());
        }
        // The module's final binding of the name is its export.
        let binding = module.entries.iter().rev().find_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let ExprKind::Bind { pattern, value, .. } =
                &module.expressions[id.0].kind
            else {
                return None;
            };
            match &module.patterns[pattern.0].kind {
                PatternKind::Name(n) if super::last(n) == name => {
                    Some((*id, *value))
                }
                _ => None,
            }
        });
        let Some((id, value)) = binding else {
            return Callee::plain(self.fresh());
        };
        let Some(scheme) = typed.binding(id) else {
            return Callee::plain(self.fresh());
        };
        // Its variables are another substitution's: quantify them all.
        let scheme = Scheme::generalize(
            &scheme.ty,
            &scheme.obligations,
            &[],
            &Subst::default(),
        );
        let ty = self.instantiate(&scheme, span);

        let heads = match &module.expressions[value.0].kind {
            ExprKind::Arms(arms) => Some(heads_in(module, arms)),
            _ => None,
        };
        self.with_heads(ty, heads.as_deref())
    }
}
