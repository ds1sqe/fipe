//! Declared types into [`Type`]s. The prelude, the commands and host
//! modules, and file modules are parsed but not resolved, so their type
//! names are looked up here: the file's own types, then the prelude,
//! then the commands. The module being inferred uses its resolved
//! targets instead.
//!
//! A declaration's lowercase names are its quantified variables, fresh
//! at each lowering, so lowering a reference is instantiation (VAR).
//! `Self<a>` in a trait method is the one higher-kinded form: it lowers
//! to a placeholder the application replaces with the receiver's family
//! (`spots.rs`). This stands in for 06's kinds.

use super::{Callee, Infer, Slot};
use crate::{
    ast::{
        Constraint, ConstraintKind, Entry, ImplMember, ItemKind, Member,
        Module, Name, TraitMember, TypeId, TypeKind, TypeParameter,
    },
    decl::{Kind, Namespace},
    resolve::{Origin, Resolved, Target},
    token::Span,
    types::{Obligation, Type},
};
use alloc::{string::String, vec::Vec};

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
    /// Inside a trait: this variable, or the placeholder when applied.
    Trait(Type),
}

/// The names bound while lowering one declaration.
pub(super) struct Vars {
    /// Type parameters and quantified variables, in order of binding.
    pub names: Vec<(String, Type)>,
    pub self_kind: SelfKind,
    pub obligations: Vec<Obligation>,
}

impl Vars {
    pub const fn new(self_kind: SelfKind) -> Self {
        Self {
            names: Vec::new(),
            self_kind,
            obligations: Vec::new(),
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

/// The placeholder for `Self<…>` in a higher-kinded trait method.
pub(super) fn family_placeholder(arguments: Vec<Type>) -> Type {
    Type::named(Origin::Prelude, "Self", arguments)
}

pub(super) fn is_placeholder(ty: &Type) -> bool {
    super::is_prelude(ty, "Self")
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
        self.lower(&Scope::This, id, &mut Vars::new(self_kind))
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
        match &node.kind {
            TypeKind::Unit => Type::Unit,
            TypeKind::Named { name, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower(scope, *argument, vars))
                    .collect();
                match scope {
                    Scope::This => {
                        let resolved = self.resolved;
                        self.name_resolved(resolved, None, id, arguments, vars)
                    }
                    Scope::Line {
                        resolved, index, ..
                    } => self.name_resolved(
                        resolved,
                        Some(*index),
                        id,
                        arguments,
                        vars,
                    ),
                    Scope::Declared { module, origin } => self
                        .name_declared(module, origin, name, arguments, vars),
                }
            }
            TypeKind::Array { element, length } => {
                let element = self.lower(scope, *element, vars);
                match module.ty(*length).map(|node| &node.kind) {
                    Some(TypeKind::Number(value)) => {
                        if let Some(length) = value.to_u64() {
                            return Type::Array {
                                element: alloc::boxed::Box::new(element),
                                length,
                            };
                        }
                        // Another module reports its own.
                        if matches!(scope, Scope::This) {
                            let span = module.types[length.0].span;
                            self.error(span, super::Reason::OutOfRange);
                        }
                        self.fresh()
                    }
                    // A number parameter, which resolve reports.
                    _ => self.fresh(),
                }
            }
            // Forms resolve reports as not yet checked.
            TypeKind::Number(_)
            | TypeKind::Tuple(_)
            | TypeKind::Both(_)
            | TypeKind::Either(_)
            | TypeKind::Not(_) => self.fresh(),
            TypeKind::Function { parameter, result } => {
                let parameter = self.lower(scope, *parameter, vars);
                Type::function(parameter, self.lower(scope, *result, vars))
            }
            TypeKind::Qualified { constraints, ty } => {
                self.constraints(module, constraints, vars);
                self.lower(scope, *ty, vars)
            }
            TypeKind::Labeled { ty, .. } => self.lower(scope, *ty, vars),
        }
    }

    /// Named constraints as obligations; a member constraint is not yet
    /// checked, and reported where it is written.
    fn constraints(
        &mut self,
        module: &Module,
        constraints: &[Constraint],
        vars: &mut Vars,
    ) {
        for constraint in constraints {
            if let ConstraintKind::Named { class, variable } = &constraint.kind
            {
                let class = named(module, *class).unwrap_or_default();
                let ty = self.variable(&super::last(variable), vars);
                vars.obligations.push(Obligation::Class { class, ty });
            }
        }
    }

    fn variable(&mut self, name: &str, vars: &mut Vars) -> Type {
        if let Some((_, ty)) = vars.names.iter().find(|(n, _)| n == name) {
            return ty.clone();
        }
        let ty = self.fresh();
        vars.names.push((String::from(name), ty.clone()));
        ty
    }

    fn self_type_of(&mut self, arguments: Vec<Type>, vars: &Vars) -> Type {
        match &vars.self_kind {
            SelfKind::Type(ty) => ty.clone(),
            SelfKind::Trait(ty) if arguments.is_empty() => ty.clone(),
            SelfKind::Trait(_) => family_placeholder(arguments),
            SelfKind::None => self.fresh(),
        }
    }

    /// A type name through a module's resolved targets; an accepted
    /// line's own declarations are named by its `index`.
    fn name_resolved(
        &mut self,
        resolved: &Resolved,
        index: Option<usize>,
        id: TypeId,
        arguments: Vec<Type>,
        vars: &Vars,
    ) -> Type {
        match resolved.ty(id) {
            Some(
                Target::Type { origin, name } | Target::Trait { origin, name },
            ) => {
                let origin = match (origin, index) {
                    (Origin::This, Some(index)) => Origin::Line(index),
                    _ => origin.clone(),
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
        if name.segments.len() == 2 {
            // `fs.Error`; `Self.Error` is an associated type, unknown here.
            return if self.declarations.get_module(first).is_some() {
                let host = Origin::Host(String::from(first));
                Type::named(host, &name.segments[1], arguments)
            } else {
                self.fresh()
            };
        }
        if let Some((_, ty)) = vars.names.iter().find(|(n, _)| n == first) {
            return ty.clone();
        }
        if first == "Self" {
            return self.self_type_of(arguments, vars);
        }
        if type_item(module, first).is_some() {
            return Type::named(origin.clone(), first, arguments);
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
            self.constraints(module, constraints, vars);
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
            let bare_self =
                named(module, *parameter).as_deref() == Some("Self");
            if self_slot.is_none()
                && (name.as_deref() == Some("self") || bare_self)
            {
                self_slot = Some(slots.len());
            }
            let ty = self.lower(scope, *parameter, vars);
            slots.push(Slot {
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
                    vars.names.reverse();
                }
                let mut callee =
                    self.finish_callee(slots, result, self_slot, vars, span);
                callee.home = Some(origin.clone());
                callee
            }
            Kind::Method { class, .. } => {
                let var = self.fresh();
                let mut callee = self.method(
                    &scope,
                    class,
                    name,
                    &SelfKind::Trait(var),
                    None,
                    span,
                );
                if callee.spots.is_none()
                    && super::lower::has_placeholder(&callee.ty)
                {
                    // `empty: Self<a>`: a value whose family the expected
                    // type must give.
                    callee.spots = Some(Vec::new());
                }
                callee
            }
            Kind::Binding {
                annotation: Some(annotation),
            } => {
                let mut vars = Vars::new(SelfKind::None);
                Callee::plain(self.lower(&scope, *annotation, &mut vars))
            }
            _ => Callee::plain(self.fresh()),
        }
    }

    fn finish_callee(
        &mut self,
        slots: Vec<Slot>,
        result: Type,
        self_slot: Option<usize>,
        vars: Vars,
        span: Span,
    ) -> Callee {
        for obligation in vars.obligations {
            self.state
                .obligations
                .push(super::Owed::new(obligation, span));
        }

        let mut callee = Callee::plain(result);
        callee.parameters = vars.names.into_iter().map(|(_, ty)| ty).collect();
        if !slots.is_empty() {
            callee.spots = Some(slots);
            callee.self_slot = self_slot;
        }
        callee
    }

    /// Trait `class`'s method `member`, declared in `scope`, with `Self`
    /// as given and the trait's parameters bound to `arguments` or fresh.
    /// A bare `Self` adds the obligation `class Self`.
    pub(super) fn method(
        &mut self,
        scope: &Scope<'a>,
        class: &str,
        member: &str,
        self_kind: &SelfKind,
        arguments: Option<&[Type]>,
        span: Span,
    ) -> Callee {
        let module = scope.module(self.module);
        let Some((parameters, members)) = trait_item(module, class) else {
            return Callee::plain(self.fresh());
        };
        let Some(ty) = members
            .iter()
            .find(|m| super::last(&m.name) == member)
            .map(|m| m.ty)
        else {
            return Callee::plain(self.fresh());
        };
        let mut vars = Vars::new(self_kind.clone());
        for (index, parameter) in parameters.iter().enumerate() {
            let ty = arguments
                .and_then(|a| a.get(index))
                .map_or_else(|| self.fresh(), Clone::clone);
            vars.names.push((super::last(&parameter.name), ty));
        }
        let (slots, result, self_slot) = self.signature(scope, ty, &mut vars);
        let SelfKind::Trait(var) = self_kind.clone() else {
            return self.finish_callee(slots, result, self_slot, vars, span);
        };
        let whole = slots.iter().fold(result.clone(), |ty, slot| {
            Type::function(slot.ty.clone(), ty)
        });
        if !has_placeholder(&whole) {
            vars.obligations.push(Obligation::Class {
                class: String::from(class),
                ty: var.clone(),
            });
        }
        // `read<I64>`: the receiver is the first type parameter.
        let mut callee =
            self.finish_callee(slots, result, self_slot, vars, span);
        callee.parameters.insert(0, var);
        callee
    }

    /// The trait method an impl member of `self_type` implements, from
    /// the impl's `implements` type in the module being inferred.
    pub(super) fn impl_method(
        &mut self,
        implements: TypeId,
        member: &str,
    ) -> Type {
        let Some(class) = named(self.module, implements) else {
            return self.fresh();
        };
        let arguments = match self.module.ty(implements).map(|n| &n.kind) {
            Some(TypeKind::Named { arguments, .. }) => arguments.clone(),
            _ => Vec::new(),
        };
        let arguments: Vec<Type> =
            arguments.iter().map(|a| self.lower_here(*a)).collect();
        let origin = match self.resolved.ty(implements) {
            Some(Target::Trait { origin, .. }) => origin.clone(),
            _ => Origin::Prelude,
        };
        let self_kind = self
            .self_type
            .clone()
            .map_or(SelfKind::None, SelfKind::Type);
        let Some(scope) = self.scope(&origin) else {
            return self.fresh();
        };
        let span = self.module.types[implements.0].span;
        self.method(&scope, &class, member, &self_kind, Some(&arguments), span)
            .whole()
    }

    /// The constructor `name` of type `ty`: a sum constructor's fields,
    /// a record's written constructor, or its fields, as the spot list.
    pub(super) fn constructor(
        &mut self,
        origin: &Origin,
        ty: &str,
        name: &str,
    ) -> Callee {
        let (Some(module), Some(scope)) =
            (self.owner(origin), self.scope(origin))
        else {
            return Callee::plain(self.fresh());
        };
        let Some((parameters, members)) = type_item(module, ty) else {
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
        let fields: Vec<&crate::ast::Field> = if let Some(fields) = sum {
            fields.iter().collect()
        } else {
            if let Some(callee) = self.written(origin, ty, module, &result) {
                return callee;
            }
            members
                .iter()
                .filter_map(|member| match member {
                    Member::Field(field) => Some(field),
                    Member::Constructor { .. } => None,
                })
                // A private field outside its module is no spot.
                .filter(|f| {
                    f.public || matches!(origin, Origin::This | Origin::Line(_))
                })
                .collect()
        };
        let mut slots = Vec::new();
        for field in fields {
            slots.push(Slot {
                name: Some(super::last(&field.name)),
                ty: self.lower(&scope, field.ty, &mut vars),
                default: field.default.is_some(),
                value: field.default,
                site: field.name.span,
            });
        }
        let mut callee = Callee::plain(result);
        callee.spots = Some(slots);
        callee.parameters = arguments;
        callee.home = Some(origin.clone()).filter(|o| *o != Origin::This);
        callee
    }

    /// A record's written constructor: its inferred type in this module,
    /// its head's names with fresh types elsewhere.
    fn written(
        &mut self,
        origin: &Origin,
        ty: &str,
        module: &'a Module,
        result: &Type,
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
            .flat_map(|(_, members)| members)
            .filter_map(|member| match member {
                ImplMember::Constructor(arm) => Some(arm.clone()),
                _ => None,
            })
            .collect();
        if arms.is_empty() {
            return None;
        }
        let slots = super::arms::heads_in(module, &arms)
            .into_iter()
            .map(|head| Slot {
                name: head.name,
                ty: Type::Var(self.state.subst.fresh()),
                default: head.default,
                value: None,
                site: Span::default(),
            })
            .collect();
        let mut callee = Callee::plain(result.clone());
        callee.spots = Some(slots);
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
        let (parameters, members) = type_item(module, &name.name)?;
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
                fields.push(Field {
                    name: super::last(&field.name),
                    ty: self.lower(&scope, field.ty, &mut vars),
                    public: field.public
                        || matches!(
                            name.origin,
                            Origin::This | Origin::Line(_)
                        ),
                    default: field.default.is_some(),
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

    /// Whether a host-declared type implements `Read` in its namespace.
    pub(super) fn reads(&self, ty: &Type) -> bool {
        let Type::Named { name, .. } = ty else {
            return false;
        };
        if !matches!(name.origin, Origin::Commands | Origin::Host(_)) {
            return false;
        }
        self.owner(&name.origin).is_some_and(|module| {
            impls(module, &name.name).any(|(implements, _)| {
                implements.and_then(|id| named(module, id)).as_deref()
                    == Some("Read")
            })
        })
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
                        super::Expected::new(field_ty.clone(), field.name.span);
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

pub(super) fn has_placeholder(ty: &Type) -> bool {
    match ty {
        Type::Named { arguments, .. } => {
            is_placeholder(ty) || arguments.iter().any(has_placeholder)
        }
        Type::Array { element, .. } => has_placeholder(element),
        Type::Function { parameter, result } => {
            has_placeholder(parameter) || has_placeholder(result)
        }
        _ => false,
    }
}

/// Replace every `Self<a…>` with `family<prefix…, a…>`.
pub(super) fn fill_family(
    ty: &Type,
    family: &(String, Origin, Vec<Type>),
) -> Type {
    match ty {
        Type::Named { name, arguments } => {
            let arguments: Vec<Type> =
                arguments.iter().map(|a| fill_family(a, family)).collect();
            if is_placeholder(ty) {
                let mut all = family.2.clone();
                all.extend(arguments);
                Type::named(family.1.clone(), &family.0, all)
            } else {
                Type::Named {
                    name: name.clone(),
                    arguments,
                }
            }
        }
        Type::Array { element, length } => Type::Array {
            element: alloc::boxed::Box::new(fill_family(element, family)),
            length: *length,
        },
        Type::Function { parameter, result } => Type::function(
            fill_family(parameter, family),
            fill_family(result, family),
        ),
        _ => ty.clone(),
    }
}

/// The last segment of a named type syntax.
pub(super) fn named(module: &Module, id: TypeId) -> Option<String> {
    match &module.ty(id)?.kind {
        TypeKind::Named { name, .. } => Some(super::last(name)),
        _ => None,
    }
}

pub(super) fn type_item<'m>(
    module: &'m Module,
    name: &str,
) -> Option<(&'m [TypeParameter], Option<&'m Vec<Member>>)> {
    module.entries.iter().find_map(|entry| match entry {
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
    })
}

pub(super) fn trait_item<'m>(
    module: &'m Module,
    name: &str,
) -> Option<(&'m [TypeParameter], &'m [TraitMember])> {
    module.entries.iter().find_map(|entry| match entry {
        Entry::Item(item) => match &item.kind {
            ItemKind::Trait {
                name: n,
                parameters,
                members,
                ..
            } if super::last(n) == name => {
                Some((parameters.as_slice(), members.as_slice()))
            }
            _ => None,
        },
        Entry::Line(_) => None,
    })
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

/// Every impl of type `ty` in `module`: its trait and members.
pub(super) fn impls<'m>(
    module: &'m Module,
    ty: &'m str,
) -> impl Iterator<Item = (Option<TypeId>, &'m [ImplMember])> + 'm {
    module.entries.iter().filter_map(move |entry| match entry {
        Entry::Item(item) => match &item.kind {
            ItemKind::Impl {
                target,
                implements,
                members,
            } if named(module, *target).as_deref() == Some(ty) => {
                Some((*implements, members.as_slice()))
            }
            _ => None,
        },
        Entry::Line(_) => None,
    })
}
