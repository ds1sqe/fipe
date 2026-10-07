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
