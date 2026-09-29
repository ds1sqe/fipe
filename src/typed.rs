//! The typed tree: a type per expression and a scheme per binding, over
//! a module and its [`Resolved`](crate::resolve::Resolved) names.

use crate::{
    ast::ExprId,
    types::{Obligation, Scheme, Type},
};
use alloc::vec::Vec;

/// What inference learned about one module. Types are fully applied:
/// a variable left in them is unsolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typed {
    pub(crate) expressions: Vec<Option<Type>>,
    pub(crate) bindings: Vec<Option<Scheme>>,
    /// Obligations left on variables, such as `Ord ?0`.
    pub unsolved: Vec<Obligation>,
}

impl Typed {
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
