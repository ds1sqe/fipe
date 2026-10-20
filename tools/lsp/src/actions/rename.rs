//! Rename: what may be renamed, to what, and the span of the name in
//! each place that spells it.

use fipe::ast::{Entry, ItemKind, Module};
use fipe::lex::lex;
use fipe::resolve::{Origin, Target};
use fipe::token::Kind;

/// The target a rename at a node acts on: the head of a member chain,
/// the type of a type member; a host, prelude or module name is
/// refused.
///
/// # Errors
/// The target is not the document's or its files' to rename.
pub fn renamed(target: &Target) -> Result<Target, &'static str> {
    let target = match target {
        Target::Members { head, .. } => head.as_ref(),
        Target::TypeMember { ty, .. } => ty.as_ref(),
        other => other,
    };
    match target {
        Target::Local { .. } => Ok(target.clone()),
        Target::Value { origin, .. }
        | Target::Constructor { origin, .. }
        | Target::Type { origin, .. } => match origin {
            Origin::This | Origin::File(_) => Ok(target.clone()),
            Origin::Prelude | Origin::Commands | Origin::Host(_) => {
                Err("a prelude or host name cannot be renamed")
            }
            Origin::Line(_) => Err("a session line's name cannot be renamed"),
        },
        Target::Module(_) => Err("a module is renamed by renaming its file"),
        _ => Err("nothing here can be renamed"),
    }
}

/// The name `target` binds; a local's is the text at its site.
#[must_use]
pub fn old_name(text: &str, target: &Target) -> Option<String> {
    match target {
        Target::Local { site } => {
            text.get(site.start..site.end).map(String::from)
        }
        Target::Value { name, .. }
        | Target::Constructor { name, .. }
        | Target::Type { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Whether `new` may replace `old`: one name of one segment, of the
/// same case, since the case says whether a name is a type or
/// constructor.
#[must_use]
pub fn valid(old: &str, new: &str) -> bool {
    let tokens: Vec<Kind> = lex(new)
        .tokens
        .into_iter()
        .map(|token| token.kind)
        .filter(|kind| !matches!(kind, Kind::Newline | Kind::Eof))
        .collect();
    let upper = |name: &str| name.starts_with(|c: char| c.is_ascii_uppercase());
    matches!(tokens.as_slice(), [Kind::Name(segments)] if segments.len() == 1)
        && upper(old) == upper(new)
}

/// Where `name` is spelled within the span `start..end` of `text`: the
/// whole span, or one dotted segment of it.
#[must_use]
pub fn name_span(
    text: &str,
    (start, end): (usize, usize),
    name: &str,
) -> Option<(usize, usize)> {
    let slice = text.get(start..end)?;
    if slice == name {
        return Some((start, end));
    }
    let mut at = 0;
    for segment in slice.split('.') {
        if segment == name {
            return Some((start + at, start + at + name.len()));
        }
        at += segment.len() + 1;
    }
    None
}

/// The spans of `name` in the `use` items of `module` that import it
/// from the file module `stem`.
#[must_use]
pub fn use_spans(
    module: &Module,
    stem: &str,
    name: &str,
) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        let ItemKind::Use { names, .. } = &item.kind else {
            continue;
        };
        for used in names {
            if let [module, imported] = used.segments.as_slice() {
                if module == stem && imported == name {
                    spans.push((used.span.end - name.len(), used.span.end));
                }
            }
        }
    }
    spans
}
