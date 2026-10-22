//! Inlay hints: an unannotated binding's inferred scheme after its
//! name.
use fipe::ast::{ExprKind, PatternKind};

use crate::analysis::Analysis;

/// A hint: the byte offset it sits at and its text.
pub struct Hint {
    pub offset: usize,
    pub label: String,
}

/// Each binding without a type annotation whose name lies in
/// `start..end`, with `: <scheme>` to show after the name.
#[must_use]
pub fn hints(analysis: &Analysis, (start, end): (usize, usize)) -> Vec<Hint> {
    let module = &analysis.module;
    let mut hints: Vec<Hint> = analysis
        .typed
        .bindings()
        .filter_map(|(id, scheme)| {
            let ExprKind::Bind {
                pattern,
                annotation: None,
                ..
            } = &module.expr(id)?.kind
            else {
                return None;
            };
            let pattern = module.pattern(*pattern)?;
            let PatternKind::Name(_) = &pattern.kind else {
                return None;
            };
            let offset = pattern.span.end;
            (start <= offset && offset <= end).then(|| Hint {
                offset,
                label: format!(": {scheme}"),
            })
        })
        .collect();
    hints.sort_by_key(|hint| hint.offset);
    hints
}
