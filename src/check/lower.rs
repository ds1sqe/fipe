//! Index the semantic identities already fixed by checking.
mod calls;
mod defaults;
mod emit;
pub use emit::file_order;
mod expressions;
mod gates;
mod patterns;
mod session;

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
    pub tables: Rc<RefCell<ir::Tables>>,
    namespaces: BTreeMap<Origin, usize>,
    meanings: BTreeMap<TypeName, usize>,
    parameters: BTreeMap<(Origin, Parameter), usize>,
    definitions: BTreeMap<(Origin, crate::token::Span), usize>,
    slots: BTreeMap<(Origin, crate::token::Span), usize>,
    gates: BTreeMap<TypeName, Vec<usize>>,
    shapes: BTreeMap<TypeName, (Origin, Shape)>,
    declarations: BTreeMap<(Origin, crate::token::Span), usize>,
    dispatch: BTreeMap<(String, bool), usize>,
    constant_defaults: Vec<usize>,
    home: Option<Origin>,
    visible: BTreeMap<crate::token::Span, usize>,
    session_variables: BTreeMap<(Origin, u32), usize>,
    weak_slots: Vec<usize>,
    emitted: Vec<(Origin, Vec<crate::ast::ExprId>)>,
    source_modules: BTreeMap<Origin, Rc<crate::ast::Module>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Parameter {
    Variable(u32),
    Rigid(String),
}

impl Index {
    pub fn namespace(&mut self, origin: &Origin) -> usize {
        if let Some(index) = self.namespaces.get(origin) {
            return *index;
        }
        let index = self.tables.borrow().namespaces.len();
        let label = match origin {
            Origin::This => String::from("main"),
            Origin::Prelude => String::from("prelude"),
            Origin::Commands => String::from("commands"),
            Origin::Host(name) | Origin::File(name) => name.clone(),
            Origin::Line(line) => alloc::format!("line {}", line + 1),
        };
        self.tables.borrow_mut().namespaces.push(label);
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
        let index = self.tables.borrow().meanings.len();
        self.tables.borrow_mut().meanings.push(ir::Meaning {
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
        match ty {
            Type::Unit => ir::Type::Unit,
            Type::Number(n) => ir::Type::Number(n.clone()),
            Type::Var(var) => self
                .session_variables
                .get(&(home.clone(), var.0))
                .copied()
                .map_or_else(
                    || {
                        ir::Type::Parameter(
                            self.parameter(home, Parameter::Variable(var.0)),
                        )
                    },
                    ir::Type::Session,
                ),
            Type::Rigid(name) => ir::Type::Parameter(
                self.parameter(home, Parameter::Rigid(name.clone())),
            ),
            Type::Named { name, arguments } => ir::Type::Meaning {
                index: self.meaning(home, name),
                arguments: arguments
                    .iter()
                    .map(|ty| self.ty(home, ty))
                    .collect(),
            },
            Type::Limited { meaning, .. } => self.ty(home, meaning),
            Type::Function { parameter, result } => ir::Type::Function {
                parameter: alloc::boxed::Box::new(self.ty(home, parameter)),
                result: alloc::boxed::Box::new(self.ty(home, result)),
            },
            Type::Array { element, length } => ir::Type::Array {
                element: alloc::boxed::Box::new(self.ty(home, element)),
                length: alloc::boxed::Box::new(self.ty(home, length)),
            },
            Type::Tuple(parts) => ir::Type::Tuple(self.parts(home, parts)),
            Type::Either(parts) => ir::Type::Packed(self.parts(home, parts)),
            Type::Sum(parts) => ir::Type::Sum(self.parts(home, parts)),
            Type::Both(parts) => ir::Type::All(self.parts(home, parts)),
            Type::Not(inner) => {
                ir::Type::Not(alloc::boxed::Box::new(self.ty(home, inner)))
            }
        }
    }

    fn parts(&mut self, home: &Origin, parts: &[Type]) -> Vec<ir::Type> {
        parts.iter().map(|ty| self.ty(home, ty)).collect()
    }

    pub fn shapes(&mut self, home: &Origin, shapes: &[Shape]) {
        for shape in shapes {
            if let Type::Named { name, .. } = &shape.ty {
                let key = TypeName {
                    origin: home_origin(home, &name.origin),
                    name: name.name.clone(),
                };
                self.shapes.insert(key, (home.clone(), shape.clone()));
            }
            let Type::Named { name, arguments } = &shape.ty else {
                unreachable!("checked layouts name their declaration")
            };
            let index = self.meaning(home, name);
            if !self.tables.borrow().meanings[index].constructors.is_empty() {
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
            self.tables.borrow_mut().meanings[index].parameters = parameters;
            self.tables.borrow_mut().meanings[index].constructors =
                constructors;
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

#[derive(Clone)]
struct Source<'a> {
    declarations: &'a crate::decl::Declarations,
    files: &'a [(String, crate::ast::Module)],
    module: Rc<crate::ast::Module>,
    resolved: Rc<crate::resolve::Resolved>,
    typed: Rc<crate::typed::Typed>,
    home: Origin,
    roots: Vec<crate::ast::ExprId>,
}

struct Scope {
    receivers: RefCell<BTreeMap<crate::token::Span, usize>>,
    nodes: Vec<Option<usize>>,
    slots: BTreeMap<crate::token::Span, usize>,
}

impl Index {
    fn slot(
        &mut self,
        home: &Origin,
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
        let ty =
            self.ty(home, &crate::types::replace(&scheme.ty, &replacements));
        let parameters: Vec<_> = body_parameters
            .iter()
            .map(|ty| {
                let ir::Type::Parameter(index) = self.ty(home, ty) else {
                    unreachable!("generic body parameter")
                };
                index
            })
            .collect();
        let hidden = hidden.iter().map(|index| parameters[*index]).collect();
        let index = self.tables.borrow().slots.len();
        self.tables.borrow_mut().slots.push(ir::Slot {
            ty,
            parameters,
            hidden,
            definition: None,
        });
        index
    }

    fn reserve(&mut self, source: &Source<'_>) -> Scope {
        let namespace = self.namespace(&source.home);
        self.shapes(&source.home, &source.typed.shapes);
        let mut nodes = alloc::vec![None; source.module.expressions.len()];
        for (index, expression) in source.module.expressions.iter().enumerate()
        {
            let Some(ty) = source.typed.expr(crate::ast::ExprId(index)) else {
                continue;
            };
            let ty = self.ty(&source.home, ty);
            let node = self.tables.borrow().nodes.len();
            self.tables.borrow_mut().nodes.push(ir::Node {
                namespace,
                span: expression.span,
                ty,
                kind: ir::Kind::Block(Vec::new()),
            });
            nodes[index] = Some(node);
        }
        let mut slots = self.visible.clone();
        for ((start, end), scheme) in &source.typed.schemes {
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
            let existing = self
                .slots
                .get(&(source.home.clone(), site))
                .copied()
                .or_else(|| self.visible.get(&site).copied());
            let slot = existing.unwrap_or_else(|| {
                self.slot(&source.home, scheme, hidden, &parameters)
            });
            slots.insert(site, slot);
        }
        Scope {
            nodes,
            slots,
            receivers: RefCell::new(BTreeMap::new()),
        }
    }

    fn node(
        &mut self,
        home: &Origin,
        span: crate::token::Span,
        ty: &Type,
        kind: ir::Kind,
    ) -> usize {
        let namespace = self.namespace(home);
        let ty = self.ty(home, ty);
        let index = self.tables.borrow().nodes.len();
        self.tables.borrow_mut().nodes.push(ir::Node {
            namespace,
            span,
            ty,
            kind,
        });
        index
    }
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
