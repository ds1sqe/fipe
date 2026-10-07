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
