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
    /// A conversion at each expression's receiving place, if any.
    pub casts: Vec<Option<Cast>>,
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
pub struct Shape {
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
