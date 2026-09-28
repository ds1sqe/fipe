//! Name resolution: every name in a module bound to what it means, every
//! dot classified, items collected, with located diagnostics. Nothing
//! here infers a type or reads a spot list.

use crate::{
    ast::{
        Arm, Entry, ExprId, ExprKind, ImplMember, ItemKind, Member, Module,
        Name, PatternId, PatternKind, TypeId, TypeKind,
    },
    decl::{Declarations, Kind, Namespace},
    token::Span,
};
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// Where a declaration lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The module being resolved.
    This,
    Prelude,
    /// The host's command namespace.
    Commands,
    /// A host module, by name.
    Host(String),
    /// A file module supplied to `resolve`, by stem.
    File(String),
}

/// What a name means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A lexical binding: the span of the pattern name that bound it.
    Local {
        site: Span,
    },
    /// A declared or hoisted value, a trait method or an inherent member.
    Value {
        origin: Origin,
        name: String,
    },
    Constructor {
        origin: Origin,
        ty: String,
        name: String,
    },
    Type {
        origin: Origin,
        name: String,
    },
    Trait {
        origin: Origin,
        name: String,
    },
    Module(Origin),
    /// A lowercase name in a type: a type variable.
    Variable(String),
    /// `Self` inside a `type`, `trait` or `impl` body.
    SelfType,
    /// `x.a.b`: members of a value, each a field or a `self` call for
    /// the checker to tell apart.
    Members {
        head: Box<Self>,
        members: Vec<String>,
    },
    /// `X.a`: a member of a type's namespace.
    TypeMember {
        ty: Box<Self>,
        member: String,
    },
    /// An unbound name in an argument position: text.
    Word,
}

/// A module with every name bound; targets are indexed like the arenas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    expressions: Vec<Option<Target>>,
    patterns: Vec<Option<Target>>,
    types: Vec<Option<Target>>,
    pub exports: Vec<Export>,
}

impl Resolved {
    #[must_use]
    pub fn expr(&self, id: ExprId) -> Option<&Target> {
        self.expressions.get(id.0).and_then(Option::as_ref)
    }

    #[must_use]
    pub fn pattern(&self, id: PatternId) -> Option<&Target> {
        self.patterns.get(id.0).and_then(Option::as_ref)
    }

    #[must_use]
    pub fn ty(&self, id: TypeId) -> Option<&Target> {
        self.types.get(id.0).and_then(Option::as_ref)
    }

    /// Every resolved expression node with its target, in arena order.
    pub fn expressions(&self) -> impl Iterator<Item = (ExprId, &Target)> {
        self.expressions
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                Some((ExprId(index), target.as_ref()?))
            })
    }
}

/// A `pub` type, trait or binding of the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub target: Target,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: Span,
    pub reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    UnknownName,
    UnknownType,
    /// A declared type, constructor or associated type is uppercase; a
    /// binding, field, parameter or member is not.
    CaseRule {
        uppercase: bool,
    },
    Duplicate {
        first: Span,
    },
    /// A prelude or host form in a program: type parameters, constraints,
    /// a signature-only declaration, a labeled parameter, an opaque type.
    DeclarationForm,
    SelfOutsideBody,
    /// `use x` names neither a host module nor a supplied file module.
    UnknownModule,
    /// `use m.n`: `m` has no public `n`.
    NotExported,
    /// `m.n` or `T.n`: no such member of the module or type namespace.
    UnknownMember,
    /// `impl T` where `T` is not a type of this module: `Self` is local.
    ImplOutsideModule,
}

/// Resolve one module against the prelude, the host's declarations and
/// the file modules its `use` lines may name.
///
/// # Errors
/// Every located failure; the walk continues past each one.
pub fn resolve(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
) -> Result<Resolved, Vec<Error>> {
    let mut resolver = Resolver {
        module,
        declarations,
        files,
        resolved: Resolved {
            expressions: vec![None; module.expressions.len()],
            patterns: vec![None; module.patterns.len()],
            types: vec![None; module.types.len()],
            exports: Vec::new(),
        },
        frame: Vec::new(),
        scopes: Vec::new(),
        self_type: None,
        errors: Vec::new(),
    };
    resolver.collect();
    resolver.walk();
    if resolver.errors.is_empty() {
        Ok(resolver.resolved)
    } else {
        Err(resolver.errors)
    }
}

/// Which namespace a frame entry belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Value,
    Type,
}

/// Where an expression stands, for the text rule of `host.md`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Argument,
    Other,
}

/// One module-frame entry, with its definition site for duplicates.
struct Defined {
    name: String,
    space: Space,
    target: Target,
    site: Span,
}

struct Resolver<'a> {
    module: &'a Module,
    declarations: &'a Declarations,
    files: &'a [(String, Module)],
    resolved: Resolved,
    /// The module frame: items, hoisted bindings, imports.
    frame: Vec<Defined>,
    /// Lexical scopes, innermost last; a file-level binding after its
    /// line lives in the first one.
    scopes: Vec<Vec<(String, Target)>>,
    self_type: Option<String>,
    errors: Vec<Error>,
}

impl Resolver<'_> {
    fn error(&mut self, span: Span, reason: Reason) {
        self.errors.push(Error { span, reason });
    }

    // ---- the module frame -------------------------------------------

    fn collect(&mut self) {
        for entry in &self.module.entries {
            match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Impl { .. } => {}
                    ItemKind::Use { names, .. } => {
                        for name in names {
                            self.import(name);
                        }
                    }
                    ItemKind::Type {
                        name,
                        parameters,
                        members,
                    } => {
                        self.case(name, true);
                        if !parameters.is_empty() {
                            self.error(name.span, Reason::DeclarationForm);
                        }
                        let Some(members) = members else {
                            self.error(name.span, Reason::DeclarationForm);
                            continue;
                        };
                        self.define(
                            name,
                            Space::Type,
                            Target::Type {
                                origin: Origin::This,
                                name: last(name),
                            },
                            item.public,
                        );
                        self.constructors(name, members, item.public);
                    }
                    ItemKind::Trait {
                        name,
                        parameters,
                        members,
                        ..
                    } => {
                        self.case(name, true);
                        if !parameters.is_empty() {
                            self.error(name.span, Reason::DeclarationForm);
                        }
                        self.define(
                            name,
                            Space::Type,
                            Target::Trait {
                                origin: Origin::This,
                                name: last(name),
                            },
                            item.public,
                        );
                        for member in members {
                            if !member.name.is_uppercase() {
                                self.define(
                                    &member.name,
                                    Space::Value,
                                    Target::Value {
                                        origin: Origin::This,
                                        name: last(&member.name),
                                    },
                                    false,
                                );
                            }
                        }
                    }
                    ItemKind::Declare { name, .. } => {
                        self.error(name.span, Reason::DeclarationForm);
                    }
                },
                Entry::Line(id) => self.hoist(*id),
            }
        }
        for entry in &self.module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Impl {
                    target,
                    implements,
                    members,
                } = &item.kind
                {
                    self.impl_item(*target, *implements, members);
                }
            }
        }
    }

    /// `use m` binds the module's name; `use m.n` binds its export `n`.
    fn import(&mut self, name: &Name) {
        let module = &name.segments[0];
        let Some(origin) = self.module_origin(module) else {
            self.error(name.span, Reason::UnknownModule);
            return;
        };
        if name.segments.len() == 1 {
            self.define(name, Space::Value, Target::Module(origin), false);
            return;
        }
        let member = &name.segments[1];
        let mut found = false;
        for space in [Space::Type, Space::Value] {
            if let Some(target) = self.exported(&origin, member, space) {
                self.define(name, space, target, false);
                found = true;
            }
        }
        if !found {
            self.error(name.span, Reason::NotExported);
        }
    }

    /// A host module or a supplied file module, by name.
    fn module_origin(&self, name: &str) -> Option<Origin> {
        if self.declarations.get_module(name).is_some() {
            Some(Origin::Host(name.to_string()))
        } else if self.file(name).is_some() {
            Some(Origin::File(name.to_string()))
        } else {
            None
        }
    }

    fn file(&self, name: &str) -> Option<&Module> {
        self.files
            .iter()
            .find(|(stem, _)| stem == name)
            .map(|(_, module)| module)
    }

    /// A public name of a host or file module.
    fn exported(
        &self,
        origin: &Origin,
        name: &str,
        space: Space,
    ) -> Option<Target> {
        match origin {
            Origin::Host(module) => {
                let namespace = self.declarations.get_module(module)?;
                match space {
                    Space::Value => {
                        namespace_value(namespace, origin.clone(), name).filter(
                            |_| namespace.value(name).is_none_or(|d| d.public),
                        )
                    }
                    Space::Type => namespace
                        .ty(name)
                        .filter(|declared| declared.public)
                        .and_then(|_| {
                            namespace_type(namespace, origin.clone(), name)
                        }),
                }
            }
            Origin::File(stem) => {
                file_export(self.file(stem)?, origin, name, space)
            }
            _ => None,
        }
    }

    /// The methods of a trait, when the trait is known.
    fn trait_methods(&self, name: &str) -> Option<Vec<String>> {
        for entry in &self.module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Trait {
                    name: trait_name,
                    members,
                    ..
                } = &item.kind
                {
                    if last(trait_name) == name {
                        return Some(
                            members
                                .iter()
                                .filter(|m| !m.name.is_uppercase())
                                .map(|m| last(&m.name))
                                .collect(),
                        );
                    }
                }
            }
        }
        let declarations = self.declarations;
        let namespaces = core::iter::once(&declarations.prelude)
            .chain(declarations.commands.iter());
        for namespace in namespaces {
            if let Some(Kind::Trait { methods, .. }) =
                namespace.ty(name).map(|declared| &declared.kind)
            {
                return Some(methods.clone());
            }
        }
        None
    }

    /// Whether `ty`'s namespace has `member`: an impl member of the
    /// type in the module that owns it, or a method of a trait it
    /// implements. `None` when the owner is not a module we can read.
    fn has_member(&self, ty: &Target, member: &str) -> Option<bool> {
        let (module, name) = match ty {
            Target::Type {
                origin: Origin::This,
                name,
            } => (self.module, name),
            Target::Type {
                origin: Origin::Prelude,
                name,
            } => (&self.declarations.prelude.module, name),
            Target::Type {
                origin: Origin::Commands,
                name,
            } => (&self.declarations.commands.as_ref()?.module, name),
            Target::Type {
                origin: Origin::Host(host),
                name,
            } => (&self.declarations.get_module(host)?.module, name),
            Target::Type {
                origin: Origin::File(stem),
                name,
            } => (self.file(stem)?, name),
            _ => return None,
        };
        for entry in &module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl {
                target,
                implements,
                members,
            } = &item.kind
            else {
                continue;
            };
            if named(module, *target).as_deref() != Some(name) {
                continue;
            }
            let written = members.iter().any(|m| match m {
                ImplMember::Named { name, .. }
                | ImplMember::Type { name, .. } => last(name) == member,
                ImplMember::Constructor(_) => false,
            });
            if written {
                return Some(true);
            }
            if let Some(trait_name) =
                implements.and_then(|id| named(module, id))
            {
                if self
                    .trait_methods(&trait_name)
                    .is_some_and(|methods| methods.iter().any(|m| m == member))
                {
                    return Some(true);
                }
            }
        }
        Some(false)
    }

    /// A record's constructor is the type's name; a sum's are its members.
    fn constructors(&mut self, ty: &Name, members: &[Member], public: bool) {
        let mut sum = false;
        for member in members {
            match member {
                Member::Field(field) => self.case(&field.name, false),
                Member::Constructor { name, fields, .. } => {
                    sum = true;
                    self.case(name, true);
                    for field in fields {
                        self.case(&field.name, false);
                    }
                    self.define(
                        name,
                        Space::Value,
                        Target::Constructor {
                            origin: Origin::This,
                            ty: last(ty),
                            name: last(name),
                        },
                        public,
                    );
                }
            }
        }
        if !sum {
            self.define(
                ty,
                Space::Value,
                Target::Constructor {
                    origin: Origin::This,
                    ty: last(ty),
                    name: last(ty),
                },
                public,
            );
        }
    }

    /// `impl T` is written only in `T`'s module; an inherent impl's
    /// members are functions in scope there, and a trait impl's members
    /// are the trait's methods.
    fn impl_item(
        &mut self,
        target: TypeId,
        implements: Option<TypeId>,
        members: &[ImplMember],
    ) {
        let span = self.module.types[target.0].span;
        let local = named(self.module, target).is_some_and(|name| {
            self.defined(&name, Space::Type).is_some_and(|defined| {
                matches!(defined.target, Target::Type { .. })
            })
        });
        if !local {
            self.error(span, Reason::ImplOutsideModule);
        }
        let methods = implements
            .and_then(|id| named(self.module, id))
            .and_then(|name| self.trait_methods(&name));
        for member in members {
            let ImplMember::Named { name, public, .. } = member else {
                continue;
            };
            self.case(name, false);
            if implements.is_none() {
                self.define(
                    name,
                    Space::Value,
                    Target::Value {
                        origin: Origin::This,
                        name: last(name),
                    },
                    *public,
                );
            } else if methods.as_ref().is_some_and(|methods| {
                !methods.iter().any(|m| *m == last(name))
            }) {
                self.error(name.span, Reason::UnknownMember);
            }
        }
    }

    /// A file-level binding whose value is an arm list is collected with
    /// the items and may recurse.
    fn hoist(&mut self, id: ExprId) {
        let ExprKind::Bind {
            public,
            pattern,
            value,
            ..
        } = &self.module.expressions[id.0].kind
        else {
            return;
        };
        let PatternKind::Name(name) = &self.module.patterns[pattern.0].kind
        else {
            return;
        };
        if !name.is_uppercase()
            && matches!(
                self.module.expressions[value.0].kind,
                ExprKind::Arms(_)
            )
        {
            self.define(
                name,
                Space::Value,
                Target::Value {
                    origin: Origin::This,
                    name: last(name),
                },
                *public,
            );
        }
    }

    fn define(
        &mut self,
        name: &Name,
        space: Space,
        target: Target,
        public: bool,
    ) {
        let text = last(name);
        if let Some(first) = self
            .frame
            .iter()
            .find(|defined| defined.name == text && defined.space == space)
        {
            let first = first.site;
            self.error(name.span, Reason::Duplicate { first });
            return;
        }
        if public {
            self.resolved.exports.push(Export {
                name: text.clone(),
                target: target.clone(),
                span: name.span,
            });
        }
        self.frame.push(Defined {
            name: text,
            space,
            target,
            site: name.span,
        });
    }

    fn case(&mut self, name: &Name, uppercase: bool) {
        if name.is_uppercase() != uppercase {
            self.error(name.span, Reason::CaseRule { uppercase });
        }
    }

    // ---- lookup -----------------------------------------------------

    fn lookup_value(&self, name: &str) -> Option<Target> {
        for scope in self.scopes.iter().rev() {
            if let Some((_, target)) =
                scope.iter().rev().find(|(n, _)| n == name)
            {
                return Some(target.clone());
            }
        }
        if let Some(defined) = self.defined(name, Space::Value) {
            return Some(defined.target.clone());
        }
        let declarations = self.declarations;
        namespace_value(&declarations.prelude, Origin::Prelude, name)
            .or_else(|| {
                declarations
                    .get_module(name)
                    .map(|_| Target::Module(Origin::Host(name.to_string())))
            })
            .or_else(|| {
                declarations.commands.as_ref().and_then(|commands| {
                    namespace_value(commands, Origin::Commands, name)
                })
            })
    }

    fn defined(&self, name: &str, space: Space) -> Option<&Defined> {
        self.frame
            .iter()
            .find(|defined| defined.name == name && defined.space == space)
    }

    fn lookup_type(&self, name: &str) -> Option<Target> {
        if let Some(defined) = self.defined(name, Space::Type) {
            return Some(defined.target.clone());
        }
        let declarations = self.declarations;
        namespace_type(&declarations.prelude, Origin::Prelude, name).or_else(
            || {
                declarations.commands.as_ref().and_then(|commands| {
                    namespace_type(commands, Origin::Commands, name)
                })
            },
        )
    }

    // ---- the walk ---------------------------------------------------

    fn walk(&mut self) {
        self.scopes.push(Vec::new());
        for entry in &self.module.entries {
            match entry {
                Entry::Item(item) => match &item.kind {
                    ItemKind::Use { .. } | ItemKind::Declare { .. } => {}
                    ItemKind::Type { name, members, .. } => {
                        self.self_type = Some(last(name));
                        let mut fields = Vec::new();
                        for member in members.iter().flatten() {
                            match member {
                                Member::Field(field) => {
                                    self.ty(field.ty);
                                    if let Some(default) = field.default {
                                        self.scopes.push(fields.clone());
                                        self.expr(default, Position::Other);
                                        self.scopes.pop();
                                    }
                                    fields.push((
                                        last(&field.name),
                                        Target::Local {
                                            site: field.name.span,
                                        },
                                    ));
                                }
                                Member::Constructor { fields, .. } => {
                                    for field in fields {
                                        self.ty(field.ty);
                                    }
                                }
                            }
                        }
                        self.self_type = None;
                    }
                    ItemKind::Trait { name, members, .. } => {
                        self.self_type = Some(last(name));
                        for member in members {
                            self.ty(member.ty);
                            if let Some(default) = member.default {
                                self.expr(default, Position::Other);
                            }
                        }
                        self.self_type = None;
                    }
                    ItemKind::Impl {
                        target,
                        implements,
                        members,
                    } => {
                        self.ty(*target);
                        if let Some(implements) = implements {
                            self.ty(*implements);
                        }
                        self.self_type = named(self.module, *target);
                        for member in members {
                            match member {
                                ImplMember::Named {
                                    annotation, value, ..
                                } => {
                                    if let Some(annotation) = annotation {
                                        self.ty(*annotation);
                                    }
                                    self.expr(*value, Position::Other);
                                }
                                ImplMember::Type { ty, .. } => self.ty(*ty),
                                ImplMember::Constructor(arm) => self.arm(arm),
                            }
                        }
                        self.self_type = None;
                    }
                },
                Entry::Line(id) => self.line(*id),
            }
        }
        self.scopes.pop();
    }

    /// A line of a file, block or group: a binding is visible after it.
    fn line(&mut self, id: ExprId) {
        let ExprKind::Bind {
            pattern,
            annotation,
            value,
            ..
        } = &self.module.expressions[id.0].kind
        else {
            self.expr(id, Position::Other);
            return;
        };
        if let Some(annotation) = annotation {
            self.ty(*annotation);
        }
        self.expr(*value, Position::Other);
        let named = match &self.module.patterns[pattern.0].kind {
            PatternKind::Name(name) if name.is_uppercase() => {
                // A binding is not uppercase; `hoist` defined nothing.
                self.error(name.span, Reason::CaseRule { uppercase: false });
                return;
            }
            PatternKind::Name(_) => true,
            _ => false,
        };
        let hoisted = named
            && self.scopes.len() == 1
            && matches!(
                self.module.expressions[value.0].kind,
                ExprKind::Arms(_)
            );
        if !hoisted {
            self.bind(*pattern);
        }
    }

    fn expr(&mut self, id: ExprId, position: Position) {
        let expr = &self.module.expressions[id.0];
        match &expr.kind {
            ExprKind::Literal(_)
            | ExprKind::Word(_)
            | ExprKind::Path(_)
            | ExprKind::Selector(_) => {}
            ExprKind::Name(name) => {
                let target = self.value_name(name, position);
                self.resolved.expressions[id.0] = target;
            }
            ExprKind::Instance { name, arguments } => {
                let target = self.value_name(name, Position::Other);
                self.resolved.expressions[id.0] = target;
                for argument in arguments {
                    self.ty(*argument);
                }
            }
            ExprKind::Record { name, fields } => {
                let target = self.record_name(name);
                self.resolved.expressions[id.0] = target;
                for field in fields {
                    self.expr(field.value, Position::Other);
                }
            }
            ExprKind::Call {
                callee, arguments, ..
            } => self.call(*callee, arguments),
            ExprKind::Field { value, .. } | ExprKind::Not { value, .. } => {
                self.expr(*value, Position::Other);
            }
            ExprKind::Group(lines) | ExprKind::Block(lines) => {
                self.scopes.push(Vec::new());
                for line in lines {
                    self.line(*line);
                }
                self.scopes.pop();
            }
            ExprKind::List(items) => {
                for item in items {
                    self.expr(*item, Position::Other);
                }
            }
            ExprKind::Outcome { payload, .. } => {
                if let Some(payload) = payload {
                    self.expr(*payload, Position::Other);
                }
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipe { left, right, .. } => {
                self.expr(*left, Position::Other);
                self.expr(*right, Position::Other);
            }
            ExprKind::Bind {
                pattern,
                annotation,
                value,
                ..
            } => {
                // A binding as an arm body or a `with` body binds nothing
                // visible after it.
                if let Some(annotation) = annotation {
                    self.ty(*annotation);
                }
                self.expr(*value, Position::Other);
                self.scopes.push(Vec::new());
                self.bind(*pattern);
                self.scopes.pop();
            }
            ExprKind::With { value, body, .. } => {
                self.expr(*value, Position::Other);
                self.expr(*body, Position::Other);
            }
            ExprKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.expr(*condition, Position::Other);
                self.expr(*then_body, Position::Other);
                if let Some(else_body) = else_body {
                    self.expr(*else_body, Position::Other);
                }
            }
            ExprKind::Arms(arms) => {
                for arm in arms {
                    self.arm(arm);
                }
            }
        }
    }

    /// A callee stands in value position; an argument may be text.
    fn call(&mut self, callee: ExprId, arguments: &[crate::ast::Argument]) {
        self.expr(callee, Position::Other);
        for argument in arguments {
            match &argument.kind {
                crate::ast::ArgumentKind::Positional(value) => {
                    self.expr(*value, Position::Argument);
                }
                crate::ast::ArgumentKind::Flag { value, .. } => {
                    if let Some(value) = value {
                        self.expr(*value, Position::Argument);
                    }
                }
            }
        }
    }

    /// A head binds its parameters in order; a default sees the earlier
    /// ones; the guard and body see them all.
    fn arm(&mut self, arm: &Arm) {
        self.scopes.push(Vec::new());
        for pattern in &arm.patterns {
            self.bind(*pattern);
        }
        if let Some(guard) = arm.guard {
            self.expr(guard, Position::Other);
        }
        self.expr(arm.body, Position::Other);
        self.scopes.pop();
    }

    /// Bind a pattern's names in the innermost scope; constructor and
    /// type names in it resolve.
    fn bind(&mut self, id: PatternId) {
        let pattern = &self.module.patterns[id.0];
        match &pattern.kind {
            PatternKind::Wildcard | PatternKind::Literal(_) => {}
            PatternKind::Name(name) => {
                if name.is_uppercase() {
                    // A bare constructor, such as `Nothing`.
                    let target = self.value_name(name, Position::Other);
                    self.resolved.patterns[id.0] = target;
                } else {
                    self.local(name);
                }
            }
            PatternKind::Constructor { name, payload } => {
                let target = self.value_name(name, Position::Other);
                self.resolved.patterns[id.0] = target;
                self.bind(*payload);
            }
            PatternKind::Default { name, value } => {
                self.expr(*value, Position::Other);
                self.local(name);
            }
            PatternKind::Group(inner) => self.bind(*inner),
            PatternKind::List(items) => {
                for item in items {
                    self.bind(*item);
                }
            }
            PatternKind::Outcome { payload, .. } => {
                if let Some(payload) = payload {
                    self.bind(*payload);
                }
            }
            PatternKind::Record { name, fields, .. } => {
                let target = self.record_name(name);
                self.resolved.patterns[id.0] = target;
                for field in fields {
                    match field.pattern {
                        Some(pattern) => self.bind(pattern),
                        None => self.local(&field.name),
                    }
                }
            }
        }
    }

    fn local(&mut self, name: &Name) {
        let target = Target::Local { site: name.span };
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((last(name), target));
        }
    }

    /// A name in value position, with its qualified tail as members.
    fn value_name(
        &mut self,
        name: &Name,
        position: Position,
    ) -> Option<Target> {
        let first = &name.segments[0];
        let qualified = name.segments.len() > 1;
        // `X.a`: a type's namespace, when `X` names a type.
        if qualified
            && (first == "Self" || first.starts_with(char::is_uppercase))
        {
            let ty = if first == "Self" {
                if self.self_type.is_none() {
                    self.error(name.span, Reason::SelfOutsideBody);
                    return None;
                }
                Some(Target::SelfType)
            } else {
                self.lookup_type(first)
            };
            if let Some(ty) = ty {
                let member = &name.segments[1];
                if self.has_member(&ty, member) == Some(false) {
                    self.error(name.span, Reason::UnknownMember);
                    return None;
                }
                let head = Target::TypeMember {
                    ty: Box::new(ty),
                    member: member.clone(),
                };
                return Some(members(head, &name.segments[2..]));
            }
        }
        let head = if first == "Self" {
            if self.self_type.is_some() {
                Some(Target::SelfType)
            } else {
                self.error(name.span, Reason::SelfOutsideBody);
                return None;
            }
        } else {
            self.lookup_value(first)
        };
        let Some(head) = head else {
            if !qualified && position == Position::Argument {
                return Some(Target::Word);
            }
            self.error(name.span, Reason::UnknownName);
            return None;
        };
        if !qualified {
            return Some(head);
        }
        // `m.a`: a module's export, which must exist.
        if let Target::Module(origin) = &head {
            let Some(export) =
                self.exported(origin, &name.segments[1], Space::Value)
            else {
                self.error(name.span, Reason::UnknownMember);
                return None;
            };
            return Some(members(export, &name.segments[2..]));
        }
        Some(members(head, &name.segments[1..]))
    }

    /// The uppercase name before `{`: a record type, or a sum's
    /// constructor with fields, such as `Rect { w, h }`.
    fn record_name(&mut self, name: &Name) -> Option<Target> {
        if name.segments.len() == 1 && name.segments[0] != "Self" {
            if let Some(ty) = self.lookup_type(&name.segments[0]) {
                return Some(ty);
            }
            if let Some(constructor @ Target::Constructor { .. }) =
                self.lookup_value(&name.segments[0])
            {
                return Some(constructor);
            }
            self.error(name.span, Reason::UnknownType);
            return None;
        }
        self.type_name(name)
    }

    /// A name in type position.
    fn type_name(&mut self, name: &Name) -> Option<Target> {
        let first = &name.segments[0];
        if first == "Self" {
            if self.self_type.is_some() {
                return Some(Target::SelfType);
            }
            self.error(name.span, Reason::SelfOutsideBody);
            return None;
        }
        if name.segments.len() == 2 {
            // `m.T`: a module's exported type.
            let found = self.module_origin(first).and_then(|origin| {
                self.exported(&origin, &name.segments[1], Space::Type)
            });
            if found.is_none() {
                self.error(name.span, Reason::UnknownType);
            }
            return found;
        }
        if name.segments.len() > 2 {
            self.error(name.span, Reason::UnknownType);
            return None;
        }
        if let Some(target) = self.lookup_type(first) {
            return Some(target);
        }
        if name.is_uppercase() {
            self.error(name.span, Reason::UnknownType);
            None
        } else if let Some(origin) = self.module_origin(first) {
            // `Rd<fs>`: an API reference.
            Some(Target::Module(origin))
        } else {
            Some(Target::Variable(first.clone()))
        }
    }

    fn ty(&mut self, id: TypeId) {
        let ty = &self.module.types[id.0];
        match &ty.kind {
            TypeKind::Unit => {}
            TypeKind::Named { name, arguments } => {
                let target = self.type_name(name);
                self.resolved.types[id.0] = target;
                for argument in arguments {
                    self.ty(*argument);
                }
            }
            TypeKind::Array { element, .. } => self.ty(*element),
            TypeKind::Function { parameter, result } => {
                self.ty(*parameter);
                self.ty(*result);
            }
            TypeKind::Qualified { .. } | TypeKind::Labeled { .. } => {
                self.error(ty.span, Reason::DeclarationForm);
            }
        }
    }
}

/// `head.a.b`: the remaining segments as members of the checker's.
fn members(head: Target, rest: &[String]) -> Target {
    if rest.is_empty() {
        head
    } else {
        Target::Members {
            head: Box::new(head),
            members: rest.to_vec(),
        }
    }
}

/// A `pub` name of a file module, read syntactically.
fn file_export(
    module: &Module,
    origin: &Origin,
    name: &str,
    space: Space,
) -> Option<Target> {
    let origin = origin.clone();
    for entry in &module.entries {
        match entry {
            Entry::Item(item) if item.public => match (&item.kind, space) {
                (ItemKind::Type { name: ty, .. }, Space::Type)
                    if last(ty) == name =>
                {
                    return Some(Target::Type {
                        origin,
                        name: name.to_string(),
                    });
                }
                (
                    ItemKind::Type {
                        name: ty, members, ..
                    },
                    Space::Value,
                ) => {
                    let sum = members
                        .iter()
                        .flatten()
                        .any(|m| matches!(m, Member::Constructor { .. }));
                    let constructor = if sum {
                        members.iter().flatten().any(|m| {
                            matches!(m, Member::Constructor { name: c, .. } if last(c) == name)
                        })
                    } else {
                        last(ty) == name
                    };
                    if constructor {
                        return Some(Target::Constructor {
                            origin,
                            ty: last(ty),
                            name: name.to_string(),
                        });
                    }
                }
                (ItemKind::Trait { name: t, .. }, Space::Type)
                    if last(t) == name =>
                {
                    return Some(Target::Trait {
                        origin,
                        name: name.to_string(),
                    });
                }
                (ItemKind::Trait { members, .. }, Space::Value)
                    if members.iter().any(|m| last(&m.name) == name) =>
                {
                    return Some(Target::Value {
                        origin,
                        name: name.to_string(),
                    });
                }
                _ => {}
            },
            Entry::Line(id) if space == Space::Value => {
                let ExprKind::Bind {
                    public: true,
                    pattern,
                    ..
                } = &module.expressions[id.0].kind
                else {
                    continue;
                };
                if let PatternKind::Name(bound) =
                    &module.patterns[pattern.0].kind
                {
                    if last(bound) == name {
                        return Some(Target::Value {
                            origin,
                            name: name.to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn last(name: &Name) -> String {
    name.segments.last().cloned().unwrap_or_default()
}

/// The type a `Named` type syntax names, for `Self` in an impl.
fn named(module: &Module, id: TypeId) -> Option<String> {
    match &module.types[id.0].kind {
        TypeKind::Named { name, .. } => Some(last(name)),
        _ => None,
    }
}

/// A value of a declaration namespace: a declared function, a trait
/// method, a binding, or a constructor of one of its types.
fn namespace_value(
    namespace: &Namespace,
    origin: Origin,
    name: &str,
) -> Option<Target> {
    if namespace.value(name).is_some() {
        return Some(Target::Value {
            origin,
            name: name.to_string(),
        });
    }
    namespace
        .types
        .iter()
        .find_map(|declared| match &declared.kind {
            Kind::Type { constructors, .. }
                if constructors.iter().any(|c| c == name) =>
            {
                Some(Target::Constructor {
                    origin: origin.clone(),
                    ty: declared.name.clone(),
                    name: name.to_string(),
                })
            }
            _ => None,
        })
}

fn namespace_type(
    namespace: &Namespace,
    origin: Origin,
    name: &str,
) -> Option<Target> {
    namespace.ty(name).map(|declared| match declared.kind {
        Kind::Trait { .. } => Target::Trait {
            origin,
            name: name.to_string(),
        },
        _ => Target::Type {
            origin,
            name: name.to_string(),
        },
    })
}
