//! Preserve checked type layouts before source references become IR indices.
use super::{
    lower::{SelfKind, Vars},
    Infer,
};
use crate::{
    ast::{Entry, ItemKind, Member},
    typed::{ConstructorShape, DefaultValue, FieldShape, Shape},
    types::Type,
};
use alloc::vec::Vec;

impl Infer<'_> {
    pub(super) fn record_operations(&mut self) {
        self.record_implementations();
        self.record_operator_operands();
        self.record_readers();
        self.record_pattern_equals();
        self.record_cast_operations();
        self.record_constructors();
        self.record_defaults();
    }

    pub(super) fn record_shapes(&mut self) -> Vec<Shape> {
        if !self.state.errors.is_empty() {
            return Vec::new();
        }
        let mut pending: Vec<_> = self
            .state
            .types
            .iter()
            .flatten()
            .map(|t| self.apply(t))
            .collect();
        pending.extend(
            self.state.locals.values().map(|l| self.apply(&l.scheme.ty)),
        );
        pending
            .extend(self.state.callee_types.values().map(|ty| self.apply(ty)));
        // A literal pattern's equality is no expression, yet a backend
        // switches on its Bool, so the unit carries that layout too.
        pending.extend(
            self.state
                .pattern_equals
                .values()
                .map(|comparison| self.apply(&comparison.ty)),
        );
        for cast in self.state.casts.iter().flatten() {
            pending.push(self.apply(&cast.from));
            pending.push(self.apply(&cast.to));
            if let Some(operation) = &cast.operation {
                pending.push(self.apply(&operation.ty));
            }
        }
        let mut shapes: Vec<Shape> = Vec::new();
        while let Some(ty) = pending.pop() {
            match &ty {
                Type::Named { name, arguments } => {
                    pending.extend(arguments.iter().cloned());
                    // ponytail: linear layout interning; index it if units carry many distinct instantiations.
                    if shapes.iter().any(|shape| matches!(&shape.ty, Type::Named { name: existing, .. } if existing == name)) {
                        continue;
                    }
                    let parameters =
                        arguments.iter().map(|_| self.fresh()).collect();
                    let template = Type::Named {
                        name: name.clone(),
                        arguments: parameters,
                    };
                    let Type::Named { arguments, .. } = &template else {
                        unreachable!()
                    };
                    let Some(constructors) =
                        self.shape_constructors(&template, name, arguments)
                    else {
                        continue;
                    };
                    for constructor in &constructors {
                        pending.extend(
                            constructor.fields.iter().map(|f| f.ty.clone()),
                        );
                    }
                    let bases = self.shape_bases(&template, name, arguments);
                    pending.extend(bases.iter().cloned());
                    shapes.push(Shape {
                        ty: template,
                        bases,
                        constructors,
                    });
                }
                Type::Array { element, length }
                | Type::Function {
                    parameter: element,
                    result: length,
                } => {
                    pending.push((**element).clone());
                    pending.push((**length).clone());
                }
                Type::Tuple(parts)
                | Type::Either(parts)
                | Type::Both(parts)
                | Type::Sum(parts) => pending.extend(parts.iter().cloned()),
                Type::Not(inner) => pending.push((**inner).clone()),
                Type::Limited { name, meaning } => {
                    pending.push((**name).clone());
                    pending.push((**meaning).clone());
                }
                Type::Unit
                | Type::Number(_)
                | Type::Var(_)
                | Type::Rigid(_) => (),
            }
        }
        shapes
    }

    fn shape_bases(
        &mut self,
        ty: &Type,
        name: &crate::types::TypeName,
        arguments: &[Type],
    ) -> Vec<Type> {
        let Some(module) = self.owner(&name.origin) else {
            return Vec::new();
        };
        let Some(scope) = self.scope(&name.origin) else {
            return Vec::new();
        };
        let Some((parameters, base)) =
            module.entries.iter().find_map(|entry| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let ItemKind::Type {
                    name: declared,
                    parameters,
                    base,
                    ..
                } = &item.kind
                else {
                    return None;
                };
                (super::last(declared) == name.name)
                    .then_some((parameters, base))
            })
        else {
            return Vec::new();
        };
        let Some(base) = base else { return Vec::new() };
        let mut vars = Vars::new(SelfKind::Type(ty.clone()));
        vars.names.extend(parameters.iter().zip(arguments).map(
            |(parameter, argument)| {
                (super::last(&parameter.name), argument.clone())
            },
        ));
        let parts = match &module.types[base.0].kind {
            crate::ast::TypeKind::Both(parts) => parts.clone(),
            _ => alloc::vec![*base],
        };
        parts
            .iter()
            .map(|part| self.lower(&scope, *part, &mut vars))
            .collect()
    }
    fn shape_constructors(
        &mut self,
        ty: &Type,
        name: &crate::types::TypeName,
        arguments: &[Type],
    ) -> Option<Vec<ConstructorShape>> {
        let module = self.owner(&name.origin)?;
        let scope = self.scope(&name.origin)?;
        let (parameters, members) = self.type_item(module, &name.name)?;
        let mut vars = Vars::new(SelfKind::Type(ty.clone()));
        vars.names.extend(
            parameters
                .iter()
                .zip(arguments)
                .map(|(p, t)| (super::last(&p.name), t.clone())),
        );
        let mut constructors = Vec::new();
        let field =
            |this: &mut Self, f: &crate::ast::Field, vars: &mut Vars| {
                let field_ty = this.lower(&scope, f.ty, vars);
                let default = f.default.map(|value| DefaultValue::Declared {
                    origin: name.origin.clone(),
                    value,
                });
                FieldShape {
                    name: super::last(&f.name),
                    ty: field_ty,
                    default,
                    public: f.public,
                }
            };
        let members = members.map_or(&[][..], Vec::as_slice);
        if members.iter().any(|m| matches!(m, Member::Constructor { .. })) {
                        for member in members {
                            if let Member::Constructor { name, fields, .. } = member {
                                constructors.push(ConstructorShape {
                                    name: super::last(name),
                                    fields: fields.iter().map(|f| field(self,f,&mut vars)).collect(),
                                });
                            }
                        }
                    } else if !members.is_empty() || module.entries.iter().any(|entry| matches!(entry,Entry::Item(item) if matches!(&item.kind,ItemKind::Type { name: n,members: Some(_),.. } if super::last(n)==name.name))) {
                        constructors.push(ConstructorShape {
                            name: name.name.clone(),
                            fields: members.iter().filter_map(|m| match m { Member::Field(f) => Some(field(self,f,&mut vars)), Member::Constructor { .. } => None }).collect(),
                        });
                    }
        Some(constructors)
    }
}

impl Infer<'_> {
    pub(super) fn main_type(&mut self) -> Type {
        let mut main = self
            .module
            .entries
            .iter()
            .rev()
            .find_map(|entry| {
                if let Entry::Line(id) = entry {
                    self.state.types[id.0].clone()
                } else {
                    None
                }
            })
            .unwrap_or(Type::Unit);
        for lift in core::mem::take(&mut self.state.main_lifts)
            .into_iter()
            .rev()
        {
            main = self.rebuild(
                &self.apply(&lift.value),
                true,
                main,
                lift.span,
                lift.span,
            );
        }
        main
    }
}

impl Infer<'_> {
    pub(super) fn record_implementations(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let uses: Vec<_> = self
            .state
            .selected
            .iter()
            .filter_map(|(site, selected)| {
                let crate::typed::Selected::Member {
                    head,
                    name,
                    argument,
                } = selected
                else {
                    return None;
                };
                Some((
                    *site,
                    self.apply(head),
                    name.clone(),
                    argument.as_ref().map(|ty| self.apply(ty)),
                ))
            })
            .collect();
        for (site, head, name, argument) in uses {
            let head = super::meaning(&head);
            if matches!(head, Type::Var(_) | Type::Rigid(_)) {
                continue;
            }
            let span = crate::token::Span {
                start: site.0,
                end: site.1,
            };
            let Some(mut callee) = self.self_function(&name, head, span) else {
                continue;
            };
            if callee.conversion.is_some() {
                let Some(source) = argument.as_ref() else {
                    continue;
                };
                let Some(chosen) =
                    self.record_overload(head, &name, source, span)
                else {
                    continue;
                };
                callee = chosen;
            }
            if callee.selected.is_none() {
                continue;
            }
            if let Some(spots) = &callee.spots {
                for (index, slot) in spots.iter().enumerate() {
                    let supplied = if Some(index) == callee.self_slot {
                        Some(head)
                    } else {
                        argument.as_ref()
                    };
                    if let Some(ty) = supplied {
                        self.unify(&slot.ty, ty, span, span);
                    }
                }
            }
            self.record_callee(&callee, span);
            self.record_operator_binding(&callee, span);
        }
    }
}

impl Infer<'_> {
    fn record_overload(
        &mut self,
        head: &Type,
        member: &str,
        source: &Type,
        span: crate::token::Span,
    ) -> Option<super::Callee> {
        let Type::Named { name, .. } = head else {
            return None;
        };
        let key = super::Key::Member(
            name.name.clone(),
            alloc::string::String::from(member),
        );
        let indices: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (item.key == key).then_some(i))
            .collect();
        let mut choices = Vec::new();
        if name.origin == crate::resolve::Origin::This {
            for index in indices {
                choices.push(self.item_callee(index, span));
            }
        }
        choices.extend(self.conversion_members(
            &name.origin,
            &name.name,
            head,
            member,
            span,
        ));
        let mut matched = choices.into_iter().filter(|callee| {
            callee.spots.as_ref().is_some_and(|spots| {
                spots
                    .iter()
                    .enumerate()
                    .find(|(index, _)| Some(*index) != callee.self_slot)
                    .is_some_and(|(_, spot)| self.same_source(&spot.ty, source))
            })
        });
        let selected = matched.next()?;
        if matched.next().is_some() {
            None
        } else {
            Some(selected)
        }
    }
}

impl Infer<'_> {
    pub(super) fn record_operation(
        &mut self,
        span: crate::token::Span,
        head: Type,
        name: &str,
        argument: Option<Type>,
        result: Type,
    ) {
        let selected = crate::typed::Selected::Member {
            head: head.clone(),
            name: alloc::string::String::from(name),
            argument: argument.clone(),
        };
        let mut spots = Vec::new();
        for ty in argument.into_iter().chain(core::iter::once(head)) {
            spots.push(super::Slot {
                place_kind: crate::typed::Place::Parameter(None),
                name: None,
                ty,
                default: false,
                value: None,
                site: span,
            });
        }
        let self_slot = Some(spots.len() - 1);
        let mut callee = super::Callee::plain(result);
        callee.selected = Some(selected);
        callee.spots = Some(spots);
        callee.self_slot = self_slot;
        self.record_callee(&callee, span);
        self.record_operator_binding(&callee, span);
    }
}

impl Infer<'_> {
    pub(super) fn record_readers(&mut self) {
        if !self.state.errors.is_empty() {
            return;
        }
        let readers: Vec<_> = self
            .state
            .selected
            .iter()
            .filter_map(|(site, selected)| {
                let crate::typed::Selected::Static { head, name } = selected
                else {
                    return None;
                };
                Some((*site, self.apply(head), name.clone()))
            })
            .collect();
        for (site, head, member) in readers {
            let Type::Named { name, .. } = super::meaning(&head) else {
                continue;
            };
            let span = crate::token::Span {
                start: site.0,
                end: site.1,
            };
            let target = crate::resolve::Target::Type {
                origin: name.origin.clone(),
                name: name.name.clone(),
            };
            let callee = self.type_member(&target, &member, span);
            self.unify(&callee.ty, &head, span, span);
            self.record_callee(&callee, span);
        }
    }
}

pub(super) fn apply_bound(
    binding: &mut crate::typed::Bound,
    subst: &crate::types::Subst,
) {
    for (_, ty) in &mut binding.spots {
        *ty = subst.apply(ty);
    }
    for (ty, _) in &mut binding.applied {
        *ty = subst.apply(ty);
    }
    for (_, default) in &mut binding.defaults {
        apply_default(default, subst);
    }
}

pub(super) fn apply_default(
    default: &mut DefaultValue,
    subst: &crate::types::Subst,
) {
    match default {
        DefaultValue::Type(ty) => *ty = subst.apply(ty),
        DefaultValue::Prepared {
            selected,
            meanings,
            ty,
        } => {
            **selected = selected.applied(subst);
            *ty = subst.apply(ty);
            for meaning in meanings {
                *meaning = subst.apply(meaning);
            }
        }
        DefaultValue::Record { ty, fields } => {
            *ty = subst.apply(ty);
            for (_, ty, value) in fields {
                *ty = subst.apply(ty);
                apply_default(value, subst);
            }
        }
        DefaultValue::Array { ty, element } => {
            *ty = subst.apply(ty);
            apply_default(element, subst);
        }
        DefaultValue::Expression(instance) => {
            **instance = apply_recorded(instance, subst);
        }
        DefaultValue::Declared { .. } => (),
    }
}

pub(super) fn apply_recorded(
    instance: &crate::typed::DeclaredExpression,
    subst: &crate::types::Subst,
) -> crate::typed::DeclaredExpression {
    let mut instance = instance.clone();
    let typed = alloc::rc::Rc::make_mut(&mut instance.typed);
    for ty in typed
        .expressions
        .iter_mut()
        .flatten()
        .chain(typed.pattern_types.values_mut())
        .chain(typed.callee_types.values_mut())
        .chain(typed.rebuild_types.values_mut())
        .chain(typed.meaning_arguments.values_mut().flatten())
        .chain(typed.main.iter_mut())
    {
        *ty = subst.apply(ty);
    }
    for scheme in typed
        .bindings
        .iter_mut()
        .flatten()
        .chain(typed.schemes.values_mut())
    {
        *scheme = super::bind::applied(scheme, subst);
    }
    for (_, local) in &mut typed.frozen {
        local.scheme = super::bind::applied(&local.scheme, subst);
    }
    for cast in typed.casts.iter_mut().flatten() {
        cast.from = subst.apply(&cast.from);
        cast.to = subst.apply(&cast.to);
        cast.operation = cast
            .operation
            .as_ref()
            .map(|operation| operation.applied(subst));
    }
    for conversion in typed.conversions.values_mut() {
        conversion.result = subst.apply(&conversion.result);
        conversion.from = subst.apply(&conversion.from);
        conversion.to = subst.apply(&conversion.to);
        conversion.parameter = subst.apply(&conversion.parameter);
    }
    for check in &mut typed.checks {
        check.target = check.target.as_ref().map(|ty| subst.apply(ty));
    }
    for comparison in typed.pattern_equals.values_mut() {
        *comparison = comparison.applied(subst);
    }
    for selected in typed.selected.values_mut() {
        *selected = selected.applied(subst);
    }
    for value in typed.derived_defaults.values_mut() {
        apply_default(value, subst);
    }
    for binding in typed.bound.values_mut() {
        apply_bound(binding, subst);
    }
    for step in typed.accesses.values_mut().flatten() {
        *step = step.applied(subst);
    }
    for root in typed.roots.values_mut() {
        *root = root.applied(subst);
    }
    for shape in &mut typed.shapes {
        shape.ty = subst.apply(&shape.ty);
        for base in &mut shape.bases {
            *base = subst.apply(base);
        }
        for field in shape.constructors.iter_mut().flat_map(|c| &mut c.fields) {
            field.ty = subst.apply(&field.ty);
            if let Some(default) = &mut field.default {
                apply_default(default, subst);
            }
        }
    }
    for obligation in &mut typed.unsolved {
        *obligation = obligation.map(|ty| subst.apply(ty));
    }
    for row in typed
        .dispatch
        .iter_mut()
        .flat_map(|dispatch| &mut dispatch.rows)
    {
        row.receiver = subst.apply(&row.receiver);
        row.argument = row.argument.as_ref().map(|ty| subst.apply(ty));
        row.selected = row.selected.applied(subst);
        row.ty = subst.apply(&row.ty);
        for ty in &mut row.meanings {
            *ty = subst.apply(ty);
        }
    }
    for nested in typed
        .default_instances
        .values_mut()
        .chain(&mut typed.declared)
    {
        *nested = apply_recorded(nested, subst);
    }
    instance
}

impl Infer<'_> {
    /// An operator supplies both its operands. A literal operand's member
    /// resolution can record the operator's site as a member access with
    /// its argument open, as `"a" ++ "b"` did; the right operand fills
    /// that spot here.
    fn record_operator_operands(&mut self) {
        use crate::ast::ExprKind;
        let operators: Vec<_> = self
            .module
            .expressions
            .iter()
            .filter_map(|expr| match expr.kind {
                ExprKind::Binary {
                    operator, right, ..
                } => Some(((operator.start, operator.end), right)),
                _ => None,
            })
            .collect();
        for (site, right) in operators {
            let Some(bound) = self.state.bound.get(&site) else {
                continue;
            };
            let [spot] = bound.open.as_slice() else {
                continue;
            };
            if bound.supplies.len() != 1 {
                continue;
            }
            let mut bound = bound.clone();
            bound
                .supplies
                .push((*spot, crate::typed::Supply::Expression(right)));
            bound.open.clear();
            self.insert(|s| &mut s.bound, site, bound);
        }
    }

    fn record_operator_binding(
        &mut self,
        callee: &super::Callee,
        span: crate::token::Span,
    ) {
        let operands = self.operators.get(&span).copied();
        let (Some((left, right)), Some(spots), Some(receiver)) =
            (operands, &callee.spots, callee.self_slot)
        else {
            return;
        };
        let mut supplies =
            alloc::vec![(receiver, crate::typed::Supply::Expression(left))];
        if let Some(right) = right {
            if let Some(argument) =
                (0..spots.len()).find(|index| *index != receiver)
            {
                supplies
                    .push((argument, crate::typed::Supply::Expression(right)));
            }
        }
        self.insert(
            |s| &mut s.bound,
            (span.start, span.end),
            crate::typed::Bound {
                spots: spots
                    .iter()
                    .map(|slot| (slot.site, slot.ty.clone()))
                    .collect(),
                supplies,
                defaults: Vec::new(),
                open: Vec::new(),
                applied: Vec::new(),
            },
        );
    }
}

impl crate::typed::Root {
    pub(super) fn applied(&self, subst: &crate::types::Subst) -> Self {
        Self {
            selected: self.selected.applied(subst),
            meanings: self.meanings.iter().map(|ty| subst.apply(ty)).collect(),
        }
    }
}

impl crate::typed::Access {
    pub(super) fn applied(&self, subst: &crate::types::Subst) -> Self {
        Self {
            head: subst.apply(&self.head),
            callee: subst.apply(&self.callee),
            result: subst.apply(&self.result),
            selected: self.selected.applied(subst),
            meanings: self.meanings.iter().map(|ty| subst.apply(ty)).collect(),
            bound: self.bound.clone().map(|mut bound| {
                apply_bound(&mut bound, subst);
                bound
            }),
        }
    }
}

impl crate::typed::Conversion {
    pub(super) fn applied(&self, subst: &crate::types::Subst) -> Self {
        Self {
            mode: self.mode,
            result: subst.apply(&self.result),
            from: subst.apply(&self.from),
            to: subst.apply(&self.to),
            parameter: subst.apply(&self.parameter),
        }
    }
}

impl crate::typed::Comparison {
    pub(super) fn applied(&self, subst: &crate::types::Subst) -> Self {
        Self {
            selected: self.selected.applied(subst),
            ty: subst.apply(&self.ty),
            meanings: self.meanings.iter().map(|ty| subst.apply(ty)).collect(),
            self_slot: self.self_slot,
        }
    }
}
