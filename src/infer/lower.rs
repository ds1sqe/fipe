//! Declared types into [`Type`]s. The prelude, the commands and host
//! modules, and file modules are parsed but not resolved, so their type
//! names are looked up here: the file's own types, then the prelude,
//! then the commands. The module being inferred uses its resolved
//! targets instead.
//!
//! A declaration's lowercase names are its quantified variables, fresh
//! at each lowering, so lowering a reference is instantiation (VAR).

use super::{Callee, Infer, Slot};
use crate::{
    ast::{
        Constraint, ConstraintKind, Entry, ImplMember, ItemKind, Member,
        Module, Name, TypeId, TypeKind, TypeParameter,
    },
    decl::{Kind, Namespace},
    resolve::{Origin, Resolved, Target},
    token::Span,
    types::{Obligation, Type},
};
use alloc::{string::String, vec::Vec};

/// Expand the existing declaration import spelling, without resolving a new type.
fn declared_path(module: &Module, name: &Name) -> Vec<String> {
    let first = name.segments[0].as_str();
    let mut path = name.segments.clone();
    for entry in &module.entries {
        if let Entry::Item(crate::ast::Item {
            kind: ItemKind::Use { names, .. },
            ..
        }) = entry
        {
            if let Some(imported) = names.iter().find(|imported| {
                imported.segments.last().is_some_and(|last| last == first)
            }) {
                path.clone_from(&imported.segments);
                path.extend_from_slice(&name.segments[1..]);
                break;
            }
        }
    }
    path
}

/// Where a type's names are looked up.
#[derive(Clone)]
pub(super) enum Scope<'m> {
    /// The module being inferred, through its resolved targets.
    This,
    /// A declaration or file module, by name.
    Declared { module: &'m Module, origin: Origin },
    /// A session's accepted line, through its resolved targets.
    Line {
        module: &'m Module,
        resolved: &'m Resolved,
        index: usize,
    },
}

impl<'m> Scope<'m> {
    /// The module whose syntax the scope reads; `this` for [`Self::This`].
    pub(super) const fn module(&self, this: &'m Module) -> &'m Module {
        match self {
            Self::This => this,
            Self::Declared { module, .. } | Self::Line { module, .. } => module,
        }
    }
}

/// What `Self` means while lowering.
#[derive(Clone)]
pub(super) enum SelfKind {
    None,
    /// Inside an impl or type body: that type.
    Type(Type),
}

/// The names bound while lowering one declaration.
pub(super) struct Vars {
    /// Type parameters and quantified variables, in order of binding.
    pub names: Vec<(String, Type)>,
    pub self_kind: SelfKind,
    pub obligations: Vec<Obligation>,
    rigid: bool,
    kinds: Vec<(String, bool)>,
}

impl Vars {
    pub const fn new(self_kind: SelfKind) -> Self {
        Self {
            names: Vec::new(),
            self_kind,
            obligations: Vec::new(),
            rigid: false,
            kinds: Vec::new(),
        }
    }
}

/// A record field, lowered.
pub(super) struct Field {
    pub name: String,
    pub ty: Type,
    pub public: bool,
    pub default: bool,
}

impl<'a> Infer<'a> {
    /// The module that declares names of `origin`.
    pub(super) fn owner(&self, origin: &Origin) -> Option<&'a Module> {
        match origin {
            Origin::This => Some(self.module),
            Origin::File(stem) => self
                .files
                .iter()
                .find(|(file, _)| file == stem)
                .map(|(_, module)| module),
            Origin::Line(_) => self.frame?.module(origin),
            _ => self.namespace(origin).map(|namespace| &namespace.module),
        }
    }

    pub(super) fn namespace(&self, origin: &Origin) -> Option<&'a Namespace> {
        let declarations = self.declarations;
        match origin {
            Origin::Prelude => Some(&declarations.prelude),
            Origin::Commands => declarations.commands.as_ref(),
            Origin::Host(name) => declarations.get_module(name),
            Origin::This | Origin::File(_) | Origin::Line(_) => None,
        }
    }

    pub(super) fn scope(&self, origin: &Origin) -> Option<Scope<'a>> {
        if *origin == Origin::This {
            return Some(Scope::This);
        }
        if let Origin::Line(index) = origin {
            let line = self.frame?.lines.get(*index)?;
            return Some(Scope::Line {
                module: &line.module,
                resolved: &line.resolved,
                index: *index,
            });
        }
        Some(Scope::Declared {
            module: self.owner(origin)?,
            origin: origin.clone(),
        })
    }

    /// A type written in the module being inferred; its lowercase names
    /// are rigid.
    pub(super) fn lower_here(&mut self, id: TypeId) -> Type {
        let self_kind = self
            .self_type
            .clone()
            .map_or(SelfKind::None, SelfKind::Type);
        let mut vars = Vars::new(self_kind);
        vars.rigid = true;
        if let Some(Type::Named { name, arguments }) = &self.self_type {
            if let Some((parameters, _)) = self
                .owner(&name.origin)
                .and_then(|m| self.type_item(m, &name.name))
            {
                for (index, parameter) in parameters.iter().enumerate() {
                    let name = super::last(&parameter.name);
                    vars.kinds
                        .push((name.clone(), !parameter.name.is_uppercase()));
                    vars.names.push((
                        name.clone(),
                        arguments
                            .get(index)
                            .cloned()
                            .unwrap_or(Type::Rigid(name)),
                    ));
                }
            }
        }
        let ty = self.lower(&Scope::This, id, &mut vars);
        for obligation in vars.obligations {
            if let Obligation::Claim { head, class, dot } = &obligation {
                if !super::dot::contains_type(&ty, head) {
                    self.error(
                        *dot,
                        super::Reason::Ambiguous {
                            class: alloc::format!("{class}"),
                        },
                    );
                }
            }
            let mut owed =
                super::Owed::new(obligation, self.module.types[id.0].span);
            let assumed = matches!(
                owed.obligation.subject(),
                Type::Rigid(_) | Type::Var(_)
            );
            owed.solved = assumed;
            owed.assumed = assumed;
            self.push_owed(owed);
        }
        ty
    }

    pub(super) fn lower(
        &mut self,
        scope: &Scope<'a>,
        id: TypeId,
        vars: &mut Vars,
    ) -> Type {
        let module = scope.module(self.module);
        let Some(node) = module.ty(id) else {
            return self.fresh();
        };
        Self::impl_parameters(module, node.span, vars);
        match &node.kind {
            TypeKind::Unit => Type::Unit,
            TypeKind::Named { .. } => self.lower_name(scope, id, vars),
            TypeKind::Array { element, length } => Type::Array {
                element: alloc::boxed::Box::new(
                    self.lower(scope, *element, vars),
                ),
                length: alloc::boxed::Box::new(
                    self.lower_number(scope, *length, vars),
                ),
            },
            TypeKind::Sum(_) => self.lower_number(scope, id, vars),
            TypeKind::Number(n) => Type::Number(n.clone()),
            TypeKind::Tuple(parts) => Type::Tuple(
                parts.iter().map(|p| self.lower(scope, *p, vars)).collect(),
            ),
            TypeKind::Not(inner) => {
                let inner = self.lower(scope, *inner, vars);
                if !self
                    .member_limits(&inner, &inner, node.span, &mut Vec::new())
                    .is_empty()
                {
                    self.error(
                        node.span,
                        super::Reason::Limit {
                            message: String::from("negation of a member limit"),
                        },
                    );
                }
                Type::Limited {
                    meaning: alloc::boxed::Box::new(
                        super::meaning(&inner).clone(),
                    ),
                    name: alloc::boxed::Box::new(Type::Not(
                        alloc::boxed::Box::new(inner),
                    )),
                }
            }
            TypeKind::Both(parts) | TypeKind::Either(parts) => {
                let parts: Vec<_> =
                    parts.iter().map(|p| self.lower(scope, *p, vars)).collect();
                let Some(first) =
                    parts.iter().find(|p| !crate::limits::modifier(p))
                else {
                    return self.fresh();
                };
                let meaning = super::meaning(first).clone();
                let same = parts.iter().all(|p| {
                    crate::limits::modifier(p) || super::meaning(p) == &meaning
                });
                if same {
                    let name = if matches!(node.kind, TypeKind::Both(_)) {
                        Type::Both(parts)
                    } else {
                        Type::Either(parts)
                    };
                    Type::Limited {
                        name: alloc::boxed::Box::new(name),
                        meaning: alloc::boxed::Box::new(meaning),
                    }
                } else if matches!(node.kind, TypeKind::Either(_)) {
                    Type::Either(parts)
                } else {
                    self.error(
                        node.span,
                        super::Reason::Limit {
                            message: String::from(
                                "intersection requires one meaning",
                            ),
                        },
                    );
                    self.fresh()
                }
            }
            TypeKind::Function { parameter, result } => {
                let parameter = self.lower(scope, *parameter, vars);
                Type::function(parameter, self.lower(scope, *result, vars))
            }
            TypeKind::Qualified { constraints, ty } => {
                self.constraints(scope, constraints, vars);
                self.lower(scope, *ty, vars)
            }
            TypeKind::Labeled { ty, .. } => self.lower(scope, *ty, vars),
        }
    }

    pub(super) fn limited(&mut self, ty: Type) -> Type {
        let Type::Named { name, arguments } = &ty else {
            return ty;
        };
        if arguments.is_empty() {
            if let Some(lowered) = self.state.lowered.get(name) {
                return lowered.clone();
            }
        }
        if self.state.lowering.contains(name) {
            return ty;
        }
        let (Some(module), Some(scope)) =
            (self.owner(&name.origin), self.scope(&name.origin))
        else {
            return ty;
        };
        let declaration = self
            .type_entries(module, &name.name)
            .into_iter()
            .find_map(|e| match e {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        resource: _,
                        name: n,
                        parameters,
                        base: Some(base),
                        members: None,
                        limits,
                    } if super::last(n) == name.name
                        && !limits.iter().any(|l| {
                            matches!(l, crate::ast::Limit::Member { .. })
                        }) =>
                    {
                        Some((parameters, *base, !limits.is_empty()))
                    }
                    _ => None,
                },
                Entry::Line(_) => None,
            });
        let Some((parameters, base, tested)) = declaration else {
            return ty;
        };
        self.state.lowering.push(name.clone());
        let mut vars = Vars::new(SelfKind::None);
        for (parameter, argument) in parameters.iter().zip(arguments) {
            vars.names
                .push((super::last(&parameter.name), argument.clone()));
        }
        let parts = match &module.types[base.0].kind {
            TypeKind::Both(parts) => parts.clone(),
            _ => alloc::vec![base],
        };
        for part in parts {
            let base = self.lower(&scope, part, &mut vars);
            if crate::limits::modifier(&base) {
                continue;
            }
            // A base of member limits names a combination the type
            // claims, unless the type tests it: then the base is a
            // packed meaning and the type narrows it (06-limits
            // §Declaration).
            let limits = self.member_limits(
                &base,
                &base,
                Span::default(),
                &mut Vec::new(),
            );
            if !limits.is_empty() && !tested {
                continue;
            }
            let meaning = match base {
                Type::Limited { meaning, .. } => *meaning,
                other => other,
            };
            self.state.lowering.pop();
            let result = Type::Limited {
                name: alloc::boxed::Box::new(ty.clone()),
                meaning: alloc::boxed::Box::new(meaning),
            };
            if arguments.is_empty() {
                self.insert(|s| &mut s.lowered, name.clone(), result.clone());
            }
            return result;
        }
        self.state.lowering.pop();
        ty
    }

    fn constraints(
        &mut self,
        scope: &Scope<'a>,
        constraints: &[Constraint],
        vars: &mut Vars,
    ) {
        for constraint in constraints {
            match &constraint.kind {
                ConstraintKind::Named { class, variable } => {
                    let head = self.variable(&super::last(variable), vars);
                    let class = self.lower(scope, *class, vars);
                    if self
                        .member_limits(
                            &class,
                            &head,
                            constraint.span,
                            &mut Vec::new(),
                        )
                        .is_empty()
                    {
                        self.error(
                            constraint.span,
                            super::Reason::ClaimBlock {
                                claim: class.clone(),
                                problem: String::from("is not a member limit"),
                            },
                        );
                    }
                    vars.obligations.push(Obligation::Claim {
                        head,
                        class,
                        dot: constraint.span,
                    });
                }
                ConstraintKind::Member { member, ty } => {
                    let head = self.variable(&member.segments[0], vars);
                    let ty = self.lower(scope, *ty, vars);
                    vars.obligations.push(Obligation::Member {
                        head,
                        name: super::last(member),
                        ty,
                        dot: constraint.span,
                    });
                }
            }
        }
    }

    pub(super) fn member_limits(
        &mut self,
        class: &Type,
        head: &Type,
        dot: Span,
        visiting: &mut Vec<crate::types::TypeName>,
    ) -> Vec<Obligation> {
        let Type::Named { name, arguments } = class else {
            return Vec::new();
        };
        if visiting.contains(name) || self.state.lowering.contains(name) {
            return Vec::new();
        }
        let (Some(module), Some(scope)) =
            (self.owner(&name.origin), self.scope(&name.origin))
        else {
            return Vec::new();
        };
        let declaration = self
            .type_entries(module, &name.name)
            .into_iter()
            .find_map(|entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        resource: _,
                        name: n,
                        parameters,
                        base,
                        limits,
                        members: None,
                    } if super::last(n) == name.name => {
                        Some((parameters, base, limits))
                    }
                    _ => None,
                },
                Entry::Line(_) => None,
            });
        let Some((parameters, base, limits)) = declaration else {
            return Vec::new();
        };
        visiting.push(name.clone());
        let mut vars = Vars::new(SelfKind::Type(head.clone()));
        for (index, parameter) in parameters.iter().enumerate() {
            let argument =
                arguments.get(index).cloned().unwrap_or_else(
                    || match parameter.default {
                        Some(default) => self.lower(&scope, default, &mut vars),
                        None => self.fresh(),
                    },
                );
            vars.names.push((super::last(&parameter.name), argument));
        }
        let mut result = Vec::new();
        if let Some(base) = base {
            let bases = match &module.types[base.0].kind {
                TypeKind::Both(parts) => parts.clone(),
                _ => alloc::vec![*base],
            };
            for base in bases {
                let base = self.lower(&scope, base, &mut vars);
                result.extend(self.member_limits(&base, head, dot, visiting));
            }
        }
        for limit in limits {
            if let crate::ast::Limit::Member { name, ty, .. } = limit {
                let ty = self.lower(&scope, *ty, &mut vars);
                result.push(Obligation::Member {
                    head: head.clone(),
                    name: super::last(name),
                    ty,
                    dot,
                });
            }
        }
        visiting.pop();
        result
    }

    fn impl_parameters(module: &Module, span: Span, vars: &mut Vars) {
        let SelfKind::Type(Type::Named { arguments, .. }) = &vars.self_kind
        else {
            return;
        };
        if arguments.is_empty() {
            return;
        }
        for entry in &module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl { target, .. } = &item.kind else {
                continue;
            };
            if item.span.start > span.start || item.span.end < span.end {
                continue;
            }
            let TypeKind::Named {
                arguments: names, ..
            } = &module.types[target.0].kind
            else {
                continue;
            };
            for (id, ty) in names.iter().zip(arguments) {
                if let TypeKind::Named { name, arguments } =
                    &module.types[id.0].kind
                {
                    let name = super::last(name);
                    if arguments.is_empty()
                        && !vars.names.iter().any(|(n, _)| *n == name)
                    {
                        vars.names.push((name, ty.clone()));
                    }
                }
            }
            break;
        }
    }

    fn variable(&mut self, name: &str, vars: &mut Vars) -> Type {
        if let Some((_, ty)) = vars.names.iter().find(|(n, _)| n == name) {
            return ty.clone();
        }
        let ty = if vars.rigid {
            Type::Rigid(String::from(name))
        } else {
            self.fresh()
        };
        vars.names.push((String::from(name), ty.clone()));
        ty
    }

    fn self_type_of(&mut self, _arguments: Vec<Type>, vars: &Vars) -> Type {
        match &vars.self_kind {
            SelfKind::Type(ty) => ty.clone(),
            SelfKind::None => self.fresh(),
        }
    }

    /// A type name through its declaration's resolved targets. Only the
    /// resolved nominal head belongs to `home`; actual arguments and
    /// substituted variables already carry their own checked origins.
    fn name_resolved(
        &mut self,
        resolved: &Resolved,
        home: &Origin,
        id: TypeId,
        arguments: Vec<Type>,
        vars: &Vars,
    ) -> Type {
        match resolved.ty(id) {
            Some(Target::Type { origin, name }) => {
                let origin = if *origin == Origin::This {
                    home.clone()
                } else {
                    origin.clone()
                };
                Type::named(origin, name, arguments)
            }
            Some(Target::Variable(name)) => {
                vars.names.iter().find(|(n, _)| n == name).map_or_else(
                    || Type::Rigid(name.clone()),
                    |(_, ty)| ty.clone(),
                )
            }
            Some(Target::SelfType) => self.self_type_of(arguments, vars),
            Some(Target::Module(origin)) => module_marker(origin),
            _ => self.fresh(),
        }
    }

    fn name_declared(
        &mut self,
        module: &Module,
        origin: &Origin,
        name: &Name,
        arguments: Vec<Type>,
        vars: &mut Vars,
    ) -> Type {
        let first = name.segments[0].as_str();
        if name.segments.len() == 1 {
            if let Some((_, ty)) = vars.names.iter().find(|(n, _)| n == first) {
                return ty.clone();
            }
            if first == "Self" {
                return self.self_type_of(arguments, vars);
            }
            if self.type_item(module, first).is_some() {
                return Type::named(origin.clone(), first, arguments);
            }
        }
        let path = declared_path(module, name);
        let full = path.join(".");
        if self.declarations.get_module(&full).is_some() {
            return module_marker(&Origin::Host(full));
        }
        if let Some((name, parents)) =
            path.split_last().filter(|(_, parents)| !parents.is_empty())
        {
            let module = parents.join(".");
            return if self.files.iter().any(|(stem, _)| *stem == module) {
                Type::named(Origin::File(module), name, arguments)
            } else if self.declarations.get_module(&module).is_some() {
                Type::named(Origin::Host(module), name, arguments)
            } else {
                self.fresh()
            };
        }
        let declarations = self.declarations;
        if declarations.prelude.ty(first).is_some() {
            return Type::named(Origin::Prelude, first, arguments);
        }
        if declarations
            .commands
            .as_ref()
            .and_then(|c| c.ty(first))
            .is_some()
        {
            return Type::named(Origin::Commands, first, arguments);
        }
        if declarations.get_module(first).is_some() {
            return module_marker(&Origin::Host(String::from(first)));
        }
        self.variable(first, vars)
    }

    /// `A -> (b: B = v) -> C`: its spots and result.
    pub(super) fn signature(
        &mut self,
        scope: &Scope<'a>,
        id: TypeId,
        vars: &mut Vars,
    ) -> (Vec<Slot>, Type, Option<usize>) {
        let module = scope.module(self.module);
        let mut current = id;
        if let Some(TypeKind::Qualified { constraints, ty }) =
            module.ty(current).map(|node| &node.kind)
        {
            self.constraints(scope, constraints, vars);
            current = *ty;
        }
        let mut slots = Vec::new();
        let mut self_slot = None;
        while let Some(TypeKind::Function { parameter, result }) =
            module.ty(current).map(|node| &node.kind)
        {
            let (name, value, site) =
                match &module.ty(*parameter).map(|n| &n.kind) {
                    Some(TypeKind::Labeled { name, default, .. }) => {
                        (Some(super::last(name)), *default, name.span)
                    }
                    _ => (None, None, Span::default()),
                };
            if name.as_deref() == Some("self") {
                self_slot = Some(slots.len());
            }
            let ty = self.lower(scope, *parameter, vars);
            slots.push(Slot {
                place_kind: crate::typed::Place::Parameter(name.clone()),
                name,
                ty,
                default: value.is_some(),
                value,
                site,
            });
            current = *result;
        }
        let result = self.lower(scope, current, vars);
        (slots, result, self_slot)
    }

    /// A declared value of the prelude, the commands or a host module.
    pub(super) fn declared_value(
        &mut self,
        origin: &Origin,
        name: &str,
        span: Span,
    ) -> Callee {
        let (Some(namespace), Some(scope)) =
            (self.namespace(origin), self.scope(origin))
        else {
            return Callee::plain(self.fresh());
        };
        let Some(declared) = namespace.value(name) else {
            return Callee::plain(self.fresh());
        };
        match &declared.kind {
            Kind::Function { .. } | Kind::Value { .. } => {
                let Some(ty) = declare_item(&namespace.module, name) else {
                    return Callee::plain(self.fresh());
                };
                let mut vars = Vars::new(SelfKind::None);
                let (slots, result, self_slot) =
                    self.signature(&scope, ty, &mut vars);
                // `to<T>` and `try<T>` name their target first.
                if *origin == Origin::Prelude && matches!(name, "to" | "try") {
                    if let Some(index) =
                        vars.names.iter().position(|(n, _)| n == "b")
                    {
                        let target = vars.names.remove(index);
                        vars.names.insert(0, target);
                    }
                }
                let mut callee =
                    self.finish_callee(slots, result, self_slot, vars, span);
                callee.home = Some(origin.clone());
                callee.selected = Some(crate::typed::Selected::Declared {
                    origin: origin.clone(),
                    site: declared.span,
                });
                if *origin == Origin::Prelude && matches!(name, "to" | "try") {
                    let target = callee.parameters[0].clone();
                    callee.conversion = Some((target, String::from(name)));
                }
                callee
            }
            Kind::Binding { annotation } => {
                self.declared_binding(origin, name, *annotation, span)
            }
            Kind::Type { .. } => Callee::plain(self.fresh()),
        }
    }

    /// Use the current checked group before consulting a raw declared cache.
    fn local_declared_binding(
        &mut self,
        home: &Origin,
        name: &str,
        span: Span,
    ) -> Option<Callee> {
        if self.declared_home.as_ref() == Some(home) {
            if let Some(index) =
                self.item_index(&super::Key::Binding(name.into()))
            {
                if matches!(self.state.status[index], super::Status::Typing) {
                    self.error(
                        span,
                        super::Reason::DeclarationValue {
                            name: name.into(),
                            origin: home.clone(),
                            declaration: self.items[index].site,
                        },
                    );
                    return Some(Callee::plain(self.fresh()));
                }
                return Some(self.item_callee(index, span));
            }
        }
        None
    }

    fn declared_binding(
        &mut self,
        home: &Origin,
        name: &str,
        annotation: Option<TypeId>,
        span: Span,
    ) -> Callee {
        use super::files::DeclaredValue;
        if let Some(callee) = self.local_declared_binding(home, name, span) {
            return callee;
        }
        let key = (home.clone(), String::from(name));
        let Some(module) = self.owner(home) else {
            return Callee::plain(self.fresh());
        };
        let binding = module.entries.iter().rev().find_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let crate::ast::ExprKind::Bind { pattern, value, .. } = module.expressions[id.0].kind else { return None };
            matches!(&module.patterns[pattern.0].kind, crate::ast::PatternKind::Name(n) if super::last(n) == name).then_some((value, module.patterns[pattern.0].span))
        });
        let Some((value, site)) = binding else {
            return Callee::plain(self.fresh());
        };
        let cached = self.file_cache.values.borrow().get(&key).cloned();
        match cached {
            Some(DeclaredValue::Typed(scheme)) => {
                return self.declared_reference(
                    home,
                    (name, site, value, annotation),
                    &scheme,
                    span,
                )
            }
            Some(DeclaredValue::Failed) => return Callee::plain(self.fresh()),
            Some(DeclaredValue::Checking) => {
                self.error(
                    span,
                    super::Reason::DeclarationValue {
                        name: name.into(),
                        origin: home.clone(),
                        declaration: span,
                    },
                );
                return Callee::plain(self.fresh());
            }
            None => {}
        }
        let ty = match annotation {
            Some(id) => self.lower(
                &Scope::Declared {
                    module,
                    origin: home.clone(),
                },
                id,
                &mut Vars::new(SelfKind::None),
            ),
            None => self.fresh(),
        };
        self.file_cache
            .values
            .borrow_mut()
            .insert(key.clone(), DeclaredValue::Checking);
        let slot = Slot {
            place_kind: crate::typed::Place::Binding(name.into()),
            name: Some(name.into()),
            ty,
            default: false,
            value: Some(value),
            site,
        };
        let (scheme, valid) =
            self.declared_expression(home, &slot, &[], span, false);
        let valid = valid.is_ok();
        let cached = if valid {
            DeclaredValue::Typed(scheme.clone())
        } else {
            DeclaredValue::Failed
        };
        self.file_cache.values.borrow_mut().insert(key, cached);
        if !valid {
            self.error(
                span,
                super::Reason::DeclarationValue {
                    name: name.into(),
                    origin: home.clone(),
                    declaration: site,
                },
            );
        }
        self.declared_reference(
            home,
            (name, site, value, annotation),
            &scheme,
            span,
        )
    }

    fn declared_reference(
        &mut self,
        home: &Origin,
        binding: (&str, Span, crate::ast::ExprId, Option<TypeId>),
        scheme: &crate::types::Scheme,
        span: Span,
    ) -> Callee {
        let (name, site, value, annotation) = binding;
        let (ty, meanings) = self.instantiate_with_meanings(scheme, span);
        let mut callee = Callee::plain(ty);
        if let (Some(annotation), Some(module)) = (annotation, self.owner(home))
        {
            if let crate::ast::ExprKind::Arms(arms) =
                &module.expressions[value.0].kind
            {
                let heads = super::arms::heads_in(module, arms);
                callee = self.with_heads(callee.whole(), Some(&heads));
            }
            self.overlay_declared_signature(
                &mut callee,
                module,
                annotation,
                home,
                (name, site),
                span,
            );
        }
        callee.meanings = meanings;
        callee.selected = Some(crate::typed::Selected::Value {
            origin: home.clone(),
            site,
        });
        callee
    }

    pub(super) fn finish_callee(
        &mut self,
        slots: Vec<Slot>,
        result: Type,
        self_slot: Option<usize>,
        vars: Vars,
        span: Span,
    ) -> Callee {
        for mut obligation in vars.obligations {
            let (Obligation::Member { dot, .. }
            | Obligation::Claim { dot, .. }) = &mut obligation;
            *dot = span;
            self.push_owed(super::Owed::new(obligation, span));
        }

        let mut callee = Callee::plain(result);
        callee.parameters = vars.names.into_iter().map(|(_, ty)| ty).collect();
        if !slots.is_empty() {
            callee.spots = Some(slots);
            callee.self_slot = self_slot;
        }
        callee
    }

    /// The constructor `name` of type `ty`: a sum constructor's fields,
    /// a record's written constructor, or its fields, as the spot list.
    pub(super) fn constructor(
        &mut self,
        origin: &Origin,
        ty: &str,
        name: &str,
    ) -> Callee {
        self.constructor_kind(origin, ty, name, false)
    }

    pub(super) fn constructor_kind(
        &mut self,
        origin: &Origin,
        ty: &str,
        name: &str,
        raw: bool,
    ) -> Callee {
        self.constructor_fields_kind(origin, ty, name, raw, false)
    }

    pub(super) fn constructor_fields_kind(
        &mut self,
        origin: &Origin,
        ty: &str,
        name: &str,
        raw: bool,
        private: bool,
    ) -> Callee {
        let (Some(module), Some(scope)) =
            (self.owner(origin), self.scope(origin))
        else {
            return Callee::plain(self.fresh());
        };
        let Some((parameters, members)) = self.type_item(module, ty) else {
            return Callee::plain(self.fresh());
        };
        let mut vars = Vars::new(SelfKind::None);
        for parameter in parameters {
            let fresh = self.fresh();
            vars.names.push((super::last(&parameter.name), fresh));
        }
        let arguments: Vec<Type> =
            vars.names.iter().map(|(_, t)| t.clone()).collect();
        let result = Type::named(origin.clone(), ty, arguments.clone());
        vars.self_kind = SelfKind::Type(result.clone());
        let members = members.map_or(&[][..], Vec::as_slice);
        let sum = members.iter().find_map(|member| match member {
            Member::Constructor {
                name: c, fields, ..
            } if super::last(c) == name => Some(fields),
            _ => None,
        });
        let inside = private
            || core::ptr::eq(module, self.module)
            || matches!(origin, Origin::This | Origin::Line(_));
        let fields: Vec<&crate::ast::Field> = if let Some(fields) = sum {
            fields.iter().collect()
        } else {
            if !raw {
                if let Some(callee) = self.written(origin, ty, module) {
                    return callee;
                }
            }
            members
                .iter()
                .filter_map(|member| match member {
                    Member::Field(field) => Some(field),
                    Member::Constructor { .. } => None,
                })
                .collect()
        };
        let mut slots = Vec::new();
        let mut sealed = None;
        for field in fields {
            let field_ty = self.lower(&scope, field.ty, &mut vars);
            let default = field.default.is_some()
                || (sum.is_none() && self.default_available(&field_ty));
            // A private field outside its module is no spot: it takes
            // its default, and without one the derived constructor does
            // not exist there.
            if sum.is_none() && !inside && !field.public {
                if !default && sealed.is_none() {
                    sealed = Some(super::last(&field.name));
                }
                continue;
            }
            slots.push(Slot {
                place_kind: if sum.is_some() {
                    crate::typed::Place::Payload
                } else {
                    crate::typed::Place::Field {
                        owner: ty.into(),
                        name: super::last(&field.name),
                    }
                },
                name: Some(super::last(&field.name)),
                ty: field_ty,
                default,
                value: field.default,
                site: field.name.span,
            });
        }
        self.parameter_claims(&result, &mut vars, Span::default());
        for obligation in vars.obligations {
            self.push_owed(super::Owed::new(obligation, Span::default()));
        }
        let mut callee = Callee::plain(result.clone());
        callee.selected = Some(crate::typed::Selected::Constructor {
            ty: result,
            name: String::from(name),
        });
        callee.spots = Some(slots);
        callee.parameters = arguments;
        callee.home = Some(origin.clone()).filter(|o| *o != Origin::This);
        callee.sealed = sealed;
        callee
    }

    /// A written constructor's checked definition in its owning source.
    fn written(
        &mut self,
        origin: &Origin,
        ty: &str,
        module: &'a Module,
    ) -> Option<Callee> {
        let key = super::Key::Constructor(String::from(ty));
        if *origin == Origin::This {
            let index = self.item_index(&key)?;
            return Some(self.item_callee(index, Span::default()));
        }
        if let Origin::Line(line) = origin {
            // Written here, or by an accepted line.
            if let Some(index) = self.item_index(&key) {
                return Some(self.item_callee(index, Span::default()));
            }
            let key = (*line, String::from(ty));
            let (start, end) = *self.frame?.constructors.get(&key)?;
            return Some(self.local(Span { start, end }, Span::default()));
        }
        let arms: Vec<_> = impls(module, ty)
            .flatten()
            .filter_map(|member| match member {
                ImplMember::Constructor { arm, .. } => Some(arm.clone()),
                ImplMember::Named { .. } => None,
            })
            .collect();
        if arms.is_empty() {
            return None;
        }
        let site = arms[0].span;
        if core::ptr::eq(module, self.module) {
            if let Some(index) = self.item_index(&key) {
                return Some(self.item_callee(index, site));
            }
        }
        let local = if let Origin::File(stem) = origin {
            let Some(checked) = self.checked_file(stem) else {
                self.constructor_error(origin, ty, site);
                return Some(Callee::plain(self.fresh()));
            };
            if !checked.resolve_errors.is_empty()
                || !checked.type_errors.is_empty()
            {
                self.constructor_error(origin, ty, site);
                return Some(Callee::plain(self.fresh()));
            }
            let Some((_, local)) = checked
                .typed
                .frozen
                .iter()
                .find(|(at, _)| *at == (site.start, site.end))
            else {
                self.constructor_error(origin, ty, site);
                return Some(Callee::plain(self.fresh()));
            };
            let mut local = local.clone();
            local.scheme = local.scheme.rehome(origin);
            local
        } else {
            let Some(local) =
                self.supplied_constructor(origin, ty, module, site)
            else {
                return Some(Callee::plain(self.fresh()));
            };
            local
        };
        let (ty, meanings) =
            self.instantiate_with_meanings(&local.scheme, self.module.span);
        let mut callee = self.with_heads(ty, local.heads.as_deref());
        callee.meanings = meanings;
        callee.home = Some(origin.clone());
        callee.selected = Some(crate::typed::Selected::Value {
            origin: origin.clone(),
            site,
        });
        Some(callee)
    }

    /// The fields of a record type, lowered at its arguments; `None` for
    /// a sum, an opaque type or anything else.
    pub(super) fn fields(&mut self, ty: &Type) -> Option<Vec<Field>> {
        let Type::Named { name, arguments } = ty else {
            return None;
        };
        let module = self.owner(&name.origin)?;
        let scope = self.scope(&name.origin)?;
        let (parameters, members) = self.type_item(module, &name.name)?;
        let members = members?;
        if members
            .iter()
            .any(|m| matches!(m, Member::Constructor { .. }))
        {
            return None;
        }
        let mut vars = Vars::new(SelfKind::Type(ty.clone()));
        for (parameter, argument) in parameters.iter().zip(arguments) {
            vars.names
                .push((super::last(&parameter.name), argument.clone()));
        }
        let mut fields = Vec::new();
        for member in members {
            if let Member::Field(field) = member {
                let ty = self.lower(&scope, field.ty, &mut vars);
                let default =
                    field.default.is_some() || self.default_available(&ty);
                fields.push(Field {
                    name: super::last(&field.name),
                    ty,
                    public: field.public
                        || core::ptr::eq(module, self.module)
                        || matches!(
                            name.origin,
                            Origin::This | Origin::Line(_)
                        ),
                    default,
                });
            }
        }
        Some(fields)
    }

    /// A sum constructor's fields with the constructor's result type.
    pub(super) fn constructor_fields(
        &mut self,
        origin: &Origin,
        ty: &str,
        name: &str,
    ) -> (Type, Vec<Field>) {
        let callee = self.constructor(origin, ty, name);
        let fields = callee
            .spots
            .unwrap_or_default()
            .into_iter()
            .map(|slot| Field {
                name: slot.name.unwrap_or_default(),
                ty: slot.ty,
                public: true,
                default: slot.default,
            })
            .collect();
        (callee.ty, fields)
    }

    /// Record field defaults, checked against their field's type with
    /// the fields before them in scope.
    pub(super) fn field_defaults(
        &mut self,
        name: &Name,
        members: Option<&[Member]>,
    ) {
        let ty = Type::named(Origin::This, &super::last(name), Vec::new());
        self.self_type = Some(ty);
        for member in members.unwrap_or_default() {
            if let Member::Field(field) = member {
                let field_ty = self.lower_here(field.ty);
                if let Some(default) = field.default {
                    let expected =
                        super::Expected::new(field_ty.clone(), field.name.span)
                            .at(crate::typed::Place::Field {
                                owner: super::last(name),
                                name: super::last(&field.name),
                            });
                    self.expr(default, Some(&expected));
                }
                self.bind_local(&field.name, field_ty);
            }
        }
        self.self_type = None;
    }

    /// `Self`, or a type name in value position: its record constructor.
    pub(super) fn self_constructor(&mut self) -> Callee {
        match self.self_type.clone() {
            Some(Type::Named { name, .. }) => {
                self.constructor(&name.origin, &name.name, &name.name)
            }
            _ => Callee::plain(self.fresh()),
        }
    }
}

/// `Rd<fs>`: the API reference a host module's name stands for.
fn module_marker(origin: &Origin) -> Type {
    let name = match origin {
        Origin::Host(name) | Origin::File(name) => name.as_str(),
        _ => "",
    };
    Type::named(origin.clone(), name, Vec::new())
}

/// The last segment of a named type syntax.
pub(super) fn named(module: &Module, id: TypeId) -> Option<String> {
    match &module.ty(id)?.kind {
        TypeKind::Named { name, .. } => Some(super::last(name)),
        _ => None,
    }
}

impl Infer<'_> {
    pub(super) fn type_item<'m>(
        &self,
        module: &'m Module,
        name: &str,
    ) -> Option<(&'m [TypeParameter], Option<&'m Vec<Member>>)> {
        self.type_entries(module, name).into_iter().find_map(
            |entry| match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Type {
                        name: n,
                        parameters,
                        members,
                        ..
                    } if super::last(n) == name => {
                        Some((parameters.as_slice(), members.as_ref()))
                    }
                    _ => None,
                },
                Entry::Line(_) => None,
            },
        )
    }
}

fn declare_item(module: &Module, name: &str) -> Option<TypeId> {
    module.entries.iter().find_map(|entry| match entry {
        Entry::Item(item) => match &item.kind {
            ItemKind::Declare { name: n, ty } if super::last(n) == name => {
                Some(*ty)
            }
            _ => None,
        },
        Entry::Line(_) => None,
    })
}

/// The members of every impl of type `ty` in `module`.
pub(super) fn impls<'m>(
    module: &'m Module,
    ty: &'m str,
) -> impl Iterator<Item = &'m [ImplMember]> + 'm {
    module.entries.iter().filter_map(move |entry| match entry {
        Entry::Item(item) => match &item.kind {
            ItemKind::Impl {
                target, members, ..
            } if named(module, *target).as_deref() == Some(ty) => {
                Some(members.as_slice())
            }
            _ => None,
        },
        Entry::Line(_) => None,
    })
}

impl<'a> Infer<'a> {
    fn lower_name(
        &mut self,
        scope: &Scope<'a>,
        id: TypeId,
        vars: &mut Vars,
    ) -> Type {
        let module = scope.module(self.module);
        let node = &module.types[id.0];
        let TypeKind::Named { name, arguments } = &node.kind else {
            unreachable!("named type")
        };
        let head = self.lower_target(scope, id, Vec::new(), vars);
        let parameters = if let Type::Named { name, .. } = &head {
            self.owner(&name.origin)
                .and_then(|m| self.type_item(m, &name.name))
                .map(|(p, _)| p)
        } else {
            None
        };
        let variable = vars.names.iter().any(|(n, _)| n == &super::last(name))
            || matches!(head, Type::Var(_) | Type::Rigid(_));
        let impl_head = module.entries.iter().any(|entry| matches!(entry, Entry::Item(item) if matches!(item.kind, ItemKind::Impl { target, .. } if target == id)));
        if !impl_head
            && !variable
            && parameters.is_some_and(|ps| {
                arguments.len()
                    < ps.iter().filter(|p| p.default.is_none()).count()
            })
            && super::last(name) != "Self"
        {
            self.error(
                node.span,
                super::Reason::Limit {
                    message: String::from("missing type arguments"),
                },
            );
        }
        if super::last(name) == "Self" && !arguments.is_empty() {
            self.error(
                node.span,
                super::Reason::Limit {
                    message: String::from("Self cannot take type arguments"),
                },
            );
        }
        if variable {
            self.variable_kind(&super::last(name), false, vars, node.span);
            if !arguments.is_empty() {
                self.error(
                    node.span,
                    super::Reason::Limit {
                        message: String::from(
                            "a parameter cannot take type arguments",
                        ),
                    },
                );
            }
        }
        let mut arguments: Vec<_> = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let number = parameters
                    .and_then(|p| p.get(index))
                    .is_some_and(|p| !p.name.is_uppercase());
                if number {
                    self.lower_number(scope, *argument, vars)
                } else {
                    let ty = self.lower(scope, *argument, vars);
                    if matches!(ty, Type::Number(_) | Type::Sum(_)) {
                        self.error(
                            module.types[argument.0].span,
                            super::Reason::Limit {
                                message: String::from(
                                    "expected a type argument, found a number",
                                ),
                            },
                        );
                    }
                    ty
                }
            })
            .collect();
        if impl_head && arguments.is_empty() {
            if let Some(parameters) = parameters {
                arguments.extend(
                    parameters
                        .iter()
                        .map(|p| self.variable(&super::last(&p.name), vars)),
                );
            }
        }
        let ty = self.lower_target(scope, id, arguments, vars);
        self.check_arity(&ty, node.span);
        self.parameter_claims(&ty, vars, node.span);
        self.limited(ty)
    }
    /// Locate the checked dependency use, never report a foreign AST byte offset
    /// as though it belonged to this source. A synthetic dependency uses the unit.
    fn resource_reference_span(
        &self,
        origin: &Origin,
        module: &Module,
        written: Span,
    ) -> Span {
        if core::ptr::eq(module, self.module) {
            return written;
        }
        let owns = |target: &Target| match target {
            Target::Type { origin: known, .. }
            | Target::Value { origin: known, .. }
            | Target::Constructor { origin: known, .. }
            | Target::Module(known) => known == origin,
            _ => false,
        };
        self.module
            .types
            .iter()
            .enumerate()
            .find_map(|(id, ty)| {
                self.resolved
                    .ty(TypeId(id))
                    .filter(|target| owns(target))
                    .map(|_| ty.span)
            })
            .or_else(|| {
                self.resolved.expressions().find_map(|(id, target)| {
                    owns(target).then(|| self.module.expressions[id.0].span)
                })
            })
            .unwrap_or(self.module.span)
    }

    fn check_arity(&mut self, ty: &Type, span: Span) {
        if let Type::Named { name, arguments } = ty {
            if self
                .owner(&name.origin)
                .and_then(|m| self.type_item(m, &name.name))
                .is_some_and(|(parameters, _)| {
                    arguments.len() > parameters.len()
                })
            {
                self.error(
                    span,
                    super::Reason::ClaimBlock {
                        claim: ty.clone(),
                        problem: String::from("has too many type arguments"),
                    },
                );
            }
        }
    }

    fn lower_target(
        &mut self,
        scope: &Scope<'a>,
        id: TypeId,
        arguments: Vec<Type>,
        vars: &mut Vars,
    ) -> Type {
        let module = scope.module(self.module);
        let TypeKind::Named { name, .. } = &module.types[id.0].kind else {
            unreachable!()
        };
        if let Scope::Declared { module, origin } = scope {
            let own = match origin {
                Origin::File(stem) if module.resource_declarations() => {
                    Some(stem.clone())
                }
                _ => None,
            };
            let first = name.segments[0].as_str();
            let local = name.segments.len() == 1
                && (first == "Self"
                    || vars.names.iter().any(|(known, _)| known == first)
                    || self.type_item(module, first).is_some());
            let forbidden = own.or_else(|| {
                if local {
                    return None;
                }
                let path = declared_path(module, name);
                // Match name_declared: a full Host module is an API marker first.
                if self.declarations.get_module(&path.join(".")).is_some() {
                    return None;
                }
                let (_, parents) = path.split_last()?;
                let stem = parents.join(".");
                self.files
                    .iter()
                    .find(|(file, module)| {
                        *file == stem && module.resource_declarations()
                    })
                    .map(|_| stem)
            });
            if let Some(stem) = forbidden {
                let at =
                    self.resource_reference_span(origin, module, name.span);
                self.error(at, super::Reason::ResourceInFile { module: stem });
                return self.fresh();
            }
        }
        match scope {
            Scope::This => {
                let resolved = self.resolved;
                self.name_resolved(resolved, &Origin::This, id, arguments, vars)
            }
            Scope::Line {
                resolved, index, ..
            } => self.name_resolved(
                resolved,
                &Origin::Line(*index),
                id,
                arguments,
                vars,
            ),
            Scope::Declared { module, origin } => {
                let cached = if let Origin::File(stem) = origin {
                    self.cached_file(stem)
                } else {
                    None
                };
                if let Some(checked) = cached {
                    self.name_resolved(
                        &checked.resolved,
                        origin,
                        id,
                        arguments,
                        vars,
                    )
                } else {
                    self.name_declared(module, origin, name, arguments, vars)
                }
            }
        }
    }

    fn variable_kind(
        &mut self,
        name: &str,
        number: bool,
        vars: &mut Vars,
        span: Span,
    ) {
        if vars
            .kinds
            .iter()
            .any(|(n, kind)| n == name && *kind != number)
        {
            self.error(
                span,
                super::Reason::Limit {
                    message: alloc::format!(
                        "{name} cannot be both a type and a number variable"
                    ),
                },
            );
        } else if !vars.kinds.iter().any(|(n, _)| n == name) {
            vars.kinds.push((String::from(name), number));
        }
    }

    fn lower_number(
        &mut self,
        scope: &Scope<'a>,
        id: TypeId,
        vars: &mut Vars,
    ) -> Type {
        let node = &scope.module(self.module).types[id.0];
        match &node.kind {
            TypeKind::Number(n) => Type::Number(n.clone()),
            TypeKind::Sum(parts) => Type::sum(
                parts
                    .iter()
                    .map(|p| self.lower_number(scope, *p, vars))
                    .collect(),
            ),
            TypeKind::Named { name, arguments }
                if !name.is_uppercase()
                    && name.segments.len() == 1
                    && arguments.is_empty() =>
            {
                let name = super::last(name);
                self.variable_kind(&name, true, vars, node.span);
                self.variable(&name, vars)
            }
            _ => {
                self.error(
                    node.span,
                    super::Reason::Limit {
                        message: String::from("expected a number argument"),
                    },
                );
                Type::Number(0.into())
            }
        }
    }
}
