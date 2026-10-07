//! Name resolution: every name in a module bound to what it means, every
//! dot classified, items collected, with located diagnostics. Nothing
//! here infers a type or reads a spot list.

use crate::{
    ast::{
        Arm, Entry, ExprId, ExprKind, ImplMember, ItemKind, Literal, Member,
        Module, Name, PatternId, PatternKind, TypeId, TypeKind,
    },
    decl::{Declarations, Kind, Namespace},
    frame::Frame,
    token::Span,
};
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

mod unchecked;

pub use unchecked::Unchecked;
pub(crate) use unchecked::{bytes, scan};

/// Where a declaration lives.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    /// A declaration of a session's accepted line, by its index.
    Line(usize),
}

/// What a name means.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    /// A lexical binding: the span of the pattern name that bound it.
    Local {
        site: Span,
    },
    /// A declared value.
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
    Module(Origin),
    /// A lowercase name in a type, or a `type` item's own parameter: a
    /// type variable.
    Variable(String),
    /// `Self` inside a `type` or `impl` body.
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
    /// What the module defines at its top level, in order: a later
    /// definition of a name shadows an earlier one.
    pub(crate) definitions: Vec<Definition>,
}

/// A top-level name a module defines, in the type namespace or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Definition {
    pub name: String,
    pub ty: bool,
    pub target: Target,
    pub site: Span,
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

/// A `pub` type or binding of the module.
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
    /// A default reads a local outside its earlier spots.
    DefaultScope,
    UnknownName,
    /// A type without a test cannot be called as a gate.
    NoGate {
        name: String,
    },
    /// A timing name used outside its one-operand form.
    TimingForm,
    UnknownType,
    /// A declared type or constructor is uppercase; a
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
    LimitedImpl,
    /// A record form names a field the type does not declare.
    UnknownField,
    /// A private field supplied, read or matched outside its module.
    PrivateField {
        field: String,
    },
    /// A non-`pub` impl member of a type, reached through `T.a` or
    /// `m.T.a` from another module.
    PrivateMember {
        member: String,
    },
    /// A `use` that reaches its own module again, at the `use` that
    /// closes the cycle; from [`resolve`], at the module's `use` that
    /// reaches the cycle.
    ImportCycle,
    /// A top-level line of a used module that is neither an item nor a
    /// binding: a used module has no main. From [`resolve`], at the
    /// module's `use` that reaches that module.
    NoMain,
    /// A form the parser accepts that the checker does not check yet.
    NotYetChecked {
        form: Unchecked,
    },
    /// A lowercase name in a type that spells an old numeric type, such
    /// as `i64`: the numeric types are uppercase, `upper`.
    NumericName {
        upper: String,
    },
}

/// Resolve one module against the prelude, the host's declarations and
/// the file modules its `use` lines may name.
///
/// The names a `use` brings
/// are visible from its line onward, like a binding's. The file modules
/// a `use` reaches, directly or through theirs, must hold items and
/// bindings only and form no cycle; each fault is reported once, at the
/// first `use` of this module that reaches it.
///
/// # Errors
/// Every located failure; the walk continues past each one.
pub fn resolve(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
) -> Result<Resolved, Vec<Error>> {
    let mut unit = Resolver::new(module, declarations, files);
    unit.unit = true;
    let (resolved, errors) = unit.run();
    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

/// [`resolve`] without its checks of the used modules, keeping what it
/// resolved beside its errors: a name that failed has no target.
pub(crate) fn resolve_partial(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
) -> (Resolved, Vec<Error>) {
    resolve_in(module, declarations, files, None)
}

/// [`resolve_partial`] after the definitions of a session's accepted
/// lines, which a name of the module finds after its own and before the
/// prelude's.
pub(crate) fn resolve_in(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    frame: Option<&Frame>,
) -> (Resolved, Vec<Error>) {
    let mut resolver = Resolver::new(module, declarations, files);
    resolver.frame_in = frame;
    resolver.run()
}

/// [`resolve_partial`] of the file module `stem` among `files`, as a
/// used module: a line that is neither an item nor a binding has no
/// main, and a `use` that reaches `stem` again closes an import cycle.
pub(crate) fn resolve_used(
    module: &Module,
    declarations: &Declarations,
    files: &[(String, Module)],
    stem: &str,
) -> (Resolved, Vec<Error>) {
    let mut resolver = Resolver::new(module, declarations, files);
    resolver.own = Some(stem);
    resolver.run()
}

impl Resolver<'_> {
    fn run(mut self) -> (Resolved, Vec<Error>) {
        self.collect();
        self.walk();
        self.timing_forms();
        let mut definitions: Vec<Definition> = self
            .frame
            .iter()
            .map(|defined| Definition {
                name: defined.name.clone(),
                ty: defined.space == Space::Type,
                target: defined.target.clone(),
                site: defined.site,
            })
            .collect();
        definitions.extend(self.top.iter().filter_map(|(name, target)| {
            let Target::Local { site } = target else {
                return None;
            };
            Some(Definition {
                name: name.clone(),
                ty: false,
                target: target.clone(),
                site: *site,
            })
        }));
        self.resolved.definitions = definitions;
        self.export_bindings();
        (self.resolved, self.errors)
    }

    fn timing_forms(&mut self) {
        let mut allowed = Vec::new();
        for expression in &self.module.expressions {
            if let ExprKind::Call {
                callee, arguments, ..
            } = &expression.kind
            {
                if timing(self.resolved.expr(*callee)).is_some()
                    && matches!(arguments.as_slice(), [crate::ast::Argument { kind: crate::ast::ArgumentKind::Positional(_), .. }])
                    && !self.module.expressions.iter().any(|parent| matches!(parent.kind, ExprKind::Pipe { right, .. } if self.module.expressions[right.0].span == expression.span))
                { allowed.push(*callee); }
            }
        }
        for (index, target) in self.resolved.expressions.iter().enumerate() {
            if timing(target.as_ref()).is_some()
                && !allowed.contains(&ExprId(index))
            {
                self.errors.push(Error {
                    span: self.module.expressions[index].span,
                    reason: Reason::TimingForm,
                });
            }
        }
        for (index, target) in self.resolved.patterns.iter().enumerate() {
            if timing(target.as_ref()).is_some() {
                self.errors.push(Error {
                    span: self.module.patterns[index].span,
                    reason: Reason::TimingForm,
                });
            }
        }
    }

    /// A module exports its final bindings: a name whose final
    /// file-level binding is `pub`.
    fn export_bindings(&mut self) {
        let mut finals: Vec<(String, bool, Span)> = Vec::new();
        for (name, public, site) in top_bindings(self.module) {
            finals.retain(|(n, _, _)| *n != name);
            finals.push((name, public, site));
        }
        for (name, public, span) in finals {
            if public {
                self.resolved.exports.push(Export {
                    target: Target::Value {
                        origin: Origin::This,
                        name: name.clone(),
                    },
                    name,
                    span,
                });
            }
        }
    }
}

/// Resolve one declared spot default of a declaration module, with the
/// spots before it in scope by name and site. Its other names are the
/// prelude's and the commands'; what does not resolve stays unresolved.
pub(crate) fn resolve_declared_expression(
    module: &Module,
    declarations: &Declarations,
    home: &Origin,
    value: ExprId,
    spots: &[(String, Span)],
    owner: Option<&str>,
) -> (Resolved, Vec<Error>) {
    let mut resolver = Resolver::new(module, declarations, &[]);
    resolver.self_type = owner.map(String::from);
    if let Some(owner) = owner {
        resolver.parameters = module
            .entries
            .iter()
            .find_map(|entry| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let ItemKind::Type {
                    name, parameters, ..
                } = &item.kind
                else {
                    return None;
                };
                (last(name) == owner).then(|| {
                    parameters
                        .iter()
                        .map(|parameter| last(&parameter.name))
                        .collect()
                })
            })
            .unwrap_or_default();
    }
    let namespace = match home {
        Origin::Prelude => Some(&declarations.prelude),
        Origin::Commands => declarations.commands.as_ref(),
        Origin::Host(name) => declarations.get_module(name),
        _ => None,
    };
    if let Some(namespace) = namespace {
        for declaration in &namespace.types {
            let name = Name {
                segments: vec![declaration.name.clone()],
                span: declaration.span,
            };
            if let Some(target) =
                namespace_type(namespace, home.clone(), &declaration.name)
            {
                resolver.define(&name, Space::Type, target, false);
            }
            if let Kind::Type { constructors, .. } = &declaration.kind {
                for constructor in constructors {
                    if let Some(target) =
                        namespace_value(namespace, home.clone(), constructor)
                    {
                        let name = Name {
                            segments: vec![constructor.clone()],
                            span: declaration.span,
                        };
                        resolver.define(&name, Space::Value, target, false);
                    }
                }
            }
        }
        for declaration in &namespace.values {
            let name = Name {
                segments: vec![declaration.name.clone()],
                span: declaration.span,
            };
            if let Some(target) =
                namespace_value(namespace, home.clone(), &declaration.name)
            {
                resolver.define(&name, Space::Value, target, false);
            }
        }
    }
    for entry in &module.entries {
        if let Entry::Item(crate::ast::Item {
            kind: ItemKind::Use { names, .. },
            ..
        }) = entry
        {
            for name in names {
                resolver.import(name);
            }
        }
    }

    let scope = spots
        .iter()
        .map(|(name, site)| (name.clone(), Target::Local { site: *site }))
        .collect();
    resolver.scopes.push(scope);
    resolver.expr(value, Position::Other);
    (resolver.resolved, resolver.errors)
}

/// Which namespace a frame entry belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Value,
    Type,
}

/// Whether a type's namespace has a member, and whether it is visible
/// from outside its module.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Membership {
    Absent,
    Private,
    Visible,
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
    /// The module frame: items and imports.
    frame: Vec<Defined>,
    frame_index: [alloc::collections::BTreeMap<String, usize>; 2],
    /// A session's accepted lines, after the module frame.
    frame_in: Option<&'a Frame>,
    /// The file-level bindings after the walk.
    top: Vec<(String, Target)>,
    /// The stem of a used module being checked as one.
    own: Option<&'a str>,
    /// Checking a whole unit: the file modules its `use` lines reach are
    /// checked as used modules, reported at those lines.
    unit: bool,
    /// The file modules already checked for [`Self::unit`].
    reached: Vec<String>,
    /// Lexical scopes, innermost last; a file-level binding after its
    /// line lives in the first one.
    scopes: Vec<Vec<(String, Target)>>,
    self_type: Option<String>,
    /// The type parameters of the `type` item being walked.
    parameters: Vec<String>,
    /// Every impl member of this module, by (type, name), for coherence.
    members: alloc::collections::BTreeMap<(String, String), (Span, bool)>,
    errors: Vec<Error>,
}

/// The fields of a record or of a constructor with fields.
struct Shape {
    fields: Vec<(String, bool)>,
    /// Declared outside the module being resolved.
    foreign: bool,
    /// The type's module writes a constructor, so the derived one's
    /// spots are not its fields.
    written: bool,
}

impl<'a> Resolver<'a> {
    fn new(
        module: &'a Module,
        declarations: &'a Declarations,
        files: &'a [(String, Module)],
    ) -> Self {
        Self {
            module,
            declarations,
            files,
            resolved: Resolved {
                expressions: vec![None; module.expressions.len()],
                patterns: vec![None; module.patterns.len()],
                types: vec![None; module.types.len()],
                exports: Vec::new(),
                definitions: Vec::new(),
            },
            frame: Vec::new(),
            frame_index: [
                alloc::collections::BTreeMap::new(),
                alloc::collections::BTreeMap::new(),
            ],
            frame_in: None,
            top: Vec::new(),
            own: None,
            unit: false,
            reached: Vec::new(),
            scopes: Vec::new(),
            self_type: None,
            parameters: Vec::new(),
            members: alloc::collections::BTreeMap::new(),
            errors: Vec::new(),
        }
    }

    /// Record an error once: a record type and its constructor share a
    /// name, so a duplicate type is found twice at one site.
    fn error(&mut self, span: Span, reason: Reason) {
        let error = Error { span, reason };
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }

    fn unchecked(&mut self, span: Span, form: Unchecked) {
        self.error(span, Reason::NotYetChecked { form });
    }

    // ---- the module frame -------------------------------------------

    fn collect(&mut self) {
        for entry in &self.module.entries {
            match entry {
                Entry::Item(item) => match &item.kind {
                    // A `use` is sequential: the walk brings its names.
                    ItemKind::Impl { .. } | ItemKind::Use { .. } => {}
                    ItemKind::Type {
                        name,
                        base,
                        members,
                        limits,
                        ..
                    } => {
                        self.case(name, true);
                        let limited = base.is_some() || !limits.is_empty();
                        if !limited && members.is_none() {
                            self.error(name.span, Reason::DeclarationForm);
                            continue;
                        }
                        self.define(
                            name,
                            Space::Type,
                            Target::Type {
                                origin: Origin::This,
                                name: last(name),
                            },
                            item.public,
                        );
                        if let Some(members) = members {
                            self.constructors(name, members, item.public);
                        } else if limits.iter().any(|limit| {
                            matches!(limit, crate::ast::Limit::Test(_))
                        }) || base.is_some_and(|id| {
                            test_base(
                                self.module,
                                &self.declarations.prelude.module,
                                id,
                                0,
                            ) || self.base_gate(id)
                        }) {
                            self.define(
                                name,
                                Space::Value,
                                Target::Type {
                                    origin: Origin::This,
                                    name: last(name),
                                },
                                item.public,
                            );
                        }
                    }
                    ItemKind::Declare { name, .. } => {
                        self.error(name.span, Reason::DeclarationForm);
                    }
                },
                Entry::Line(id) => self.main_line(*id),
            }
        }
        for entry in &self.module.entries {
            if let Entry::Item(item) = entry {
                if let ItemKind::Impl {
                    target,
                    claim,
                    members,
                } = &item.kind
                {
                    self.impl_item(*target, members, claim.is_some());
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
        if self.cycle(module) {
            self.error(name.span, Reason::ImportCycle);
        }
        if self.unit && matches!(origin, Origin::File(_)) {
            self.used_faults(name.span, module);
        }
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

    /// Whether `use stem` in the used module being checked closes an
    /// import cycle: it names that module, or a module loaded before it
    /// whose `use` lines reach it.
    fn cycle(&self, stem: &str) -> bool {
        let Some(own) = self.own else { return false };
        if stem == own {
            return true;
        }
        let position =
            |name: &str| self.files.iter().position(|(s, _)| s == name);
        if position(stem) >= position(own) {
            return false;
        }
        let mut seen: Vec<&str> = Vec::new();
        let mut pending = alloc::vec![stem];
        while let Some(next) = pending.pop() {
            if next == own {
                return true;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(module) = self.file(next) {
                pending.extend(used(module));
            }
        }
        false
    }

    /// Report at `span` a cycle among, or a bare line in, the file
    /// modules `use stem` reaches that no earlier `use` reached.
    fn used_faults(&mut self, span: Span, stem: &str) {
        let mut reached = core::mem::take(&mut self.reached);
        let (cycle, main) = self.reach(stem, &mut Vec::new(), &mut reached);
        self.reached = reached;
        if cycle {
            self.error(span, Reason::ImportCycle);
        }
        if main {
            self.error(span, Reason::NoMain);
        }
    }

    /// Whether the file modules from `stem` on, past those `reached`,
    /// return to one on the `path` of uses that led here, and whether one
    /// has a line that is neither an item nor a binding.
    fn reach<'s>(
        &'s self,
        stem: &'s str,
        path: &mut Vec<&'s str>,
        reached: &mut Vec<String>,
    ) -> (bool, bool) {
        if path.contains(&stem) {
            return (true, false);
        }
        if reached.iter().any(|r| r == stem)
            || self.declarations.get_module(stem).is_some()
        {
            return (false, false);
        }
        let Some(module) = self.file(stem) else {
            return (false, false);
        };
        let mut main = module.entries.iter().any(|entry| {
            matches!(entry, Entry::Line(id)
                if !matches!(module.expressions[id.0].kind, ExprKind::Bind { .. }))
        });
        let mut cycle = false;
        path.push(stem);
        for next in used(module) {
            let (c, m) = self.reach(next, path, reached);
            cycle |= c;
            main |= m;
        }
        path.pop();
        reached.push(String::from(stem));
        (cycle, main)
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
            Origin::File(stem) => file_export(
                self.file(stem)?,
                &self.declarations.prelude.module,
                origin,
                name,
                space,
            ),
            _ => None,
        }
    }

    /// Whether `ty`'s namespace has `member`: an impl member of the
    /// type in its owning module. `None` when we cannot read that module.
    fn has_member(&self, ty: &Target, member: &str) -> Option<Membership> {
        if let Target::Type {
            origin: Origin::Line(line),
            name,
        } = ty
        {
            // Its impls are in this module or in accepted lines.
            let frame = self.frame_in?;
            let lines = frame.impls.get(&(*line, name.clone()));
            let found = core::iter::once(self.module)
                .chain(frame.lines.get(*line).map(|l| &l.module))
                .chain(lines.into_iter().flatten().filter_map(|index| {
                    frame.lines.get(*index).map(|line| &line.module)
                }))
                .map(|module| self.impl_has(module, name, member, false))
                .find(|membership| *membership != Membership::Absent)
                .unwrap_or(Membership::Absent);
            return Some(found);
        }
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
            } => {
                // Another file module's inherent member must be `pub`.
                return Some(self.impl_has(
                    self.file(stem)?,
                    name,
                    member,
                    true,
                ));
            }
            _ => return None,
        };
        Some(self.impl_has(module, name, member, false))
    }

    /// Whether an impl of the type `name` in `module` has `member`; with
    /// `public`, an inherent member that is not `pub` is [`Membership::
    /// Private`] rather than visible.
    fn impl_has(
        &self,
        module: &Module,
        name: &str,
        member: &str,
        public: bool,
    ) -> Membership {
        let mut hidden = false;
        for entry in &module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl {
                target, members, ..
            } = &item.kind
            else {
                continue;
            };
            if named(module, *target).as_deref() != Some(name) {
                continue;
            }
            for entry in members {
                if let ImplMember::Named {
                    name,
                    public: visible,
                    ..
                } = entry
                {
                    if last(name) == member {
                        if *visible || !public || module.claim_member(name.span)
                        {
                            return Membership::Visible;
                        }
                        hidden = true;
                    }
                }
            }
        }
        if hidden {
            return Membership::Private;
        }
        if matches!(member, "eq" | "cmp" | "show" | "default") && module.entries.iter().any(|entry| matches!(entry,
            Entry::Item(crate::ast::Item { kind: ItemKind::Type { name: n, members: Some(_), .. }, .. }) if last(n) == name)) {
            return Membership::Visible;
        }
        for entry in &module.entries {
            if let Entry::Item(crate::ast::Item {
                kind:
                    ItemKind::Type {
                        name: n,
                        base: Some(base),
                        members: Some(_),
                        ..
                    },
                ..
            }) = entry
            {
                if last(n) == name
                    && self.claim_member(module, *base, member, &mut Vec::new())
                {
                    return Membership::Visible;
                }
            }
        }
        Membership::Absent
    }

    fn claim_member(
        &self,
        module: &Module,
        id: TypeId,
        member: &str,
        visiting: &mut Vec<String>,
    ) -> bool {
        match &module.types[id.0].kind {
            TypeKind::Both(parts) => parts
                .iter()
                .any(|p| self.claim_member(module, *p, member, visiting)),
            TypeKind::Named { name, .. } => {
                let text = name.segments.join(".");
                if visiting.contains(&text) {
                    return false;
                }
                visiting.push(text);
                let candidates = [module, &self.declarations.prelude.module];
                let found = candidates.iter().find_map(|m| {
                    m.entries.iter().find_map(|e| match e {
                        Entry::Item(crate::ast::Item {
                            kind:
                                ItemKind::Type {
                                    name: n,
                                    base,
                                    limits,
                                    members: None,
                                    ..
                                },
                            ..
                        }) if last(n) == last(name) => Some((*m, base, limits)),
                        _ => None,
                    })
                });
                let result = found.is_some_and(|(m,base,limits)| limits.iter().any(|l| matches!(l,crate::ast::Limit::Member { name, .. } if last(name) == member)) || base.is_some_and(|b| self.claim_member(m,b,member,visiting)));
                visiting.pop();
                result
            }
            _ => false,
        }
    }

    /// A record's constructor is the type's name; a sum's are its members.
    fn constructors(&mut self, ty: &Name, members: &[Member], public: bool) {
        let mut sum = false;
        let mut seen: Vec<&Name> = Vec::new();
        for member in members {
            match member {
                Member::Field(field) => {
                    self.case(&field.name, false);
                    self.field_once(&mut seen, &field.name);
                }
                Member::Constructor { name, fields, .. } => {
                    sum = true;
                    self.case(name, true);
                    let mut own: Vec<&Name> = Vec::new();
                    for field in fields {
                        self.case(&field.name, false);
                        self.field_once(&mut own, &field.name);
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

    /// One field per name in a record or a constructor.
    fn field_once<'n>(&mut self, seen: &mut Vec<&'n Name>, name: &'n Name) {
        let text = last(name);
        if let Some(first) = seen.iter().find(|n| last(n) == text) {
            let first = first.span;
            self.error(name.span, Reason::Duplicate { first });
        } else {
            seen.push(name);
        }
    }

    /// `impl T` belongs to the module declaring T and supplies its members.
    fn impl_item(
        &mut self,
        target: TypeId,
        members: &[ImplMember],
        claimed: bool,
    ) {
        let span = self.module.types[target.0].span;
        if let Some(name) = named(self.module, target) {
            if let Some(Target::Type { origin, name }) = self.lookup_type(&name)
            {
                if self.owner(&origin).is_some_and(|(module, _)| module.entries.iter().any(|entry| matches!(entry,
                    Entry::Item(crate::ast::Item { kind: ItemKind::Type { name: n, base, members: None, limits, .. }, .. })
                        if last(n) == name && (base.is_some() || !limits.is_empty())))) {
                    self.error(span, Reason::LimitedImpl);
                    return;
                }
            }
        }
        // An accepted line's type is the session's own.
        let local = named(self.module, target).is_some_and(|name| {
            self.defined(&name, Space::Type).map_or_else(
                || {
                    self.frame_in.and_then(|f| f.types.get(&name)).is_some_and(
                        |target| {
                            matches!(
                                target,
                                Target::Type {
                                    origin: Origin::Line(_),
                                    ..
                                }
                            )
                        },
                    )
                },
                |defined| matches!(defined.target, Target::Type { .. }),
            )
        });
        if !local
            && named(self.module, target)
                .is_some_and(|name| self.lookup_type(&name).is_some())
        {
            self.error(span, Reason::ImplOutsideModule);
        }
        let ty = named(self.module, target).unwrap_or_default();
        for member in members {
            let ImplMember::Named { name, .. } = member else {
                continue;
            };
            self.case(name, false);
            if let (
                Some(frame),
                Some(Target::Type {
                    origin: Origin::Line(line),
                    name: owner,
                }),
            ) = (self.frame_in, self.lookup_type(&ty))
            {
                if let Some(first) =
                    frame.members.get(&((line, owner), last(name)))
                {
                    let first = Span {
                        start: first.0,
                        end: first.1,
                    };
                    if !claimed || !self.claim_site(first) {
                        self.error(name.span, Reason::Duplicate { first });
                        continue;
                    }
                }
            }
            // One definition per (type, name), owned by the meaning.
            let text = last(name);
            let key = (ty.clone(), text);
            if let Some((first, prior_claim)) = self.members.get(&key) {
                if !(claimed && *prior_claim) {
                    let first = *first;
                    self.error(name.span, Reason::Duplicate { first });
                    continue;
                }
            }
            self.members.entry(key).or_insert((name.span, claimed));
        }
    }

    fn claim_site(&self, site: Span) -> bool {
        core::iter::once(self.module).chain(self.frame_in.into_iter().flat_map(|f| f.lines.iter().map(|l| &l.module))).any(|module| module.entries.iter().any(|e| matches!(e,Entry::Item(item) if matches!(item.kind,ItemKind::Impl { claim: Some(_), .. }) && item.span.start <= site.start && site.end <= item.span.end)))
    }

    /// A used module holds items and bindings only.
    fn main_line(&mut self, id: ExprId) {
        let expr = &self.module.expressions[id.0];
        if self.own.is_some() && !matches!(expr.kind, ExprKind::Bind { .. }) {
            self.error(expr.span, Reason::NoMain);
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
        if let Some(first) = self.defined(&text, space) {
            let first = first.site;
            self.error(name.span, Reason::Duplicate { first });
            return;
        }
        if matches!(
            (&target, self.lookup_value(&text)),
            (Target::Constructor { .. }, Some(Target::Type { .. }))
                | (Target::Type { .. }, Some(Target::Constructor { .. }))
        ) && matches!(space, Space::Value)
        {
            self.error(name.span, Reason::Duplicate { first: name.span });
            return;
        }
        if public {
            self.resolved.exports.push(Export {
                name: text.clone(),
                target: target.clone(),
                span: name.span,
            });
        }
        self.frame_index[space as usize].insert(text.clone(), self.frame.len());
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
        if let Some(target) = self.frame_in.and_then(|f| f.values.get(name)) {
            return Some(target.clone());
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
        self.frame_index[space as usize]
            .get(name)
            .map(|i| &self.frame[*i])
    }

    fn lookup_type(&self, name: &str) -> Option<Target> {
        if let Some(defined) = self.defined(name, Space::Type) {
            return Some(defined.target.clone());
        }
        if let Some(target) = self.frame_in.and_then(|f| f.types.get(name)) {
            return Some(target.clone());
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
                    ItemKind::Use { names, .. } => {
                        for name in names {
                            self.import(name);
                        }
                    }
                    ItemKind::Declare { .. } => {}
                    ItemKind::Type {
                        name,
                        parameters,
                        members,
                        base,
                        limits,
                    } => {
                        self.self_type = Some(last(name));
                        self.parameters =
                            parameters.iter().map(|p| last(&p.name)).collect();
                        for parameter in parameters {
                            if let Some(bound) = parameter.bound {
                                self.ty(bound);
                            }
                            if let Some(default) = parameter.default {
                                self.ty(default);
                            }
                        }
                        if let Some(base) = base {
                            match &self.module.types[base.0].kind {
                                TypeKind::Both(parts) => {
                                    for part in parts {
                                        self.ty(*part);
                                    }
                                }
                                _ => self.ty(*base),
                            }
                        }
                        for limit in limits {
                            match limit {
                                crate::ast::Limit::Member { ty, .. } => {
                                    self.ty(*ty);
                                }
                                crate::ast::Limit::Test(id) => {
                                    self.scopes.push(vec![(
                                        String::from("self"),
                                        Target::Local { site: name.span },
                                    )]);
                                    self.expr(*id, Position::Other);
                                    self.scopes.pop();
                                }
                            }
                        }
                        let mut fields = Vec::new();
                        for member in members.iter().flatten() {
                            match member {
                                Member::Field(field) => {
                                    self.ty(field.ty);
                                    if let Some(default) = field.default {
                                        self.scopes.push(fields.clone());
                                        self.default_expression(default);
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
                        self.parameters.clear();
                    }
                    ItemKind::Impl {
                        target,
                        claim,
                        members,
                    } => {
                        self.walk_impl(*target, *claim, members);
                    }
                },
                Entry::Line(id) => self.line(*id),
            }
        }
        self.top = self.scopes.pop().unwrap_or_default();
    }

    fn walk_impl(
        &mut self,
        target: TypeId,
        claim: Option<TypeId>,
        members: &[ImplMember],
    ) {
        if let TypeKind::Named { arguments, .. } =
            &self.module.types[target.0].kind
        {
            self.parameters = arguments
                .iter()
                .filter_map(|id| match &self.module.types[id.0].kind {
                    TypeKind::Named { name, arguments }
                        if arguments.is_empty() && name.segments.len() == 1 =>
                    {
                        Some(last(name))
                    }
                    _ => None,
                })
                .collect();
        }
        self.ty(target);
        self.self_type = named(self.module, target);
        if let Some(claim) = claim {
            self.ty(claim);
        }
        for member in members {
            match member {
                ImplMember::Named {
                    name,
                    annotation,
                    value,
                    ..
                } => {
                    if let Some(annotation) = annotation {
                        self.ty(*annotation);
                    }
                    match value {
                        Some(value) => {
                            self.expr(*value, Position::Other);
                        }
                        None => self.error(name.span, Reason::DeclarationForm),
                    }
                }
                ImplMember::Constructor { arm, .. } => {
                    self.arm(arm);
                }
            }
        }
        self.self_type = None;
        self.parameters.clear();
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
        // An arm binding's own name is in scope in its arms.
        let recursive = self.recursive(*pattern, *value);
        if let Some(name) = recursive {
            self.local(name);
        }
        self.expr(*value, Position::Other);
        match &self.module.patterns[pattern.0].kind {
            PatternKind::Name(name) if name.is_uppercase() => {
                self.error(name.span, Reason::CaseRule { uppercase: false });
            }
            _ if recursive.is_some() => {}
            _ => self.bind(*pattern),
        }
    }

    /// The name of a binding `name = arms`, which its arms may call.
    fn recursive(&self, pattern: PatternId, value: ExprId) -> Option<&'a Name> {
        let module = self.module;
        match (
            &module.patterns[pattern.0].kind,
            &module.expressions[value.0].kind,
        ) {
            (PatternKind::Name(name), ExprKind::Arms(_))
                if !name.is_uppercase() =>
            {
                Some(name)
            }
            _ => None,
        }
    }

    fn expr(&mut self, id: ExprId, position: Position) {
        let expr = &self.module.expressions[id.0];
        match &expr.kind {
            ExprKind::Literal(Literal::Affixed(affixed))
                if bytes(affixed).is_none() =>
            {
                self.unchecked(expr.span, Unchecked::Affix);
            }
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
                let names: Vec<_> = fields.iter().map(|f| &f.name).collect();
                self.fields(target.as_ref(), &names);
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
            ExprKind::Tuple(items) | ExprKind::List(items) => {
                for item in items {
                    self.expr(*item, Position::Other);
                }
            }
            ExprKind::Outcome { payload, .. } => {
                if let Some(payload) = payload {
                    self.expr(*payload, Position::Other);
                }
            }
            ExprKind::Binary { .. } | ExprKind::Pipe { .. } => self.chain(id),
            ExprKind::Bind {
                pattern,
                annotation,
                value,
                ..
            } => self.inner_binding(*pattern, *annotation, *value),
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

    /// A flat chain nests to the left: walk its spine without a frame
    /// per operator, in source order.
    fn chain(&mut self, id: ExprId) {
        let mut rights = Vec::new();
        let mut node = id;
        while let ExprKind::Binary { left, right, .. }
        | ExprKind::Pipe { left, right, .. } =
            &self.module.expressions[node.0].kind
        {
            rights.push(*right);
            node = *left;
        }
        self.expr(node, Position::Other);
        for right in rights.into_iter().rev() {
            self.expr(right, Position::Other);
        }
    }

    /// A binding as an arm body or a `with` body binds nothing visible
    /// after it; an arm binding still sees itself.
    fn inner_binding(
        &mut self,
        pattern: PatternId,
        annotation: Option<TypeId>,
        value: ExprId,
    ) {
        if let Some(annotation) = annotation {
            self.ty(annotation);
        }
        self.scopes.push(Vec::new());
        let recursive = self.recursive(pattern, value);
        if let Some(name) = recursive {
            self.local(name);
        }
        self.expr(value, Position::Other);
        if recursive.is_none() {
            self.bind(pattern);
        }
        self.scopes.pop();
    }

    /// A callee stands in value position; an argument may be text.
    fn call(&mut self, callee: ExprId, arguments: &[crate::ast::Argument]) {
        self.expr(callee, Position::Other);
        // A derived constructor called from outside its module has only
        // the public fields as spots.
        let shape = self.resolved.expressions[callee.0]
            .as_ref()
            .filter(|t| matches!(t, Target::Constructor { .. }))
            .and_then(|t| self.shape(t))
            .filter(|shape| shape.foreign && !shape.written);
        let public = shape
            .as_ref()
            .map_or(usize::MAX, |s| s.fields.iter().filter(|f| f.1).count());
        let private_flag = arguments.iter().any(|argument| matches!(&argument.kind, crate::ast::ArgumentKind::Flag { name, .. } if shape.as_ref().is_some_and(|s| s.fields.iter().any(|f| f.0 == *name && !f.1))));
        let mut positional = 0;
        for argument in arguments {
            match &argument.kind {
                crate::ast::ArgumentKind::Positional(value) => {
                    self.expr(*value, Position::Argument);
                    if positional == public && !private_flag {
                        let field = shape.as_ref().and_then(|s| {
                            s.fields.iter().find(|f| !f.1).map(|f| f.0.clone())
                        });
                        if let Some(field) = field {
                            self.error(
                                argument.span,
                                Reason::PrivateField { field },
                            );
                        }
                    }
                    positional += 1;
                }
                crate::ast::ArgumentKind::Flag { name, value } => {
                    if let Some(value) = value {
                        self.expr(*value, Position::Argument);
                    }
                    let private = shape.as_ref().is_some_and(|s| {
                        s.fields.iter().any(|f| f.0 == *name && !f.1)
                    });
                    if private {
                        self.error(
                            argument.span,
                            Reason::PrivateField {
                                field: name.clone(),
                            },
                        );
                    }
                }
            }
        }
    }

    /// The fields a record form names must exist and, outside the
    /// type's module, be public.
    fn fields(&mut self, target: Option<&Target>, names: &[&Name]) {
        let Some(shape) = target.and_then(|t| self.shape(t)) else {
            return;
        };
        for name in names {
            let text = last(name);
            match shape.fields.iter().find(|f| f.0 == text) {
                None => self.error(name.span, Reason::UnknownField),
                Some((_, false)) if shape.foreign => {
                    self.error(name.span, Reason::PrivateField { field: text });
                }
                Some(_) => {}
            }
        }
    }

    /// The declared fields behind a record type, `Self`, or a
    /// constructor, when its module is one we can read.
    fn shape(&self, target: &Target) -> Option<Shape> {
        let (module, foreign, ty, constructor) = match target {
            Target::SelfType => {
                (self.module, false, self.self_type.clone()?, None)
            }
            Target::Type { origin, name } => {
                let (module, foreign) = self.owner(origin)?;
                (module, foreign, name.clone(), None)
            }
            Target::Constructor { origin, ty, name } => {
                let (module, foreign) = self.owner(origin)?;
                (module, foreign, ty.clone(), Some(name.clone()))
            }
            _ => return None,
        };
        let members = module.entries.iter().find_map(|entry| match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Type { name, members, .. } if last(name) == ty => {
                    members.as_ref()
                }
                _ => None,
            },
            Entry::Line(_) => None,
        })?;
        let fields: Vec<(String, bool)> = match constructor {
            // A sum constructor's fields are its payload, always visible.
            Some(name) if name != ty => members
                .iter()
                .find_map(|member| match member {
                    Member::Constructor {
                        name: c, fields, ..
                    } if last(c) == name => Some(fields),
                    _ => None,
                })?
                .iter()
                .map(|f| (last(&f.name), true))
                .collect(),
            _ => members
                .iter()
                .filter_map(|member| match member {
                    Member::Field(f) => Some((last(&f.name), f.public)),
                    Member::Constructor { .. } => None,
                })
                .collect(),
        };
        let written = module.entries.iter().any(|entry| match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Impl {
                    target, members, ..
                } => {
                    named(module, *target).as_deref() == Some(&ty)
                        && members.iter().any(|m| {
                            matches!(m, ImplMember::Constructor { .. })
                        })
                }
                _ => false,
            },
            Entry::Line(_) => false,
        });
        Some(Shape {
            fields,
            foreign,
            written,
        })
    }

    /// The module that owns a declaration, and whether it is another one.
    fn owner(&self, origin: &Origin) -> Option<(&Module, bool)> {
        let (module, foreign) = match origin {
            Origin::This => (self.module, false),
            Origin::Prelude => (&self.declarations.prelude.module, true),
            Origin::Commands => {
                (&self.declarations.commands.as_ref()?.module, true)
            }
            Origin::Host(host) => {
                (&self.declarations.get_module(host)?.module, true)
            }
            Origin::File(stem) => (self.file(stem)?, true),
            // The session is one module for privacy.
            Origin::Line(_) => (self.frame_in?.module(origin)?, false),
        };
        Some((module, foreign && !core::ptr::eq(module, self.module)))
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
            PatternKind::Literal(Literal::Affixed(affixed))
                if bytes(affixed).is_none() =>
            {
                self.unchecked(pattern.span, Unchecked::Affix);
            }
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
                if self.gate(name) {
                    let target = self.lookup_type(&name.segments[0]);
                    self.resolved.patterns[id.0] = target;
                } else {
                    let target = self.value_name(name, Position::Other);
                    self.resolved.patterns[id.0] = target;
                }
                self.bind(*payload);
            }
            PatternKind::Default { name, value } => {
                self.default_expression(*value);
                self.local(name);
            }
            PatternKind::Group(inner) => self.bind(*inner),
            PatternKind::Tuple(items) | PatternKind::List(items) => {
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
                let names: Vec<_> = fields.iter().map(|f| &f.name).collect();
                self.fields(target.as_ref(), &names);
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

    fn base_gate(&self, id: TypeId) -> bool {
        match &self.module.types[id.0].kind {
            TypeKind::Named { name, .. } => {
                let Some(Target::Type { origin, name }) =
                    self.lookup_type(&last(name))
                else {
                    return false;
                };
                self.owner(&origin).is_some_and(|(module, _)| {
                    test_chain(
                        module,
                        &self.declarations.prelude.module,
                        &name,
                        0,
                    )
                })
            }
            TypeKind::Both(parts) | TypeKind::Either(parts) => {
                parts.iter().any(|p| self.base_gate(*p))
            }
            TypeKind::Not(inner) => self.base_gate(*inner),
            _ => false,
        }
    }

    /// A pattern head that names a type and no value: a gate.
    fn gate(&self, name: &Name) -> bool {
        let [first] = name.segments.as_slice() else {
            return false;
        };
        (self.lookup_value(first).is_none()
            || matches!(self.lookup_value(first), Some(Target::Type { .. })))
            && self.lookup_type(first).is_some()
    }

    fn local(&mut self, name: &Name) {
        let target = Target::Local { site: name.span };
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((last(name), target));
        }
    }

    /// A name in value position, with its qualified tail as members.
    fn missing_value(&self, name: &str, qualified: bool) -> Reason {
        if !qualified && self.lookup_type(name).is_some() {
            Reason::NoGate { name: name.into() }
        } else {
            Reason::UnknownName
        }
    }

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
                match self.has_member(&ty, member) {
                    Some(Membership::Absent) => {
                        self.error(name.span, Reason::UnknownMember);
                        return None;
                    }
                    Some(Membership::Private) => {
                        self.error(
                            name.span,
                            Reason::PrivateMember {
                                member: member.clone(),
                            },
                        );
                        return None;
                    }
                    Some(Membership::Visible) | None => {}
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
            let reason = self.missing_value(first, qualified);
            self.error(name.span, reason);
            return None;
        };
        if !qualified {
            return Some(head);
        }
        // `m.a`: a module's export, which must exist; `m.T.a` reads the
        // namespace of its type `T`, as `T.a` does.
        if let Target::Module(origin) = &head {
            let ty = &name.segments[1];
            if name.segments.len() > 2 && ty.starts_with(char::is_uppercase) {
                if let Some(ty) = self.exported(origin, ty, Space::Type) {
                    let member = &name.segments[2];
                    match self.has_member(&ty, member) {
                        Some(Membership::Absent) => {
                            self.error(name.span, Reason::UnknownMember);
                            return None;
                        }
                        Some(Membership::Private) => {
                            self.error(
                                name.span,
                                Reason::PrivateMember {
                                    member: member.clone(),
                                },
                            );
                            return None;
                        }
                        Some(Membership::Visible) | None => {}
                    }
                    let head = Target::TypeMember {
                        ty: Box::new(ty),
                        member: member.clone(),
                    };
                    return Some(members(head, &name.segments[3..]));
                }
            }
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
        if name.segments.len() == 1 && self.parameters.contains(first) {
            return Some(Target::Variable(first.clone()));
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
        } else if NUMERIC.contains(&first.as_str()) {
            let upper = first.to_ascii_uppercase();
            self.error(name.span, Reason::NumericName { upper });
            None
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
            TypeKind::Array { element, length } => {
                self.ty(*element);
                self.ty(*length);
            }
            TypeKind::Number(_) => (),
            TypeKind::Function { parameter, result } => {
                self.ty(*parameter);
                self.ty(*result);
            }
            TypeKind::Sum(parts)
            | TypeKind::Tuple(parts)
            | TypeKind::Both(parts)
            | TypeKind::Either(parts) => {
                for part in parts {
                    self.ty(*part);
                }
            }
            TypeKind::Not(inner) => {
                self.ty(*inner);
            }
            TypeKind::Qualified { constraints, ty } => {
                for constraint in constraints {
                    match &constraint.kind {
                        crate::ast::ConstraintKind::Named { class, .. } => {
                            self.ty(*class);
                        }
                        crate::ast::ConstraintKind::Member { ty, .. } => {
                            self.ty(*ty);
                        }
                    }
                }
                self.ty(*ty);
            }
            TypeKind::Labeled { .. } => {
                self.error(ty.span, Reason::DeclarationForm);
            }
        }
    }
}

/// The numeric types' spellings before 06, which earn a hint.
const NUMERIC: [&str; 10] = [
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
];

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
    prelude: &Module,
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
                    if members.is_none() {
                        if last(ty) == name
                            && test_chain(module, prelude, name, 0)
                        {
                            return Some(Target::Type {
                                origin,
                                name: name.to_string(),
                            });
                        }
                        continue;
                    }
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
                _ => {}
            },
            _ => {}
        }
    }
    // Its final binding of the name decides.
    let exported = space == Space::Value
        && top_bindings(module)
            .filter(|(bound, _, _)| bound == name)
            .last()
            .is_some_and(|(_, public, _)| public);
    exported.then(|| Target::Value {
        origin,
        name: name.to_string(),
    })
}

/// Each file-level `name = …` binding: its name, whether it is `pub`,
/// and its site, in order.
fn top_bindings(
    module: &Module,
) -> impl Iterator<Item = (String, bool, Span)> + '_ {
    module.entries.iter().filter_map(|entry| {
        let Entry::Line(id) = entry else { return None };
        let ExprKind::Bind {
            public, pattern, ..
        } = &module.expressions[id.0].kind
        else {
            return None;
        };
        match &module.patterns[pattern.0].kind {
            PatternKind::Name(bound) => {
                Some((last(bound), *public, bound.span))
            }
            _ => None,
        }
    })
}

/// The stems a module's `use` lines name.
fn used(module: &Module) -> impl Iterator<Item = &str> {
    module
        .entries
        .iter()
        .flat_map(|entry| match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Use { names, .. } => names.as_slice(),
                _ => &[],
            },
            Entry::Line(_) => &[],
        })
        .filter_map(|name| name.segments.first().map(String::as_str))
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

/// A declared function, binding, or constructor of a namespace.
fn namespace_value(
    namespace: &Namespace,
    origin: Origin,
    name: &str,
) -> Option<Target> {
    if origin == Origin::Prelude && matches!(name, "Compile" | "Init") {
        return Some(Target::Type {
            origin,
            name: name.to_string(),
        });
    }
    if namespace.module.entries.iter().any(|e| matches!(e, Entry::Item(item) if matches!(&item.kind, ItemKind::Type { name: n, members: None, .. } if last(n) == name))) && test_chain(&namespace.module, &namespace.module, name, 0) {
        return Some(Target::Type { origin, name: name.to_string() });
    }
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
    namespace.ty(name).map(|_| Target::Type {
        origin,
        name: name.to_string(),
    })
}

fn test_chain(
    module: &Module,
    prelude: &Module,
    name: &str,
    depth: usize,
) -> bool {
    if depth > crate::parse::MAX_NESTING {
        return false;
    }
    let declaration = module.entries.iter().find_map(|entry| {
        let Entry::Item(item) = entry else {
            return None;
        };
        if let ItemKind::Type {
            name: n,
            base,
            limits,
            ..
        } = &item.kind
        {
            if last(n) == name {
                return Some((base, limits));
            }
        }
        None
    });
    if let Some((base, limits)) = declaration {
        limits
            .iter()
            .any(|l| matches!(l, crate::ast::Limit::Test(_)))
            || base.is_some_and(|id| test_base(module, prelude, id, depth + 1))
    } else if !core::ptr::eq(module, prelude) {
        test_chain(prelude, prelude, name, depth + 1)
    } else {
        false
    }
}
fn test_base(
    module: &Module,
    prelude: &Module,
    id: TypeId,
    depth: usize,
) -> bool {
    match &module.types[id.0].kind {
        TypeKind::Named { name, .. } => {
            test_chain(module, prelude, &last(name), depth)
        }
        TypeKind::Both(parts) | TypeKind::Either(parts) => {
            parts.iter().any(|p| test_base(module, prelude, *p, depth))
        }
        TypeKind::Not(inner) => test_base(module, prelude, *inner, depth),
        _ => false,
    }
}

/// Timing forms are identified by origin, so user declarations may shadow them.
pub(crate) fn timing(target: Option<&Target>) -> Option<crate::limits::Phase> {
    match target {
        Some(Target::Type {
            origin: Origin::Prelude,
            name,
        }) if name == "Compile" => Some(crate::limits::Phase::Compile),
        Some(Target::Type {
            origin: Origin::Prelude,
            name,
        }) if name == "Init" => Some(crate::limits::Phase::Init),
        _ => None,
    }
}

impl Resolver<'_> {
    fn default_expression(&mut self, value: ExprId) {
        let mut allowed: Vec<_> = self
            .scopes
            .first()
            .into_iter()
            .chain(self.scopes.last())
            .flat_map(|scope| scope.iter().map(|(_, target)| target.clone()))
            .collect();
        if let Some(frame) = self.frame_in {
            allowed.extend(frame.values.values().cloned());
        }
        self.expr(value, Position::Other);
        let span = self.module.expressions[value.0].span;
        let mut errors = Vec::new();
        for (id, target) in self.resolved.expressions() {
            let expression = &self.module.expressions[id.0];
            if expression.span.start < span.start
                || expression.span.end > span.end
            {
                continue;
            }
            let mut root = target;
            while let Target::Members { head, .. } = root {
                root = head;
            }
            let Target::Local { site } = root else {
                continue;
            };
            if allowed.contains(root)
                || (span.start <= site.start && site.end <= span.end)
            {
                continue;
            }
            let mut name = expression.span;
            if let ExprKind::Name(written)
            | ExprKind::Instance { name: written, .. } = &expression.kind
            {
                if let Some(first) = written.segments.first() {
                    name.end = name.start + first.len();
                }
            }
            errors.push(name);
        }
        for span in errors {
            self.error(span, Reason::DefaultScope);
        }
    }
}
