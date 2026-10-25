//! Index the semantic identities already fixed by checking.
mod calls;
mod defaults;
mod emit;
pub use emit::file_order;
mod expressions;
mod gates;
mod patterns;
mod session;
pub use session::Journal;

use crate::{
    ir,
    resolve::Origin,
    typed::Shape,
    types::{Type, TypeName},
};
use alloc::{collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;

#[derive(Clone, Debug, Default)]
pub struct Index {
    store: session::Store,
    namespaces: BTreeMap<Origin, usize>,
    meanings: BTreeMap<TypeName, usize>,
    parameters: BTreeMap<(Origin, Parameter), usize>,
    definitions: BTreeMap<(Origin, crate::token::Span), usize>,
    gates: BTreeMap<TypeName, Vec<usize>>,
    declarations: BTreeMap<(Origin, crate::token::Span), usize>,
    dispatch: BTreeMap<(String, bool), usize>,
    constant_defaults: Vec<usize>,
    home: Option<Origin>,
    /// A line's own variables that became weak, by slot.
    session_variables: BTreeMap<(Origin, u32), usize>,
    /// How many weak variables each line started with: a variable below
    /// that is the earlier weak variable of that number.
    weak_before: BTreeMap<Origin, usize>,
    weak_slots: Vec<usize>,
    emitted: Vec<(Origin, Vec<crate::ast::ExprId>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Parameter {
    Variable(u32),
    Rigid(String),
}

impl Index {
    /// The session's tables, for reading.
    pub fn tables(&self) -> core::cell::Ref<'_, ir::Tables> {
        self.store.tables()
    }

    pub fn namespace(&mut self, origin: &Origin) -> usize {
        if let Some(index) = self.namespaces.get(origin) {
            return *index;
        }
        let label = match origin {
            Origin::This => String::from("main"),
            Origin::Prelude => String::from("prelude"),
            Origin::Commands => String::from("commands"),
            Origin::Host(name) | Origin::File(name) => name.clone(),
            Origin::Line(line) => alloc::format!("line {}", line + 1),
        };
        let index = self.store.push_namespace(label);
        self.namespaces.insert(origin.clone(), index);
        index
    }

    fn meaning(&mut self, home: &Origin, name: &TypeName) -> usize {
        let name = TypeName {
            origin: home_origin(home, &name.origin),
            name: name.name.clone(),
        };
        if let Some(index) = self.meanings.get(&name) {
            return *index;
        }
        let namespace = self.namespace(&name.origin);
        let index = self.store.push_meaning(ir::Meaning {
            resource: false,
            label: name.name.clone(),
            namespace,
            parameters: Vec::new(),
            constructors: Vec::new(),
        });
        self.meanings.insert(name, index);
        index
    }

    fn parameter(&mut self, home: &Origin, parameter: Parameter) -> usize {
        let next = self.parameters.len();
        *self
            .parameters
            .entry((home.clone(), parameter))
            .or_insert(next)
    }

    pub fn ty(&mut self, home: &Origin, ty: &Type) -> ir::Type {
        self.type_in(home, home, ty)
    }

    fn source_ty(&mut self, source: &Source<'_>, ty: &Type) -> ir::Type {
        self.type_in(&source.home, &source.parameters_home, ty)
    }

    fn type_in(
        &mut self,
        home: &Origin,
        parameters_home: &Origin,
        ty: &Type,
    ) -> ir::Type {
        self.type_input(TypeInput {
            ty,
            home,
            parameters_home,
            bindings: &[],
        })
    }

    fn type_input(&mut self, input: TypeInput<'_>) -> ir::Type {
        if let Some(actual) = input.bound() {
            return self.type_input(actual);
        }
        let TypeInput {
            ty,
            home,
            parameters_home,
            ..
        } = input;
        match ty {
            Type::Unit => ir::Type::Unit,
            Type::Number(n) => ir::Type::Number(n.clone()),
            Type::Var(var) => self
                .session_variables
                .get(&(parameters_home.clone(), var.0))
                .copied()
                .or_else(|| {
                    let before = *self.weak_before.get(parameters_home)?;
                    ((var.0 as usize) < before)
                        .then(|| self.weak_slots[var.0 as usize])
                })
                .map_or_else(
                    || {
                        ir::Type::Parameter(self.parameter(
                            parameters_home,
                            Parameter::Variable(var.0),
                        ))
                    },
                    ir::Type::Session,
                ),
            Type::Rigid(name) => ir::Type::Parameter(
                self.parameter(parameters_home, Parameter::Rigid(name.clone())),
            ),
            Type::Named { name, arguments } => ir::Type::Meaning {
                index: self.meaning(home, name),
                arguments: arguments
                    .iter()
                    .map(|ty| self.type_input(input.child(ty)))
                    .collect(),
            },
            Type::Limited { meaning, .. } => {
                self.type_input(input.child(meaning))
            }
            Type::Function { parameter, result } => ir::Type::Function {
                parameter: alloc::boxed::Box::new(
                    self.type_input(input.child(parameter)),
                ),
                result: alloc::boxed::Box::new(
                    self.type_input(input.child(result)),
                ),
            },
            Type::Array { element, length } => ir::Type::Array {
                element: alloc::boxed::Box::new(
                    self.type_input(input.child(element)),
                ),
                length: alloc::boxed::Box::new(
                    self.type_input(input.child(length)),
                ),
            },
            Type::Tuple(parts) => ir::Type::Tuple(
                parts
                    .iter()
                    .map(|ty| self.type_input(input.child(ty)))
                    .collect(),
            ),
            Type::Either(parts) => ir::Type::Packed(
                parts
                    .iter()
                    .map(|ty| self.type_input(input.child(ty)))
                    .collect(),
            ),
            Type::Sum(parts) => ir::Type::Sum(
                parts
                    .iter()
                    .map(|ty| self.type_input(input.child(ty)))
                    .collect(),
            ),
            Type::Both(parts) => ir::Type::All(
                parts
                    .iter()
                    .map(|ty| self.type_input(input.child(ty)))
                    .collect(),
            ),
            Type::Not(inner) => ir::Type::Not(alloc::boxed::Box::new(
                self.type_input(input.child(inner)),
            )),
        }
    }

    pub fn shapes(&mut self, home: &Origin, shapes: &[Shape]) {
        for shape in shapes {
            if let Type::Named { name, .. } = &shape.ty {
                let key = TypeName {
                    origin: home_origin(home, &name.origin),
                    name: name.name.clone(),
                };
                // A checked shape is complete even with no constructors.
                // Keep its first home and generic variable correspondence.
                if self.store.shapes().contains_key(&key) {
                    continue;
                }
                self.store.insert_shape(key, (home.clone(), shape.clone()));
            }
            let Type::Named { name, arguments } = &shape.ty else {
                unreachable!("checked layouts name their declaration")
            };
            let index = self.meaning(home, name);
            if !self.store.tables().meanings[index].constructors.is_empty() {
                continue;
            }
            let parameters = arguments
                .iter()
                .map(|ty| {
                    let ir::Type::Parameter(parameter) = self.ty(home, ty)
                    else {
                        unreachable!("layout parameters are generic")
                    };
                    parameter
                })
                .collect();
            let constructors = shape
                .constructors
                .iter()
                .map(|constructor| ir::Constructor {
                    label: constructor.name.clone(),
                    fields: constructor
                        .fields
                        .iter()
                        .map(|field| ir::Field {
                            label: field.name.clone(),
                            ty: self.ty(home, &field.ty),
                        })
                        .collect(),
                })
                .collect();
            let mut meaning = self.store.meaning_mut(index);
            meaning.resource = shape.resource;
            meaning.parameters = parameters;
            meaning.constructors = constructors;
        }
    }
}

fn home_origin(home: &Origin, origin: &Origin) -> Origin {
    if *origin == Origin::This {
        home.clone()
    } else {
        origin.clone()
    }
}

fn form(literal: &crate::ast::Literal) -> ir::Form {
    use crate::ast::Literal;
    match literal {
        Literal::Unit => ir::Form::Unit,
        Literal::Int {
            negative,
            magnitude,
            radix,
            digits,
        } => ir::Form::Whole {
            negative: *negative,
            magnitude: magnitude.clone(),
            radix: *radix,
            digits: *digits,
        },
        Literal::Float(text) => fraction(text),
        Literal::Str { value, .. } => ir::Form::Text(value.clone()),
        Literal::Affixed(value) => ir::Form::Bytes(
            crate::resolve::bytes(value)
                .expect("checked byte literal")
                .to_vec(),
        ),
    }
}

fn fraction(text: &str) -> ir::Form {
    use crate::{limits::known::Integer, token::Magnitude};
    let (mantissa, exponent) =
        text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches(['-', '+']);
    let places = mantissa.split_once('.').map_or(0, |(_, part)| {
        part.bytes().filter(|byte| *byte != b'_').count()
    });
    let digits: String = mantissa.chars().filter(|ch| *ch != '.').collect();
    let exponent = Integer::new(
        exponent.starts_with('-'),
        Magnitude::parse(exponent.trim_start_matches(['-', '+']), 10)
            .expect("checked fractional exponent"),
    )
    .add(&Integer::new(
        true,
        Magnitude::from(u64::try_from(places).expect("source length fits u64")),
    ));
    ir::Form::Fraction {
        negative,
        digits: Magnitude::parse(&digits, 10)
            .expect("checked fractional digits"),
        exponent_negative: exponent.negative,
        exponent: exponent.magnitude,
    }
}

#[derive(Clone, Copy)]
struct TypeInput<'a> {
    ty: &'a Type,
    home: &'a Origin,
    parameters_home: &'a Origin,
    bindings: &'a [(crate::types::Var, Self)],
}

impl<'a> TypeInput<'a> {
    const fn child(self, ty: &'a Type) -> Self {
        Self { ty, ..self }
    }

    fn bound(self) -> Option<Self> {
        let Type::Var(var) = self.ty else {
            return None;
        };
        self.bindings
            .iter()
            .find_map(|(formal, actual)| (formal == var).then_some(*actual))
    }
}

#[derive(Clone)]
struct Source<'a> {
    initializes_file: bool,
    declarations: &'a crate::decl::Declarations,
    files: &'a [(String, crate::ast::Module)],
    module: Rc<crate::ast::Module>,
    resolved: Rc<crate::resolve::Resolved>,
    typed: Rc<crate::typed::Typed>,
    home: Origin,
    parameters_home: Origin,
    roots: Vec<crate::ast::ExprId>,
}

/// A source closure prepared without changing the session's runtime index.
pub struct Prepared<'a> {
    sources: Vec<Source<'a>>,
    definitions: Vec<Vec<DefinitionInput>>,
    contributions: Vec<Contribution>,
}

struct Contribution {
    nodes: crate::infer::SourceNodes,
    owned: alloc::collections::BTreeSet<crate::token::Span>,
    lexical: bool,
    replay: bool,
}

struct Readiness<'a, 'module> {
    sources: &'a [Source<'module>],
    contributions: &'a [Contribution],
    owned: &'a alloc::collections::BTreeSet<(Origin, crate::token::Span)>,
    definitions: &'a alloc::collections::BTreeSet<(Origin, crate::token::Span)>,
}

struct DefinitionInput {
    site: crate::token::Span,
    body_span: crate::token::Span,
    label: String,
    expression: Option<crate::ast::ExprId>,
    arms: Vec<crate::ast::Arm>,
    fill: bool,
}

struct Scope {
    receiving_calls: RefCell<BTreeMap<crate::token::Span, usize>>,
    payload_casts: BTreeMap<usize, (crate::token::Span, crate::ast::ExprId)>,
    receivers: RefCell<BTreeMap<crate::token::Span, usize>>,
    nodes: Vec<Option<usize>>,
    slots: BTreeMap<crate::token::Span, usize>,
}

impl Index {
    fn slot(
        &mut self,
        home: &Origin,
        parameters_home: &Origin,
        site: crate::token::Span,
        scheme: &crate::types::Scheme,
        hidden: &[usize],
        body_parameters: &[Type],
    ) -> usize {
        let replacements: Vec<_> = scheme
            .variables
            .iter()
            .copied()
            .zip(body_parameters.iter().cloned())
            .collect();
        let ty = self.type_in(
            home,
            parameters_home,
            &crate::types::replace(&scheme.ty, &replacements),
        );
        let parameters: Vec<_> = body_parameters
            .iter()
            .map(|ty| {
                let ir::Type::Parameter(index) =
                    self.type_in(home, parameters_home, ty)
                else {
                    unreachable!("generic body parameter")
                };
                index
            })
            .collect();
        let hidden = hidden.iter().map(|index| parameters[*index]).collect();
        let namespace = self.namespace(home);
        self.store.push_slot(ir::Slot {
            ty,
            parameters,
            hidden,
            definition: None,
            name: Some((namespace, site)),
        })
    }

    fn reserve(
        &mut self,
        source: &Source<'_>,
        contribution: &Contribution,
        allocated: &BTreeMap<(Origin, crate::token::Span), usize>,
    ) -> Scope {
        let namespace = self.namespace(&source.home);
        self.shapes(&source.home, &source.typed.shapes);
        let unused = conversion_heads(source);
        let mut nodes = alloc::vec![None; source.module.expressions.len()];
        for (index, expression) in source.module.expressions.iter().enumerate()
        {
            if unused[index] || !contribution.nodes.expressions.contains(&index)
            {
                continue;
            }
            let Some(ty) = source.typed.expr(crate::ast::ExprId(index)) else {
                continue;
            };
            let ty = self.source_ty(source, ty);
            let node = self.store.push_node(ir::Node {
                namespace,
                span: expression.span,
                ty,
                kind: ir::Kind::Block(Vec::new()),
            });
            nodes[index] = Some(node);
        }
        let mut slots = if contribution.lexical {
            self.store.visible().clone()
        } else {
            BTreeMap::new()
        };
        for ((start, end), scheme) in &source.typed.schemes {
            let site = crate::token::Span {
                start: *start,
                end: *end,
            };
            if !contribution.owned.contains(&site) {
                continue;
            }
            let hidden = source
                .typed
                .hidden
                .get(&(*start, *end))
                .map_or(&[][..], Vec::as_slice);
            let parameters = source
                .typed
                .body_parameters
                .get(&(*start, *end))
                .cloned()
                .unwrap_or_else(|| {
                    scheme.variables.iter().map(|var| Type::Var(*var)).collect()
                });
            let site = crate::token::Span {
                start: *start,
                end: *end,
            };
            let existing = if contribution.replay {
                None
            } else {
                allocated
                    .get(&(source.home.clone(), site))
                    .copied()
                    .or_else(|| {
                        self.store
                            .slots()
                            .get(&(source.home.clone(), site))
                            .copied()
                    })
                    .or_else(|| {
                        contribution
                            .lexical
                            .then(|| self.store.visible().get(&site).copied())
                            .flatten()
                    })
            };
            let slot = existing.unwrap_or_else(|| {
                self.slot(
                    &source.home,
                    &source.parameters_home,
                    site,
                    scheme,
                    hidden,
                    &parameters,
                )
            });
            slots.insert(site, slot);
        }
        let payload_casts = source.module.expressions.iter().filter_map(|expression| {
            let crate::ast::ExprKind::Pipe { op: crate::ast::PipeOp::Good | crate::ast::PipeOp::Bad, left, right, .. } = &expression.kind else { return None };
            let stage = source.module.expressions[right.0].span;
            let bound = source.typed.bound.get(&(stage.start, stage.end))?;
            bound.supplies.iter().any(|(_, supply)| matches!(supply,
                crate::typed::Supply::Pipe(site) if *site == source.module.expressions[left.0].span
            )).then_some((left.0, (stage, *right)))
        }).collect();
        Scope {
            receiving_calls: RefCell::new(BTreeMap::new()),
            payload_casts,
            nodes,
            slots,
            receivers: RefCell::new(BTreeMap::new()),
        }
    }

    fn source_node(
        &mut self,
        source: &Source<'_>,
        span: crate::token::Span,
        ty: &Type,
        kind: ir::Kind,
    ) -> usize {
        let namespace = self.namespace(&source.home);
        let ty = self.source_ty(source, ty);
        self.store.push_node(ir::Node {
            namespace,
            span,
            ty,
            kind,
        })
    }
}

// A conversion call selects its operation after its source supply is known;
// its direct static head has a type, but no independently executable operation.
fn conversion_heads(source: &Source<'_>) -> Vec<bool> {
    use crate::ast::ExprKind;
    let typed = &source.typed;
    let mut unused = alloc::vec![false; source.module.expressions.len()];
    for expression in &source.module.expressions {
        let ExprKind::Call { callee, .. } = &expression.kind else {
            continue;
        };
        let call = (expression.span.start, expression.span.end);
        if !typed.conversions.contains_key(&call)
            || !typed.selected.contains_key(&call)
            || !typed.bound.contains_key(&call)
        {
            continue;
        }
        let head = &source.module.expressions[callee.0];
        if !matches!(head.kind, ExprKind::Name(_) | ExprKind::Instance { .. })
            || !matches!(
                source.resolved.expr(*callee),
                Some(crate::resolve::Target::TypeMember { .. })
            )
        {
            continue;
        }
        let site = (head.span.start, head.span.end);
        if typed.selected.contains_key(&site)
            || typed.conversions.contains_key(&site)
            || typed.bound.contains_key(&site)
            || typed.accesses.contains_key(&site)
            || typed.roots.contains_key(&site)
            || typed.derived_defaults.contains_key(&site)
            || typed.casts[callee.0].is_some()
            || source.roots.contains(callee)
            || typed
                .tests
                .iter()
                .any(|test| test.expression.is_none() && test.value == *callee)
            || typed.checks.iter().any(|check| {
                check.place_origin == Origin::This
                    && check.value.map_or(check.place == head.span, |value| {
                        value == *callee
                    })
            })
        {
            continue;
        }
        unused[callee.0] = true;
    }
    unused
}

impl Scope {
    fn expression(&self, id: crate::ast::ExprId) -> usize {
        self.nodes[id.0].expect("executable expression was checked")
    }
}

struct Body {
    definition: usize,
    expression: Option<crate::ast::ExprId>,
    arms: Vec<crate::ast::Arm>,
}

fn lines(module: &crate::ast::Module) -> Vec<crate::ast::ExprId> {
    module
        .entries
        .iter()
        .filter_map(|entry| match entry {
            crate::ast::Entry::Line(id) => Some(*id),
            crate::ast::Entry::Item(_) => None,
        })
        .collect()
}

fn owner<'a>(
    declarations: &'a crate::decl::Declarations,
    files: &'a [(String, crate::ast::Module)],
    origin: &Origin,
) -> &'a crate::ast::Module {
    match origin {
        Origin::Prelude => &declarations.prelude.module,
        Origin::Commands => {
            &declarations
                .commands
                .as_ref()
                .expect("command namespace")
                .module
        }
        Origin::Host(name) => {
            &declarations
                .get_module(name)
                .expect("checked namespace")
                .module
        }
        Origin::File(name) => {
            &files
                .iter()
                .find(|(stem, _)| stem == name)
                .expect("checked file")
                .1
        }
        _ => unreachable!("supplied expression owner"),
    }
}

fn free_parameters(ty: &ir::Type, parameters: &mut Vec<usize>) {
    match ty {
        ir::Type::Parameter(index) => {
            if !parameters.contains(index) {
                parameters.push(*index);
            }
        }
        ir::Type::Meaning {
            arguments: parts, ..
        }
        | ir::Type::Tuple(parts)
        | ir::Type::Packed(parts)
        | ir::Type::All(parts)
        | ir::Type::Sum(parts) => {
            for ty in parts {
                free_parameters(ty, parameters);
            }
        }
        ir::Type::Function { parameter, result }
        | ir::Type::Array {
            element: parameter,
            length: result,
        } => {
            free_parameters(parameter, parameters);
            free_parameters(result, parameters);
        }
        ir::Type::Not(inner) => free_parameters(inner, parameters),
        _ => (),
    }
}

#[cfg(test)]
mod source_binding_tests {
    use super::*;

    #[test]
    fn execution_binders_and_weak_slots_do_not_change_nominal_home() {
        let mut index = Index::default();
        let caller = Origin::Line(3);
        let host = Origin::Host(String::from("host"));
        let other = Origin::Host(String::from("other"));
        let variable = Type::Var(crate::types::Var(2));
        let rigid = Type::Rigid(String::from("a"));
        assert_eq!(
            index.type_in(&host, &caller, &variable),
            index.ty(&caller, &variable)
        );
        assert_eq!(
            index.type_in(&host, &caller, &rigid),
            index.ty(&caller, &rigid)
        );
        assert_ne!(index.ty(&host, &variable), index.ty(&caller, &variable));
        assert_ne!(index.ty(&host, &rigid), index.ty(&caller, &rigid));
        assert_ne!(index.ty(&host, &variable), index.ty(&other, &variable));
        index.session_variables.insert((caller.clone(), 2), 9);
        assert_eq!(
            index.type_in(&host, &caller, &variable),
            ir::Type::Session(9)
        );
        assert!(matches!(index.ty(&host, &variable), ir::Type::Parameter(_)));
        let nominal = Type::Named {
            name: TypeName {
                origin: Origin::This,
                name: String::from("W"),
            },
            arguments: alloc::vec![variable],
        };
        let ir::Type::Meaning {
            index: meaning,
            arguments,
        } = index.type_in(&host, &caller, &nominal)
        else {
            panic!("nominal")
        };
        assert_eq!(arguments, [ir::Type::Session(9)]);
        assert_eq!(
            index.store.tables().namespaces
                [index.store.tables().meanings[meaning].namespace],
            "host"
        );
    }

    #[test]
    fn replay_owned_default_parameter_slots_are_private_and_fresh() {
        let declarations = crate::decl::Declarations::new();
        let module = Rc::new(
            crate::parse::parse(crate::lex::lex("f = (copy = 7) => copy\nf"))
                .unwrap(),
        );
        let resolved = Rc::new(
            crate::resolve::resolve(&module, &declarations, &[]).unwrap(),
        );
        let typed = Rc::new(
            crate::infer::infer(&module, &resolved, &declarations, &[])
                .unwrap(),
        );
        let source = Source {
            initializes_file: false,
            declarations: &declarations,
            files: &[],
            roots: lines(&module),
            module,
            resolved,
            typed,
            home: Origin::Host(String::from("host")),
            parameters_home: Origin::This,
        };
        let mut index = Index::default();
        let mut contribution =
            Index::contribution(&source, &[], source.roots.clone(), false);
        contribution.replay = true;
        let site = source
            .module
            .patterns
            .iter()
            .find_map(|p| match &p.kind {
                crate::ast::PatternKind::Default { name, .. } => {
                    Some(name.span)
                }
                _ => None,
            })
            .unwrap();
        assert!(contribution.owned.contains(&site));
        let first = index.reserve(
            &source,
            &contribution,
            &alloc::collections::BTreeMap::default(),
        );
        index
            .store
            .insert_slot((source.home.clone(), site), first.slots[&site]);
        let second = index.reserve(
            &source,
            &contribution,
            &alloc::collections::BTreeMap::default(),
        );
        assert_ne!(first.slots[&site], second.slots[&site]);
        assert_eq!(
            index.store.slots()[&(source.home.clone(), site)],
            first.slots[&site]
        );
    }
}

#[cfg(test)]
mod shape_completion_tests {
    use super::*;
    use crate::{decl::Declarations, embed::Outcome, lex::lex, parse::parse};

    fn declarations() -> Declarations {
        let mut declarations = Declarations::new();
        declarations
            .module("layout", "pub type Empty<T>\npub value: Empty<Integer>")
            .unwrap();
        declarations
    }

    fn checked_shapes() -> (Vec<Shape>, TypeName) {
        let declarations = declarations();
        let module = parse(lex("use layout\nlayout.value")).unwrap();
        let resolved =
            crate::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed = crate::infer::infer(&module, &resolved, &declarations, &[])
            .unwrap();
        let key = typed
            .shapes
            .iter()
            .find_map(|shape| {
                let Type::Named { name, .. } = &shape.ty else {
                    return None;
                };
                (name.origin == Origin::Host("layout".into())
                    && name.name == "Empty")
                    .then_some(name.clone())
            })
            .expect("actual complete checked Empty layout");
        (typed.shapes, key)
    }

    #[test]
    fn completed_constructorless_shape_preserves_first_home_and_variables() {
        let (first, key) = checked_shapes();
        let (second, second_key) = checked_shapes();
        assert_eq!(key, second_key);
        let mut index = Index::default();
        let first_home = Origin::Line(0);
        index.shapes(&first_home, &first);
        let before = index.tables().meanings.clone();
        let stored = index.store.shapes().get(&key).unwrap().clone();
        assert_eq!(stored.0, first_home);
        let meaning = index.meanings[&key];
        assert!(before[meaning].constructors.is_empty());
        assert_eq!(before[meaning].parameters.len(), 1);
        let Type::Named { arguments, .. } = &stored.1.ty else {
            panic!("checked generic shape");
        };
        assert!(matches!(arguments.as_slice(), [Type::Var(_)]));
        index.shapes(&Origin::Line(1), &second);
        assert_eq!(index.tables().meanings, before);
        assert_eq!(index.store.shapes().get(&key), Some(&stored));
    }

    #[test]
    fn ordinary_session_repeated_source_keeps_constructorless_layouts() {
        let mut session = crate::session::Session::new(declarations());
        let Outcome::Done(first) = session.line("use layout\nlayout.value")
        else {
            panic!("no file request");
        };
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        let first = first.unit.unwrap();
        let tables = Rc::clone(&first.tables);
        let before = tables.borrow().meanings.clone();
        assert!(before.iter().any(|meaning| {
            meaning.label == "Empty"
                && meaning.parameters.len() == 1
                && meaning.constructors.is_empty()
        }));
        let Outcome::Done(second) = session.line("layout.value") else {
            panic!("no file request");
        };
        assert!(second.diagnostics.is_empty(), "{:?}", second.diagnostics);
        assert!(second.accepted.is_some());
        assert!(Rc::ptr_eq(&tables, &second.unit.unwrap().tables));
        assert_eq!(tables.borrow().meanings, before);
    }

    #[test]
    fn meaning_only_forward_placeholder_receives_its_first_complete_shape() {
        let (shapes, key) = checked_shapes();
        let mut index = Index::default();
        let home = Origin::Line(0);
        let meaning = index.meaning(&home, &key);
        assert!(!index.store.shapes().contains_key(&key));
        assert!(index.tables().meanings[meaning].parameters.is_empty());
        let before = index.tables().clone();
        for (line, keep_then_undo) in [(0, false), (1, true)] {
            index.begin();
            index.shapes(&Origin::Line(line), &shapes);
            assert_eq!(index.meanings[&key], meaning);
            assert_ne!(
                index.tables().meanings[meaning],
                before.meanings[meaning]
            );
            assert_eq!(index.tables().meanings[meaning].parameters.len(), 1);
            assert!(index.store.shapes().contains_key(&key));
            if keep_then_undo {
                let journal = index.commit();
                index.withdraw(journal);
            } else {
                index.rollback();
            }
            assert_eq!(index.meanings[&key], meaning);
            assert!(!index.store.shapes().contains_key(&key));
            assert_eq!(*index.tables(), before);
        }
        index.shapes(&home, &shapes);
        assert_eq!(index.meanings[&key], meaning);
        assert_eq!(index.tables().meanings[meaning].parameters.len(), 1);
        assert!(index.tables().meanings[meaning].constructors.is_empty());
        assert_eq!(index.store.shapes()[&key].0, home);
    }

    #[test]
    fn rollback_and_withdraw_remove_shape_completion_before_refill() {
        let (shapes, key) = checked_shapes();
        let mut index = Index::default();
        let before = index.tables().clone();
        for (line, keep_then_undo) in [(0, false), (1, true)] {
            index.begin();
            index.shapes(&Origin::Line(line), &shapes);
            assert!(index.store.shapes().contains_key(&key));
            let meaning = index.meanings[&key];
            assert_eq!(index.tables().meanings[meaning].parameters.len(), 1);
            if keep_then_undo {
                let journal = index.commit();
                index.withdraw(journal);
            } else {
                index.rollback();
            }
            assert!(!index.store.shapes().contains_key(&key));
            assert!(!index.meanings.contains_key(&key));
            assert_eq!(*index.tables(), before);
        }
        index.shapes(&Origin::Line(2), &shapes);
        assert_eq!(index.store.shapes()[&key].0, Origin::Line(2));
        let meaning = index.meanings[&key];
        assert_eq!(index.tables().meanings[meaning].parameters.len(), 1);
    }
}
