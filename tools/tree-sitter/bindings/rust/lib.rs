//! The Fipe grammar for the [tree-sitter] parsing library.
//!
//! ```
//! let mut parser = tree_sitter::Parser::new();
//! parser
//!     .set_language(&tree_sitter_fipe::LANGUAGE.into())
//!     .expect("the Fipe grammar loads");
//! let tree = parser.parse("ls / | len\n", None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_fipe() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for this grammar.
// SAFETY: `tree_sitter_fipe` is the generated parser's entry point; it
// takes nothing and returns a pointer to a static `TSLanguage`.
pub const LANGUAGE: LanguageFn =
    unsafe { LanguageFn::from_raw(tree_sitter_fipe) };

/// The content of the grammar's [`node-types.json`].
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

#[cfg(test)]
mod tests {
    #[test]
    fn the_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("the Fipe grammar loads");
    }
}
