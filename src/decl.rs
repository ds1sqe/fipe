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

    /// A host module, replacing an earlier one of the same name.
    ///
    /// # Errors
    /// Syntax errors, lines that declare nothing, duplicate names, and
    /// forms the checker does not check yet.
    pub fn module(&mut self, name: &str, text: &str) -> Result<(), Vec<Error>> {
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
        self.modules.retain(|module| module.name != name);
        self.modules.push(namespace);
        Ok(())
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
        errors.extend(spots::check(&namespace.module));
        docs::check(&namespace.module, &mut errors);
        claims::check(&namespace.module, others, &mut errors);
        for index in 0..namespace.module.entries.len() {
            if let Err(error) = namespace.collect(index) {
                errors.push(error);
            }
        }
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
    /// A second declaration of a name in the same namespace.
    Duplicate {
        first: Span,
    },
    /// A form the parser accepts that the checker does not check yet.
    NotYetChecked {
        form: Unchecked,
    },
}

/// The types and the values one item declares.
fn item_names(module: &Module, item: &Item) -> (Vec<Declared>, Vec<Declared>) {
    let mut types = Vec::new();
    let mut values = Vec::new();
    match &item.kind {
        ItemKind::Use { .. } | ItemKind::Impl { .. } => {}
        ItemKind::Type {
            name,
            parameters,
            base,
            members,
            limits,
        } => types.push(Declared {
            name: last(name),
            public: item.public,
            kind: Kind::Type {
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
