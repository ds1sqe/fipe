//! Declarations: what the checker knows before it types a line. The
//! prelude and every host declaration are Fipe source parsed by the
//! crate's own parser; a host supplies text, never a Rust type.

use crate::{
    ast::{
        Entry, ExprId, ExprKind, Item, ItemKind, Member, Module, Name,
        PatternKind, TypeId, TypeKind,
    },
    lex::lex,
    parse::{self, parse},
    resolve::{scan, Unchecked},
    token::Span,
};
use alloc::{string::String, vec::Vec};

mod claims;
mod docs;
pub(crate) mod spots;
pub use spots::Problem as SpotProblem;

const PRELUDE: &str = include_str!("../prelude/prelude.fipe");

/// The prelude's source, which the spans of its declarations index.
#[must_use]
pub const fn prelude_source() -> &'static str {
    PRELUDE
}

/// The prelude, the host's command namespace and the host's modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declarations {
    pub prelude: Namespace,
    pub commands: Option<Namespace>,
    pub modules: Vec<Namespace>,
}

impl Declarations {
    /// The prelude alone.
    ///
    /// # Panics
    /// If the bundled prelude does not parse, which is a crate defect.
    #[must_use]
    pub fn new() -> Self {
        let prelude = Namespace::parse("prelude", PRELUDE)
            .unwrap_or_else(|errors| panic!("prelude: {errors:?}"));
        Self {
            prelude,
            commands: None,
            modules: Vec::new(),
        }
    }

    /// The host's command namespace, replacing any earlier one.
    ///
    /// # Errors
    /// Syntax errors, lines that declare nothing, duplicate names, and
    /// forms the checker does not check yet.
    pub fn commands(&mut self, text: &str) -> Result<(), Vec<Error>> {
        let others: Vec<_> =
            core::iter::once(("prelude", &self.prelude.module))
                .chain(
                    self.modules.iter().map(|m| (m.name.as_str(), &m.module)),
                )
                .collect();
        self.commands = Some(Namespace::parse_in("commands", text, &others)?);
        Ok(())
    }

    /// A host module, replacing an earlier one of the same dotted name.
    /// Missing parents are created as empty namespaces. Every segment must
    /// be a lowercase name; a submodule cannot share a value name in its parent.
    ///
    /// # Errors
    /// Invalid module paths, submodule/value collisions, syntax errors,
    /// lines that declare nothing, duplicate names, and
    /// forms the checker does not check yet.
    pub fn module(&mut self, name: &str, text: &str) -> Result<(), Vec<Error>> {
        let named = lex(name);
        let valid = named.errors.is_empty()
            && matches!(named.tokens.as_slice(), [token, end]
                if matches!(end.kind, crate::token::Kind::Eof)
                    && token.span == (Span { start: 0, end: name.len() })
                    && matches!(&token.kind, crate::token::Kind::Name(parts)
                        if parts.iter().all(|part| part.starts_with(char::is_lowercase))))
            && !matches!(name.split('.').next(), Some("prelude" | "commands"));
        if !valid {
            return Err(alloc::vec![Error {
                span: Span::default(),
                reason: Reason::ModuleName {
                    name: String::from(name)
                },
            }]);
        }
        let others: Vec<_> =
            core::iter::once(("prelude", &self.prelude.module))
                .chain(
                    self.modules
                        .iter()
                        .filter(|m| m.name != name)
                        .map(|m| (m.name.as_str(), &m.module)),
                )
                .collect();
        let namespace = Namespace::parse_in(name, text, &others)?;
        let conflict = name
            .match_indices('.')
            .find_map(|(at, _)| {
                let parent = self.get_module(&name[..at])?;
                let child = name[at + 1..].split('.').next()?;
                parent
                    .value(child)
                    .map(|_| String::from(&name[..at + 1 + child.len()]))
            })
            .or_else(|| {
                self.submodules(name).find_map(|child| {
                    let member = child.name.rsplit('.').next()?;
                    namespace.value(member).map(|_| child.name.clone())
                })
            });
        if let Some(name) = conflict {
            return Err(alloc::vec![Error {
                span: Span::default(),
                reason: Reason::ModuleConflict { name },
            }]);
        }
        for (at, _) in name.match_indices('.') {
            let parent = &name[..at];
            if self.get_module(parent).is_none() {
                self.modules.push(Namespace {
                    name: String::from(parent),
                    module: Module::default(),
                    types: Vec::new(),
                    values: Vec::new(),
                });
            }
        }
        self.modules.retain(|module| module.name != name);
        self.modules.push(namespace);
        Ok(())
    }

    /// Complete classification, independent of which meanings a unit reaches.
    pub(crate) fn has_resources(&self) -> bool {
        self.commands.iter().chain(&self.modules).any(|namespace| {
            namespace
                .types
                .iter()
                .any(|ty| matches!(ty.kind, Kind::Type { resource: true, .. }))
        })
    }

    /// The immediate host submodules of a namespace.
    pub fn submodules<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Namespace> {
        self.modules.iter().filter(move |module| {
            module
                .name
                .rsplit_once('.')
                .is_some_and(|(parent, _)| parent == name)
        })
    }

    #[must_use]
    pub fn get_module(&self, name: &str) -> Option<&Namespace> {
        self.modules.iter().find(|module| module.name == name)
    }
}

impl Default for Declarations {
    fn default() -> Self {
        Self::new()
    }
}

/// One declaration file.
///
/// The parsed source's arenas hold every type and default; `types` and
/// `values` are the names it declares, in separate namespaces, so an
/// opaque type and its constructor share a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub name: String,
    pub module: Module,
    pub types: Vec<Declared>,
    pub values: Vec<Declared>,
}

impl Namespace {
    /// # Errors
    /// Syntax errors, lines that declare nothing, duplicate names, and
    /// forms the checker does not check yet.
    ///
    /// # Panics
    /// If the bundled prelude does not parse, a crate defect.
    pub fn parse(name: &str, text: &str) -> Result<Self, Vec<Error>> {
        let prelude = (name != "prelude")
            .then(|| parse(lex(PRELUDE)).expect("bundled prelude parses"));
        let others: Vec<_> = prelude.iter().map(|m| ("prelude", m)).collect();
        Self::parse_in(name, text, &others)
    }

    fn parse_in(
        name: &str,
        text: &str,
        others: &[(&str, &Module)],
    ) -> Result<Self, Vec<Error>> {
        let module = parse(lex(text)).map_err(|errors| {
            errors
                .into_iter()
                .map(|error| Error {
                    span: error.span,
                    reason: Reason::Syntax(error.reason),
                })
                .collect::<Vec<_>>()
        })?;
        let mut namespace = Self {
            name: String::from(name),
            module,
            types: Vec::new(),
            values: Vec::new(),
        };
        let mut errors: Vec<Error> = (if name == "prelude" {
            Vec::new()
        } else {
            scan(&namespace.module)
        })
        .into_iter()
        .map(|(span, form)| Error {
            span,
            reason: Reason::NotYetChecked { form },
        })
        .collect();
        errors.extend(resource_errors(&namespace));
        errors.extend(spots::check(&namespace.module));
        docs::check(&namespace.module, &mut errors);
        claims::check(&namespace.module, others, &mut errors);
        for index in 0..namespace.module.entries.len() {
            if let Err(error) = namespace.collect(index) {
                errors.push(error);
            }
        }
        errors.extend(constructor_collisions(&namespace.module));
        if errors.is_empty() {
            let mut supplied: Vec<Self> = others
                .iter()
                .map(|(name, module)| {
                    let mut ns = Self {
                        name: String::from(*name),
                        module: (*module).clone(),
                        types: Vec::new(),
                        values: Vec::new(),
                    };
                    for i in 0..ns.module.entries.len() {
                        let _ = ns.collect(i);
                    }
                    ns
                })
                .collect();
            supplied.push(namespace.clone());
            let prelude = supplied.remove(
                supplied.iter().position(|n| n.name == "prelude").unwrap(),
            );
            let commands = supplied
                .iter()
                .position(|n| n.name == "commands")
                .map(|i| supplied.remove(i));
            let declarations = Declarations {
                prelude,
                commands,
                modules: supplied,
            };
            let origin = match name {
                "prelude" => crate::resolve::Origin::Prelude,
                "commands" => crate::resolve::Origin::Commands,
                _ => crate::resolve::Origin::Host(String::from(name)),
            };
            errors.extend(
                crate::infer::check_declarations(&declarations, origin)
                    .into_iter()
                    .map(|e| Error {
                        span: e.span,
                        reason: Reason::ClaimBlock {
                            problem: match e.reason {
                                crate::infer::Reason::Mismatch {
                                    expected,
                                    actual,
                                    ..
                                } => alloc::format!(
                                    "expected {}, found {}",
                                    expected.beside(&actual),
                                    actual.beside(&expected)
                                ),
                                _ => String::from(
                                    "supplied member type does not meet claim",
                                ),
                            },
                        },
                    }),
            );
        }
        if errors.is_empty() {
            Ok(namespace)
        } else {
            Err(errors)
        }
    }

    #[must_use]
    pub fn ty(&self, name: &str) -> Option<&Declared> {
        self.types.iter().find(|declared| declared.name == name)
    }

    #[must_use]
    pub fn value(&self, name: &str) -> Option<&Declared> {
        self.values.iter().find(|declared| declared.name == name)
    }

    fn collect(&mut self, index: usize) -> Result<(), Error> {
        let (types, values) = match &self.module.entries[index] {
            Entry::Item(item) => item_names(&self.module, item),
            Entry::Line(id) => {
                (Vec::new(), alloc::vec![binding(&self.module, *id)?])
            }
        };
        for declared in types {
            Self::push(&mut self.types, declared)?;
        }
        for declared in values {
            Self::push(&mut self.values, declared)?;
        }
        Ok(())
    }

    fn push(
        table: &mut Vec<Declared>,
        declared: Declared,
    ) -> Result<(), Error> {
        if let Some(first) = table.iter().find(|d| d.name == declared.name) {
            return Err(Error {
                span: declared.span,
                reason: Reason::Duplicate { first: first.span },
            });
        }
        table.push(declared);
        Ok(())
    }
}

/// A name a declaration file puts in scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub name: String,
    pub public: bool,
    pub kind: Kind,
    /// The span of the declared name.
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `type Name<params> { … }`; a record's constructor is its own name,
    /// a sum's are its uppercase members, an opaque type has none.
    Type {
        resource: bool,
        parameters: usize,
        opaque: bool,
        constructors: Vec<String>,
    },
    /// `name: A -> B -> C`: one spot per parameter, then the result.
    Function {
        constraints: usize,
        spots: Vec<Spot>,
        result: TypeId,
    },
    /// `name: T` with no parameters.
    Value { ty: TypeId },
    /// `name = value` in a declaration file.
    Binding { annotation: Option<TypeId> },
}

/// One parameter of a declared function; labeled when written
/// `(name: type = default)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub name: Option<String>,
    pub ty: TypeId,
    pub default: Option<ExprId>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: Span,
    pub reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A host module name is not a dotted path of lowercase names.
    ModuleName {
        name: String,
    },
    /// A submodule and a value would occupy the same namespace member.
    ModuleConflict {
        name: String,
    },
    Spots {
        problem: SpotProblem,
    },
    ClaimBlock {
        problem: String,
    },
    /// A declaration doc names a spot absent from its signature.
    UnknownDocSpot {
        name: String,
    },
    Syntax(parse::Reason),
    /// A line that is neither an item nor a `name = value` binding.
    NotDeclaration,
    /// A marked resource must be an opaque Host/commands declaration.
    ResourceForm,
    /// A second declaration of a name in the same namespace.
    Duplicate {
        first: Span,
    },
    /// Two types in one declaration namespace declare the same value constructor.
    ConstructorCollision {
        name: String,
        first_type: String,
        second_type: String,
        first: Span,
    },
    /// A form the parser accepts that the checker does not check yet.
    NotYetChecked {
        form: Unchecked,
    },
}

/// Resource headers stay opaque and belong only to Host registration.
fn resource_errors(namespace: &Namespace) -> Vec<Error> {
    let mut errors = Vec::new();
    for entry in &namespace.module.entries {
        if let Entry::Item(Item {
            kind:
                ItemKind::Type {
                    resource: true,
                    name: declared,
                    base,
                    members,
                    limits,
                    ..
                },
            ..
        }) = entry
        {
            if namespace.name == "prelude"
                || base.is_some()
                || members.is_some()
                || !limits.is_empty()
            {
                errors.push(Error {
                    span: declared.span,
                    reason: Reason::ResourceForm,
                });
            }
        }
    }
    errors
}

/// Constructors share a local value namespace, even when their owners are private.
/// Keep this separate from explicit values and from declarations in other sources.
fn constructor_collisions(module: &Module) -> Vec<Error> {
    let mut errors = Vec::new();
    let mut owners = Vec::new();
    let mut seen: Vec<(&Name, &Name)> = Vec::new();
    for entry in &module.entries {
        let Entry::Item(Item {
            kind:
                ItemKind::Type {
                    name: owner,
                    members,
                    ..
                },
            ..
        }) = entry
        else {
            continue;
        };
        let owner_name = last(owner);
        // Existing type-name validation owns duplicate headers.
        if owners.contains(&owner_name) {
            continue;
        }
        owners.push(owner_name);
        let Some(members) = members else { continue };
        let mut names: Vec<_> = members
            .iter()
            .filter_map(|member| match member {
                Member::Constructor { name, .. } => Some(name),
                Member::Field(_) => None,
            })
            .collect();
        if names.is_empty() {
            names.push(owner);
        }
        for name in names {
            if let Some((first_owner, first)) =
                seen.iter().find(|(_, prior)| last(prior) == last(name))
            {
                if last(first_owner) != last(owner) {
                    errors.push(Error {
                        span: name.span,
                        reason: Reason::ConstructorCollision {
                            name: last(name),
                            first_type: last(first_owner),
                            second_type: last(owner),
                            first: first.span,
                        },
                    });
                }
            } else {
                seen.push((owner, name));
            }
        }
    }
    errors
}

/// The types and the values one item declares.
fn item_names(module: &Module, item: &Item) -> (Vec<Declared>, Vec<Declared>) {
    let mut types = Vec::new();
    let mut values = Vec::new();
    match &item.kind {
        ItemKind::Use { .. } | ItemKind::Impl { .. } => {}
        ItemKind::Type {
            resource,
            name,
            parameters,
            base,
            members,
            limits,
        } => types.push(Declared {
            name: last(name),
            public: item.public,
            kind: Kind::Type {
                resource: *resource,
                parameters: parameters.len(),
                opaque: members.is_none()
                    && base.is_none()
                    && limits.is_empty(),
                constructors: members
                    .as_ref()
                    .map_or_else(Vec::new, |members| {
                        constructors(name, members)
                    }),
            },
            span: name.span,
        }),
        ItemKind::Declare { name, ty } => values.push(Declared {
            name: last(name),
            public: item.public,
            kind: signature(module, *ty),
            span: name.span,
        }),
    }
    (types, values)
}

/// A line in a declaration file must be `name = value`.
fn binding(module: &Module, id: ExprId) -> Result<Declared, Error> {
    let expr = &module.expressions[id.0];
    let ExprKind::Bind {
        public,
        pattern,
        annotation,
        ..
    } = &expr.kind
    else {
        return Err(Error {
            span: expr.span,
            reason: Reason::NotDeclaration,
        });
    };
    let pattern = &module.patterns[pattern.0];
    let PatternKind::Name(name) = &pattern.kind else {
        return Err(Error {
            span: pattern.span,
            reason: Reason::NotDeclaration,
        });
    };
    Ok(Declared {
        name: last(name),
        public: *public,
        kind: Kind::Binding {
            annotation: *annotation,
        },
        span: name.span,
    })
}

fn last(name: &Name) -> String {
    name.segments.last().cloned().unwrap_or_default()
}

fn constructors(name: &Name, members: &[Member]) -> Vec<String> {
    let sum: Vec<_> = members
        .iter()
        .filter_map(|member| match member {
            Member::Constructor { name, .. } => Some(last(name)),
            Member::Field(_) => None,
        })
        .collect();
    if sum.is_empty() {
        alloc::vec![last(name)]
    } else {
        sum
    }
}

fn signature(module: &Module, ty: TypeId) -> Kind {
    let mut current = ty;
    let mut constraints = 0;
    if let Some(TypeKind::Qualified {
        constraints: list,
        ty,
    }) = module.ty(current).map(|ty| &ty.kind)
    {
        constraints = list.len();
        current = *ty;
    }
    let mut spots = Vec::new();
    while let Some(TypeKind::Function { parameter, result }) =
        module.ty(current).map(|ty| &ty.kind)
    {
        spots.push(spot(module, *parameter));
        current = *result;
    }
    if spots.is_empty() && constraints == 0 {
        Kind::Value { ty: current }
    } else {
        Kind::Function {
            constraints,
            spots,
            result: current,
        }
    }
}

fn spot(module: &Module, id: TypeId) -> Spot {
    let node = &module.types[id.0];
    match &node.kind {
        TypeKind::Labeled { name, ty, default } => Spot {
            name: Some(last(name)),
            ty: *ty,
            default: *default,
            span: node.span,
        },
        _ => Spot {
            name: None,
            ty: id,
            default: None,
            span: node.span,
        },
    }
}
