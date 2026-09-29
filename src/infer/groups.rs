//! Hoisted file-level arm bindings ([types.md] Generalization): groups
//! by reference, each typed monomorphically and generalized at its
//! boundary, when its first member's line or first outside use comes.
//! A use inside the group takes a copy of the member's type, unified
//! with it at the group's end, so a use at a second type names both
//! uses: there is no polymorphic recursion.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/types.md

use super::{bind::Part, Callee, Expected, Infer, Status, Use};
use crate::{
    ast::ExprKind,
    resolve::{Origin, Target},
    token::Span,
    types::{Scheme, Subst, Type},
};
use alloc::{vec, vec::Vec};

impl Infer<'_> {
    /// Group the hoisted items by the references among their values:
    /// strongly connected components of the resolver's targets.
    pub(super) fn group_items(&mut self) {
        let hoisted: Vec<usize> = (0..self.items.len())
            .filter(|index| self.items[*index].line.is_some())
            .collect();
        let spans: Vec<Span> = hoisted
            .iter()
            .filter_map(|index| self.items[*index].line)
            .map(|line| self.span(line))
            .collect();
        let mut edges = vec![Vec::new(); hoisted.len()];
        for (id, target) in self.resolved.expressions() {
            let Some(name) = value_name(target) else {
                continue;
            };
            let to = hoisted.iter().position(|index| {
                matches!(&self.items[*index].key, super::Key::Binding(n) if n == name)
            });
            let span = self.span(id);
            let from = spans
                .iter()
                .position(|s| s.start <= span.start && span.end <= s.end);
            if let (Some(from), Some(to)) = (from, to) {
                edges[from].push(to);
            }
        }
        for component in components(&edges) {
            let group = self.groups.len();
            let members: Vec<usize> =
                component.iter().map(|index| hoisted[*index]).collect();
            for member in &members {
                self.items[*member].group = group;
            }
            self.groups.push(members);
        }
    }

    /// A value of this module's items: an instance of a hoisted
    /// binding's scheme, typing its group first; a copy inside the
    /// group; the item's type otherwise.
    pub(super) fn item_callee(&mut self, index: usize, span: Span) -> Callee {
        let (ty, heads) = {
            let item = &self.items[index];
            (item.ty.clone(), item.heads.clone())
        };
        let ty = match self.state.status[index].clone() {
            Status::Mono => ty,
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

    /// Type one group, then generalize its members together.
    pub(super) fn type_group(&mut self, group: usize) {
        let members = self.groups[group].clone();
        let self_type = self.self_type.take();
        for member in &members {
            self.state.status[*member] = Status::Typing;
        }
        let mark = self.mark();
        let mut parts = Vec::new();
        for member in &members {
            parts.push(self.hoisted_value(*member));
        }
        self.flush_uses(&members);
        let schemes = self.generalize(&parts, mark, &members);
        for (member, scheme) in members.iter().zip(schemes) {
            if let Some(line) = self.items[*member].line {
                self.state.bindings[line.0] = Some(scheme.clone());
            }
            self.state.status[*member] = Status::Done(scheme);
        }
        self.self_type = self_type;
    }

    /// A hoisted binding's value, checked against its item's type.
    fn hoisted_value(&mut self, index: usize) -> Part {
        let module = self.module;
        let item = &self.items[index];
        let mut part = Part {
            ty: item.ty.clone(),
            site: item.site,
            annotated: false,
            public: item.public,
        };
        let Some(line) = item.line else { return part };
        let ExprKind::Bind {
            value, annotation, ..
        } = &module.expressions[line.0].kind
        else {
            return part;
        };
        part.annotated = annotation.is_some();
        let span = annotation.map_or(part.site, |a| module.types[a.0].span);
        let expected = Expected::new(part.ty.clone(), span);
        self.expr(*value, Some(&expected));
        self.state.types[line.0] = Some(Type::Unit);
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

/// The name of this module's value a target reads, through members.
fn value_name(target: &Target) -> Option<&str> {
    match target {
        Target::Value {
            origin: Origin::This,
            name,
        } => Some(name),
        Target::Members { head, .. } => value_name(head),
        _ => None,
    }
}

/// Strongly connected components (Tarjan), each in index order.
fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct Walk<'e> {
        edges: &'e [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(walk: &mut Walk<'_>, node: usize) {
        walk.index[node] = Some(walk.next);
        walk.low[node] = walk.next;
        walk.next += 1;
        walk.stack.push(node);
        walk.on_stack[node] = true;
        let edges = walk.edges;
        for &to in &edges[node] {
            match walk.index[to] {
                None => {
                    visit(walk, to);
                    walk.low[node] = walk.low[node].min(walk.low[to]);
                }
                Some(index) if walk.on_stack[to] => {
                    walk.low[node] = walk.low[node].min(index);
                }
                Some(_) => {}
            }
        }
        if walk.index[node] == Some(walk.low[node]) {
            let mut component = Vec::new();
            while let Some(member) = walk.stack.pop() {
                walk.on_stack[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            component.sort_unstable();
            walk.out.push(component);
        }
    }
    let count = edges.len();
    let mut walk = Walk {
        edges,
        index: vec![None; count],
        low: vec![0; count],
        stack: Vec::new(),
        on_stack: vec![false; count],
        next: 0,
        out: Vec::new(),
    };
    for node in 0..count {
        if walk.index[node].is_none() {
            visit(&mut walk, node);
        }
    }
    walk.out
}
