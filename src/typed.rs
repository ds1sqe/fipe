//! The typed tree: a type per expression and a scheme per binding, over
//! a module and its [`Resolved`](crate::resolve::Resolved) names.

use crate::limits::known::Known;
pub use crate::limits::{
    Check, Frequency, Holding, Phase, Place, Reason as CheckReason,
};
use crate::{
    ast::ExprId,
    infer::Local,
    types::{Obligation, Scheme, Subst, Type},
};
use alloc::vec::Vec;

/// What inference learned about one module. Types are fully applied:
/// a variable left in them is unsolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typed {
    /// Declaration contracts at exact instantiated member uses, separate from meanings.
    pub member_contracts:
        alloc::collections::BTreeMap<MemberContractUse, MemberContract>,
    /// Actual source reads carried by each generalized binding obligation.
    pub member_contract_reads: alloc::collections::BTreeMap<
        (usize, usize),
        Vec<Vec<MemberContractRead>>,
    >,

    /// Physical record fields filled by each constructor call slot.
    pub constructor_fields:
        alloc::collections::BTreeMap<(usize, usize), Vec<usize>>,
    /// Explicit derived defaults lowered as checked construction plans.
    pub derived_defaults:
        alloc::collections::BTreeMap<(usize, usize), DefaultValue>,
    /// Adapters selected for explicit neutral conversions.
    pub conversions: alloc::collections::BTreeMap<(usize, usize), Conversion>,
    /// Equality operations for literal patterns, keyed by pattern index.
    pub pattern_equals: alloc::collections::BTreeMap<usize, Comparison>,
    /// Variables as written in a generic body, in scheme-variable order.
    pub body_parameters:
        alloc::collections::BTreeMap<(usize, usize), Vec<Type>>,
    /// Limit-test functions needed by gates and open entries.
    pub tests: Vec<Test>,
    /// Checked targets for tag-dispatched members.
    pub dispatch: Vec<Dispatch>,
    /// Member chains in source order, including intermediate applications.
    pub accesses: alloc::collections::BTreeMap<(usize, usize), Vec<Access>>,
    /// A type's member at the head of a member chain, keyed by the
    /// chain's span; `selected` there is the chain's last step.
    pub roots: alloc::collections::BTreeMap<(usize, usize), Root>,
    /// The line's own variables that became weak, by their weak number;
    /// a variable below `weak_before` is an earlier weak variable with
    /// that number.
    pub session_variables: alloc::collections::BTreeMap<u32, usize>,
    /// How many weak variables the session had before this line.
    pub(crate) weak_before: u32,
    /// Scheme-variable positions needed by receiverless members.
    pub hidden: alloc::collections::BTreeMap<(usize, usize), Vec<usize>>,
    /// Binding schemes before session freezing, keyed by declaration span.
    pub schemes: alloc::collections::BTreeMap<(usize, usize), Scheme>,
    pub(crate) pattern_types: alloc::collections::BTreeMap<usize, Type>,
    /// Generic declaration layouts used by this checked tree.
    pub shapes: Vec<Shape>,
    /// Checked supplied defaults, keyed by receiving call span and spot.
    pub default_instances:
        alloc::collections::BTreeMap<(usize, usize, usize), DeclaredExpression>,
    /// Checked supplied expressions used by defaults or host constants.
    pub declared: Vec<DeclaredExpression>,
    pub(crate) constructors: Vec<DeclaredConstructor>,
    /// Instantiated scheme meanings, in quantified-variable order.
    pub meaning_arguments:
        alloc::collections::BTreeMap<(usize, usize), Vec<Type>>,
    /// Implementations chosen for calls, keyed by stage span.
    pub selected: alloc::collections::BTreeMap<(usize, usize), Selected>,
    /// The implicit main after good-side binding lifts.
    pub main: Option<Type>,
    /// The callee before its recorded supplies and defaults.
    pub callee_types: alloc::collections::BTreeMap<(usize, usize), Type>,
    /// Outcome decisions keyed by the stage or operator span.
    pub rebuild_types: alloc::collections::BTreeMap<(usize, usize), Type>,
    pub rebuilds: alloc::collections::BTreeMap<(usize, usize), Rebuild>,
    pub modes: alloc::collections::BTreeMap<(usize, usize), Mode>,
    /// A conversion at each expression's receiving place, if any; boxed,
    /// since most expressions have none.
    pub casts: Vec<Option<alloc::boxed::Box<Cast>>>,
    /// Non-fatal call-spacing diagnostics, in module source coordinates.
    pub warnings: Vec<Warning>,
    /// Binding decisions keyed by the call or stage span in this module.
    pub bound: alloc::collections::BTreeMap<(usize, usize), Bound>,
    pub(crate) expressions: Vec<Option<Type>>,
    pub(crate) bindings: Vec<Option<Scheme>>,
    /// Obligations left on variables, such as `Ord a`.
    pub unsolved: Vec<Obligation>,
    /// Open entries to be tested when evaluation reaches their places.
    pub checks: Vec<Check>,
    pub(crate) known: Vec<Known>,
    pub(crate) supplied: alloc::collections::BTreeMap<
        (usize, usize),
        Vec<(alloc::string::String, crate::infer::Supply)>,
    >,
    pub(crate) facts: Vec<((usize, usize), Known)>,
    /// Every binding and item by its site, for a session's frame.
    pub(crate) frozen: Vec<((usize, usize), Local)>,
    /// The weak variables after this module: those its frozen
    /// monomorphic types share with the lines after it.
    pub(crate) weak: Subst,
}

impl Typed {
    /// The checked input of a pattern.
    #[must_use]
    pub fn pattern(&self, id: crate::ast::PatternId) -> Option<&Type> {
        self.pattern_types.get(&id.0)
    }

    /// Facts held of an expression, separate from its type.
    #[must_use]
    pub fn known(&self, id: ExprId) -> Option<&Known> {
        self.known.get(id.0)
    }

    /// The type of an expression; `None` for one inference never reached.
    #[must_use]
    pub fn expr(&self, id: ExprId) -> Option<&Type> {
        self.expressions.get(id.0).and_then(Option::as_ref)
    }

    /// The scheme of the binding `pattern = value` at `id`, for a name
    /// pattern.
    #[must_use]
    pub fn binding(&self, id: ExprId) -> Option<&Scheme> {
        self.bindings.get(id.0).and_then(Option::as_ref)
    }

    /// Every binding's scheme, in arena order.
    pub fn bindings(&self) -> impl Iterator<Item = (ExprId, &Scheme)> {
        self.bindings
            .iter()
            .enumerate()
            .filter_map(|(index, scheme)| {
                Some((ExprId(index), scheme.as_ref()?))
            })
    }
}

/// Both tuple and argument-list readings bind this call's spots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub span: crate::token::Span,
    pub style: crate::ast::CallStyle,
}

/// The checker binds once; evaluation consumes these decisions in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bound {
    /// Declaration sites and instantiated types of this call's spots.
    pub spots: Vec<(crate::token::Span, Type)>,
    /// Supplies in source order, with their zero-based declaration spot.
    pub supplies: Vec<(usize, Supply)>,
    /// Defaults in declaration order.
    pub defaults: Vec<(usize, DefaultValue)>,
    /// Unfilled spots in declaration order.
    pub open: Vec<usize>,
    /// Operands past the spot list, applied in order to the bound result
    /// when it is a function, each with the parameter it fills.
    pub applied: Vec<(Type, ExprId)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Supply {
    Expression(ExprId),
    Flag(crate::token::Span),
    Pipe(crate::token::Span),
    Head(crate::token::Span),
    /// One operand appended to its collected spot, in source order.
    Element(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    Record {
        ty: Type,
        fields: Vec<(crate::token::Span, Type, Self)>,
    },
    Array {
        ty: Type,
        element: alloc::boxed::Box<Self>,
    },
    Expression(alloc::boxed::Box<DeclaredExpression>),
    Prepared {
        selected: alloc::boxed::Box<Selected>,
        meanings: Vec<Type>,
        ty: Type,
    },
    Declared {
        origin: crate::resolve::Origin,
        value: ExprId,
    },
    Type(Type),
}

/// One row of the closed implicit conversion table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastRow {
    Number,
    Read,
    Show,
    Codec,
    Bytes,
    Pack,
}

/// A cast preserves the source meaning; its limits are entered afterward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cast {
    pub operation: Option<Comparison>,
    pub row: CastRow,
    pub from: Type,
    pub to: Type,
}

impl CastRow {
    #[must_use]
    pub const fn can_fail(self) -> bool {
        matches!(self, Self::Number | Self::Read | Self::Codec)
    }
}

pub use crate::ir::{Mode, Rebuild};

/// A selected implementation, before source identities become IR indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selected {
    Static {
        head: Type,
        name: alloc::string::String,
    },
    Field {
        head: Type,
        index: usize,
    },
    Member {
        head: Type,
        name: alloc::string::String,
        argument: Option<Type>,
    },
    Value {
        origin: crate::resolve::Origin,
        site: crate::token::Span,
    },
    Declared {
        origin: crate::resolve::Origin,
        site: crate::token::Span,
    },
    Constructor {
        ty: Type,
        name: alloc::string::String,
    },
    Derived {
        ty: Type,
        kind: crate::ir::Derived,
    },
}

/// A supplied expression retains the same checked data as program code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredExpression {
    pub origin: crate::resolve::Origin,
    pub value: ExprId,
    pub resolved: alloc::rc::Rc<crate::resolve::Resolved>,
    pub typed: alloc::rc::Rc<Typed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeclaredConstructor {
    pub dependencies: Vec<(crate::resolve::Origin, crate::token::Span)>,
    pub home: crate::resolve::Origin,
    pub roots: Vec<ExprId>,
    pub resolved: alloc::rc::Rc<crate::resolve::Resolved>,
    pub typed: alloc::rc::Rc<Typed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub resource: bool,
    pub ty: Type,
    pub bases: Vec<Type>,
    pub constructors: Vec<ConstructorShape>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorShape {
    pub name: alloc::string::String,
    pub fields: Vec<FieldShape>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldShape {
    pub name: alloc::string::String,
    pub ty: Type,
    pub default: Option<DefaultValue>,
    pub public: bool,
}

/// The type's member a dotted member chain starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub selected: Selected,
    pub meanings: Vec<Type>,
}

/// One checked step in a dotted member chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    pub head: Type,
    pub callee: Type,
    pub result: Type,
    pub selected: Selected,
    pub meanings: Vec<Type>,
    pub bound: Option<Bound>,
}

impl Selected {
    pub(crate) fn applied(&self, subst: &Subst) -> Self {
        match self {
            Self::Static { head, name } => Self::Static {
                head: subst.apply(head),
                name: name.clone(),
            },
            Self::Field { head, index } => Self::Field {
                head: subst.apply(head),
                index: *index,
            },
            Self::Member {
                head,
                name,
                argument,
            } => Self::Member {
                head: subst.apply(head),
                name: name.clone(),
                argument: argument.as_ref().map(|t| subst.apply(t)),
            },
            Self::Constructor { ty, name } => Self::Constructor {
                ty: subst.apply(ty),
                name: name.clone(),
            },
            Self::Derived { ty, kind } => Self::Derived {
                ty: subst.apply(ty),
                kind: *kind,
            },
            other => other.clone(),
        }
    }
}

impl Typed {
    /// Recursive uses were checked monomorphically before generalization.
    pub(crate) fn complete_references(&mut self, subst: &Subst) {
        for contract in self.member_contracts.values_mut() {
            contract.selected = contract.selected.applied(subst);
            contract.meanings =
                contract.meanings.iter().map(|ty| subst.apply(ty)).collect();
            contract.receiver = subst.apply(&contract.receiver);
            contract.callable = subst.apply(&contract.callable);
        }
        for (span, selected) in &self.selected {
            let Selected::Value {
                origin: crate::resolve::Origin::This,
                site,
            } = selected
            else {
                continue;
            };
            let Some(scheme) = self.schemes.get(&(site.start, site.end)) else {
                continue;
            };
            if self
                .meaning_arguments
                .get(span)
                .is_some_and(|args| args.len() == scheme.variables.len())
            {
                continue;
            }
            let Some(actual) = self.callee_types.get(span) else {
                continue;
            };
            let mut subst = subst.clone();
            let fresh: Vec<_> = scheme
                .variables
                .iter()
                .map(|var| (*var, Type::Var(subst.fresh())))
                .collect();
            let template = crate::types::replace(&scheme.ty, &fresh);
            subst
                .unify(&template, actual)
                .expect("checked reference matches its scheme");
            self.meaning_arguments.insert(
                *span,
                fresh.iter().map(|(_, ty)| subst.apply(ty)).collect(),
            );
        }
    }
}

/// One generic member's checked receiver alternatives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub name: alloc::string::String,
    pub receiverless: bool,
    pub rows: Vec<DispatchRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchRow {
    pub receiver: Type,
    pub receiver_spot: Option<usize>,
    pub argument: Option<Type>,
    pub selected: Selected,
    pub meanings: Vec<Type>,
    pub ty: Type,
}

/// A limit test checked in its declaration's lexical context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    pub origin: crate::resolve::Origin,
    pub owner: Type,
    pub self_site: crate::token::Span,
    pub value: ExprId,
    /// Supplied tests retain their checked source; local tests use this tree.
    pub expression: Option<DeclaredExpression>,
}

/// A literal pattern's callable equality operation, separate from its reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub selected: Selected,
    pub meanings: Vec<Type>,
    pub ty: Type,
    pub self_slot: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub result: Type,
    pub mode: crate::ir::ConversionMode,
    pub from: Type,
    pub to: Type,
    pub parameter: Type,
}

/// A member read in its owning checked source, including its chain step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberContractRead {
    pub span: crate::token::Span,
    pub step: usize,
}

/// The exact binding instantiation that selected a declaration contract.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemberContractUse {
    pub origin: crate::resolve::Origin,
    pub binding: crate::token::Span,
    pub at: crate::token::Span,
    pub obligation: usize,
}

/// Checked callable places; this records no implementation body facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberContract {
    pub selected: Selected,
    pub home: crate::resolve::Origin,
    pub meanings: Vec<Type>,
    pub receiver: Type,
    pub callable: Type,
    pub reads: Vec<MemberContractRead>,
}

#[cfg(test)]
mod same_owner_done_tests {
    extern crate std;
    use super::*;
    use crate::{
        ast, check, compile, decl::Declarations, ir, lex::lex, mir,
        parse::parse, resolve::Origin,
    };

    fn expressions<'a>(
        value: &'a DefaultValue,
        found: &mut Vec<&'a DeclaredExpression>,
    ) {
        match value {
            DefaultValue::Expression(instance) => found.push(instance),
            DefaultValue::Record { fields, .. } => {
                for (_, _, value) in fields {
                    expressions(value, found);
                }
            }
            DefaultValue::Array { element, .. } => expressions(element, found),
            _ => {}
        }
    }

    fn identity(
        host: &str,
        module: &ast::Module,
    ) -> (ast::ExprId, (usize, usize), (usize, usize)) {
        let helper = host.find("helper:").unwrap();
        let root = module
            .entries
            .iter()
            .find_map(|entry| {
                let ast::Entry::Item(ast::Item {
                    kind:
                        ast::ItemKind::Type {
                            name,
                            members: Some(members),
                            ..
                        },
                    ..
                }) = entry
                else {
                    return None;
                };
                if name.segments.as_slice() != ["R"] {
                    return None;
                }
                members.iter().find_map(|member| {
                    let ast::Member::Field(field) = member else {
                        return None;
                    };
                    (field.name.segments.as_slice() == ["checked"]
                        && !field.public)
                        .then_some(field.default)
                        .flatten()
                })
            })
            .unwrap();
        let ast::ExprKind::Group(parts) = &module.expr(root).unwrap().kind
        else {
            panic!("grouped default")
        };
        let [call] = parts.as_slice() else {
            panic!("one default call")
        };
        let ast::ExprKind::Call { callee, .. } =
            &module.expr(*call).unwrap().kind
        else {
            panic!("helper call")
        };
        let read = module.expr(*callee).unwrap();
        assert_eq!(&host[read.span.start..read.span.end], "helper");
        (
            root,
            (helper, helper + "helper".len()),
            (read.span.start, read.span.end),
        )
    }

    fn predicate_arguments(
        predicate: &ir::Predicate,
        test: usize,
    ) -> Vec<ir::Type> {
        let mut pending = alloc::vec![predicate];
        let mut arguments = Vec::new();
        while let Some(predicate) = pending.pop() {
            match predicate {
                ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                    pending.extend(parts);
                }
                ir::Predicate::Not(inner) => pending.push(inner),
                ir::Predicate::Test { index, meanings } if *index == test => {
                    arguments.extend(meanings.iter().cloned());
                }
                _ => {}
            }
        }
        arguments
    }

    fn helper_predicate(
        host: &str,
        program: &mir::Program,
        tables: &ir::Tables,
        helper: usize,
        helper_body: usize,
    ) {
        let test = tables
            .tests
            .iter()
            .enumerate()
            .find(|(_, test)| {
                tables.namespaces[test.namespace] == "host"
                    && host.get(test.span.start..test.span.end)
                        == Some("self.len > 0")
            })
            .map(|(index, _)| index)
            .unwrap();
        let ir::Type::Function { parameter, .. } =
            &tables.definitions[helper].ty
        else {
            panic!("helper callable")
        };
        let ir::Type::Meaning { arguments, .. } = parameter.as_ref() else {
            panic!("helper Vec")
        };
        let [element] = arguments.as_slice() else {
            panic!("one helper element")
        };
        let mut gates = 0;
        for block in &program.bodies[helper_body].blocks {
            for statement in &block.statements {
                if let mir::Statement::Enter { entry, .. } = statement {
                    let arguments = predicate_arguments(
                        &tables.entries[*entry].predicate,
                        test,
                    );
                    if !arguments.is_empty() {
                        assert_eq!(
                            arguments.as_slice(),
                            core::slice::from_ref(element)
                        );
                        gates += 1;
                    }
                }
            }
        }
        assert!(
            gates > 0,
            "the helper body owns its actual predicate with its own parameter"
        );
    }

    fn emitted_calls(
        host: &str,
        source: &str,
        declarations: &Declarations,
        predicate: bool,
    ) {
        let module = parse(lex(source)).unwrap();
        let unit =
            check::check(&module, declarations, &[], check::Options::default())
                .unwrap();
        let program = compile::compile(&unit);
        let tables = program.tables.borrow();
        let definition = |label: &str| {
            tables
                .definitions
                .iter()
                .enumerate()
                .find(|(_, definition)| {
                    definition.label == label
                        && tables.namespaces[definition.namespace] == "host"
                        && host.get(definition.span.start..definition.span.end)
                            == Some(label)
                })
                .map(|(index, _)| index)
                .unwrap()
        };
        let helper = definition("helper");
        let build = definition("build");
        let helper_body = program.definitions[helper].unwrap();
        let build_body = program.definitions[build].unwrap();
        let ir::Type::Function { parameter, .. } =
            &tables.definitions[build].ty
        else {
            panic!("build callable")
        };
        let ir::Type::Meaning { index, arguments } = parameter.as_ref() else {
            panic!("actual build Vec")
        };
        assert_eq!(tables.meanings[*index].label, "Vec");
        let [element @ ir::Type::Parameter(parameter)] = arguments.as_slice()
        else {
            panic!("one actual build parameter")
        };
        assert_eq!(
            program.bodies[build_body].hidden,
            if predicate {
                alloc::vec![*parameter]
            } else {
                Vec::new()
            }
        );
        if predicate {
            helper_predicate(host, &program, &tables, helper, helper_body);
        }
        let mut count = 0;
        for block in &program.bodies[build_body].blocks {
            if let mir::Terminator::Call {
                callee: mir::Callee::Body(target),
                meanings,
                ..
            } = &block.terminator
            {
                if *target == helper_body {
                    count += 1;
                    assert_eq!(
                        meanings,
                        &if predicate {
                            alloc::vec![element.clone()]
                        } else {
                            Vec::new()
                        }
                    );
                }
            }
        }
        assert!(
            count > 0,
            "the actual written build default calls its own helper"
        );
    }

    fn case(host: &str, source: &str, predicate: bool, reverse: bool) {
        std::println!(
            "DONE_CASE_BEGIN predicate={predicate} reverse={reverse}"
        );
        let host_module =
            parse(lex(host)).expect("written build source parses");
        let (root, helper, read) = identity(host, &host_module);
        let mut declarations = Declarations::new();
        declarations
            .module("host", host)
            .expect("written build header admits");
        let module = parse(lex(source)).unwrap();
        let resolved =
            crate::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed = crate::infer::infer(&module, &resolved, &declarations, &[])
            .unwrap();
        std::println!(
            "DONE_CASE_ADMITTED predicate={predicate} reverse={reverse}"
        );
        let build = host.find("build:").unwrap();
        let checked = typed.constructors.iter().find(|checked|
            checked.home == Origin::Host("host".into()) && checked.typed.schemes.contains_key(&(build, build + "build".len())))
            .expect("actual completed written-build source, not a raw Host binding snapshot");
        let receiving_start = host.find("R { items: xs }").unwrap();
        let receiving =
            (receiving_start, receiving_start + "R { items: xs }".len());
        assert!(
            host_module.expressions.iter().any(|expression| (
                expression.span.start,
                expression.span.end
            ) == receiving
                && matches!(expression.kind, ast::ExprKind::Record { .. })),
            "actual written build record-construction AST"
        );
        let bound = checked
            .typed
            .bound
            .get(&receiving)
            .expect("actual executed build constructor binding");
        let mut candidates = Vec::new();
        for (spot, value) in &bound.defaults {
            match value {
                DefaultValue::Declared { origin, value } => {
                    let instance = checked.typed.default_instances.get(&(receiving.0, receiving.1, *spot))
                        .expect("the exact executed default descriptor has its current lookup");
                    assert_eq!(&instance.origin, origin);
                    assert_eq!(&instance.value, value);
                    candidates.push(instance);
                }
                _ => expressions(value, &mut candidates),
            }
        }
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|instance| {
                instance.origin == checked.home && instance.value == root
            })
            .collect();
        assert_eq!(
            candidates.len(),
            1,
            "exact executed default snapshot from the build Bound"
        );
        let instance = candidates[0];
        assert!(
            matches!(instance.typed.selected.get(&read), Some(Selected::Value { origin: Origin::This, site })
            if (site.start, site.end) == helper),
            "actual helper read belongs to the same checked owner"
        );
        assert_eq!(instance.typed.schemes[&helper].variables.len(), 1);
        assert_eq!(checked.typed.schemes[&helper].variables.len(), 1);
        assert_eq!(
            checked.typed.body_parameters[&helper],
            alloc::vec![Type::Rigid("a".into())],
            "canonical helper owns its written annotated body parameter"
        );
        std::println!("DONE_CASE_LOCAL_ROUTE predicate={predicate} reverse={reverse} receiving={receiving:?}");
        std::println!("DONE_MASKS predicate={predicate} reverse={reverse} authoritative={:?} snapshot={:?}", checked.typed.hidden.get(&helper), instance.typed.hidden.get(&helper));
        let expected = if predicate {
            alloc::vec![0]
        } else {
            Vec::new()
        };
        assert_eq!(
            checked.typed.hidden[&helper], expected,
            "authoritative post-limits helper mask"
        );
        assert_eq!(
            instance.typed.hidden[&helper], checked.typed.hidden[&helper],
            "retained default sees final same-owner helper demand"
        );
        emitted_calls(host, source, &declarations, predicate);
        std::println!("DONE_CASE_END predicate={predicate} reverse={reverse}");
    }

    #[test]
    fn same_owner_done_helper_masks_reach_retained_defaults() {
        const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type R<T> { pub items: Vec<T>, checked: Integer = (helper items) }\nhelper: Vec<a> -> Integer = xs => {\n checked: Nonempty<a> = xs\n 0\n}\nimpl R<T> { build: Vec<T> -> Self = xs => R { items: xs } }\npub read: R<a> -> Integer = r => r.checked\n";
        const CONTROL: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type R<T> { pub items: Vec<T>, checked: Integer = (helper items) }\nhelper: Vec<a> -> Integer = xs => 0\nimpl R<T> { build: Vec<T> -> Self = xs => R { items: xs } }\npub read: R<a> -> Integer = r => r.checked\n";
        for predicate in [true, false] {
            for reverse in [false, true] {
                let uses = if reverse {
                    "(host.read (make [\"s\"]), host.read (make [1]))"
                } else {
                    "(host.read (make [1]), host.read (make [\"s\"]))"
                };
                let source = alloc::format!(
                    "use host\nmake = xs => host.R.build xs\n{uses}\n"
                );
                case(
                    if predicate { HOST } else { CONTROL },
                    &source,
                    predicate,
                    reverse,
                );
            }
        }
    }

    fn transitive_calls(
        host: &str,
        source: &str,
        declarations: &Declarations,
        predicate: bool,
    ) {
        let module = parse(lex(source)).unwrap();
        let unit =
            check::check(&module, declarations, &[], check::Options::default())
                .unwrap();
        let program = compile::compile(&unit);
        let tables = program.tables.borrow();
        let find = |label: &str| {
            tables
                .definitions
                .iter()
                .enumerate()
                .find(|(_, definition)| {
                    definition.label == label
                        && tables.namespaces[definition.namespace] == "host"
                        && host.get(definition.span.start..definition.span.end)
                            == Some(label)
                })
                .map(|(index, _)| index)
                .unwrap()
        };
        let helper = find("helper");
        let leaf = find("predicate_helper");
        let build = find("build");
        if predicate {
            helper_predicate(
                host,
                &program,
                &tables,
                leaf,
                program.definitions[leaf].unwrap(),
            );
        }
        for (caller, callee) in [(build, helper), (helper, leaf)] {
            let caller_body = program.definitions[caller].unwrap();
            let target_body = program.definitions[callee].unwrap();
            let ir::Type::Function { parameter, .. } =
                &tables.definitions[caller].ty
            else {
                panic!("actual generic caller")
            };
            let ir::Type::Meaning { index, arguments } = parameter.as_ref()
            else {
                panic!("actual Vec input")
            };
            assert_eq!(tables.meanings[*index].label, "Vec");
            let [element @ ir::Type::Parameter(parameter)] =
                arguments.as_slice()
            else {
                panic!("one caller element")
            };
            assert_eq!(
                program.bodies[caller_body].hidden,
                if predicate {
                    alloc::vec![*parameter]
                } else {
                    Vec::new()
                }
            );
            let mut count = 0;
            for block in &program.bodies[caller_body].blocks {
                if let mir::Terminator::Call {
                    callee: mir::Callee::Body(target),
                    meanings,
                    ..
                } = &block.terminator
                {
                    if *target == target_body {
                        assert_eq!(
                            meanings,
                            &if predicate {
                                alloc::vec![element.clone()]
                            } else {
                                Vec::new()
                            }
                        );
                        count += 1;
                    }
                }
            }
            assert!(count > 0, "the actual caller body passes its own element witness along the helper chain");
        }
    }

    fn transitive_aliases(
        checked: &DeclaredConstructor,
        receiving: (usize, usize),
        step_start: usize,
        root: ast::ExprId,
        read: crate::token::Span,
        helper: (usize, usize),
    ) -> usize {
        let steps = checked
            .typed
            .accesses
            .get(&receiving)
            .expect("actual written build access chain");
        let (index, step) = steps.iter().enumerate().find(|(_, step)| matches!(&step.selected,
            Selected::Value { origin: Origin::This, site } if site.start == step_start && site.end == step_start + "step".len())).unwrap();
        assert!(
            index + 1 < steps.len(),
            "the method is actually followed by a field read"
        );
        let bound = step
            .bound
            .as_ref()
            .expect("actual intermediate method application");
        let mut aliases = 0;
        for (spot, value) in &bound.defaults {
            let DefaultValue::Expression(instance) = value else {
                panic!("intermediate application must own its actual Expression snapshot")
            };
            assert_eq!(
                instance.origin, checked.home,
                "the actual intermediate default belongs to its checked owner"
            );
            assert_eq!(instance.value, root);
            let lookup = checked
                .typed
                .default_instances
                .get(&(receiving.0, receiving.1, *spot))
                .expect("same-invocation current lookup");
            assert_eq!(instance.origin, lookup.origin);
            assert_eq!(instance.value, lookup.value);
            assert!(alloc::rc::Rc::ptr_eq(&instance.typed, &lookup.typed), "same receiving/spot/origin/root aliases share final Typed identity");
            assert!(
                matches!(instance.typed.selected.get(&(read.start, read.end)), Some(Selected::Value { origin: Origin::This, site })
                if (site.start, site.end) == helper),
                "default helper is the same checked owner"
            );
            assert_eq!(
                instance.typed.hidden[&helper],
                checked.typed.hidden[&helper]
            );
            aliases += 1;
        }
        assert!(
            aliases > 0,
            "a genuine intermediate default alias was observed"
        );
        aliases
    }

    fn transitive_alias_case(
        host: &str,
        source: &str,
        predicate: bool,
        reverse: bool,
    ) {
        std::println!(
            "TRANSITIVE_BEGIN predicate={predicate} reverse={reverse}"
        );
        let host_module =
            parse(lex(host)).expect("transitive Access header parses");
        let mut declarations = Declarations::new();
        declarations
            .module("host", host)
            .expect("transitive Access header admits");
        let module = parse(lex(source)).unwrap();
        let resolved =
            crate::resolve::resolve(&module, &declarations, &[]).unwrap();
        let typed = crate::infer::infer(&module, &resolved, &declarations, &[])
            .unwrap();
        std::println!(
            "TRANSITIVE_ADMITTED predicate={predicate} reverse={reverse}"
        );
        let build = host.find("build:").unwrap();
        let checked = typed
            .constructors
            .iter()
            .find(|checked| {
                checked.home == Origin::Host("host".into())
                    && checked
                        .typed
                        .schemes
                        .contains_key(&(build, build + "build".len()))
            })
            .expect("actual checked build source");
        let chain_start = host.find("(Self xs).step.checked").unwrap();
        let receiving =
            (chain_start, chain_start + "(Self xs).step.checked".len());
        assert!(host_module.expressions.iter().any(|expression| (expression.span.start, expression.span.end) == receiving
            && matches!(&expression.kind, ast::ExprKind::Field { field, .. } if field.segments.as_slice() == ["step", "checked"])), "actual grouped Field chain");
        let root_start = host.find("(helper self.items)").unwrap();
        let root = host_module
            .expressions
            .iter()
            .enumerate()
            .find(|(_, expression)| {
                expression.span.start == root_start
                    && expression.span.end
                        == root_start + "(helper self.items)".len()
                    && matches!(expression.kind, ast::ExprKind::Group(_))
            })
            .map(|(index, _)| ast::ExprId(index))
            .unwrap();
        let ast::ExprKind::Group(parts) = &host_module.expr(root).unwrap().kind
        else {
            unreachable!()
        };
        let [call] = parts.as_slice() else {
            panic!("one actual signature-default call")
        };
        let ast::ExprKind::Call { callee, .. } =
            host_module.expr(*call).unwrap().kind
        else {
            panic!("actual helper call")
        };
        let read = host_module.expr(callee).unwrap().span;
        assert_eq!(&host[read.start..read.end], "helper");
        let helper_start = host.find("\nhelper:").unwrap() + 1;
        let helper = (helper_start, helper_start + "helper".len());
        let leaf_start = host.find("predicate_helper:").unwrap();
        let leaf = (leaf_start, leaf_start + "predicate_helper".len());
        let step_start = host.find("step:").unwrap();
        let aliases = transitive_aliases(
            checked, receiving, step_start, root, read, helper,
        );
        let expected = if predicate {
            alloc::vec![0]
        } else {
            Vec::new()
        };
        for (site, name) in [(helper, "a"), (leaf, "b")] {
            assert_eq!(checked.typed.schemes[&site].variables.len(), 1);
            assert_eq!(
                checked.typed.body_parameters[&site],
                alloc::vec![Type::Rigid(name.into())]
            );
            assert_eq!(checked.typed.hidden[&site], expected, "predicate demand propagates through the fixed-result helper chain");
        }
        std::println!("TRANSITIVE_ALIAS predicate={predicate} reverse={reverse} receiving={receiving:?} aliases={aliases}");
        transitive_calls(host, source, &declarations, predicate);
        std::println!("TRANSITIVE_END predicate={predicate} reverse={reverse}");
    }

    #[test]
    fn same_owner_transitive_predicate_reaches_intermediate_default_alias() {
        const HOST: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type Result<T> { pub items: Vec<T>, pub checked: Integer }\npub type Probe<T> { pub items: Vec<T> }\nhelper: Vec<a> -> Integer = xs => predicate_helper xs\npredicate_helper: Vec<b> -> Integer = ys => {\n checked: Nonempty<b> = ys\n 0\n}\nimpl Probe<T> {\n step: (self: Self) -> (checked: Integer = (helper self.items)) -> Result<T> = self checked => Result { items: self.items, checked: checked },\n build: Vec<T> -> Integer = xs => (Self xs).step.checked\n}\n";
        const CONTROL: &str = "pub type Nonempty<T>: Vec<T> where self.len > 0\npub type Result<T> { pub items: Vec<T>, pub checked: Integer }\npub type Probe<T> { pub items: Vec<T> }\nhelper: Vec<a> -> Integer = xs => predicate_helper xs\npredicate_helper: Vec<b> -> Integer = ys => 0\nimpl Probe<T> {\n step: (self: Self) -> (checked: Integer = (helper self.items)) -> Result<T> = self checked => Result { items: self.items, checked: checked },\n build: Vec<T> -> Integer = xs => (Self xs).step.checked\n}\n";
        for predicate in [true, false] {
            for reverse in [false, true] {
                let uses = if reverse {
                    "(make [\"s\"], make [1])"
                } else {
                    "(make [1], make [\"s\"])"
                };
                let source = alloc::format!(
                    "use host\nmake = xs => host.Probe.build xs\n{uses}\n"
                );
                transitive_alias_case(
                    if predicate { HOST } else { CONTROL },
                    &source,
                    predicate,
                    reverse,
                );
            }
        }
    }
}
