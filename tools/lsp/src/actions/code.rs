//! Code actions.
//!
//! A `use` for an unknown name a host module or a file beside the
//! document exports, the inferred scheme written as an unannotated
//! binding's annotation, and the crate's own suggestions as quick
//! fixes (double parentheses to keep a tuple; `||` or `|:` into a
//! text place).
use std::path::Path;

use fipe::ast::{Entry, ExprKind, ItemKind, PatternKind};
use fipe::lex::lex;
use fipe::parse::parse_partial;

use crate::analysis::Analysis;
use crate::loader::{self, Host};

/// One action: its title, the kind the protocol names, and the text
/// edits as byte spans replaced by text.
pub struct Action {
    pub title: String,
    pub kind: &'static str,
    pub edits: Vec<((usize, usize), String)>,
}

/// The actions for `start..end` of the document at `path`.
///
/// `diagnostics` are the ones the client sent (their spans and
/// messages); `check` analyses a candidate text and counts its
/// diagnostics, for the actions offered only when the result checks.
#[must_use]
pub fn actions(
    analysis: &Analysis,
    host: &Host,
    path: Option<&Path>,
    (start, end): (usize, usize),
    diagnostics: &[((usize, usize), String)],
    check: &dyn Fn(&str) -> usize,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let overlaps = |(from, to): (usize, usize)| from <= end && start <= to;
    for (span, message) in diagnostics {
        if !overlaps(*span) {
            continue;
        }
        if message.starts_with("unknown name") {
            let name = analysis.text.get(span.0..span.1).unwrap_or_default();
            for module in exporters(analysis, host, path, name) {
                actions.push(use_action(analysis, &module, name));
            }
        }
        if message.contains("cannot enter a text place") {
            if let Some(at) = pipe_before(&analysis.text, *span) {
                for (operator, what) in
                    [("||", "the good side"), ("|:", "the payload")]
                {
                    actions.push(Action {
                        title: format!("Pipe {what} with `{operator}`"),
                        kind: "quickfix",
                        edits: vec![((at, at + 1), String::from(operator))],
                    });
                }
            }
        }
    }
    for warning in &analysis.typed.warnings {
        let span = (warning.span.start, warning.span.end);
        if !overlaps(span) {
            continue;
        }
        if let Some((open, close)) = last_group(&analysis.text, span) {
            actions.push(Action {
                title: String::from("Keep the tuple: double the parentheses"),
                kind: "quickfix",
                edits: vec![
                    ((open, open + 1), String::from("((")),
                    ((close, close + 1), String::from("))")),
                ],
            });
        }
    }
    // The annotation is the language's form of a signature (`name: T =
    // value`), so it is offered only when the document checks with it
    // written.
    for (name, (name_start, name_end), scheme) in unannotated(analysis) {
        if !overlaps((name_start, name_end)) {
            continue;
        }
        let annotation = format!(": {scheme}");
        let mut candidate = analysis.text.clone();
        candidate.insert_str(name_end, &annotation);
        if check(&candidate) == 0 {
            actions.push(Action {
                title: format!("Annotate `{name}{annotation}`"),
                kind: "refactor.rewrite",
                edits: vec![((name_end, name_end), annotation)],
            });
        }
    }
    actions
}

/// The host modules and files beside the document that export `name`.
fn exporters(
    analysis: &Analysis,
    host: &Host,
    path: Option<&Path>,
    name: &str,
) -> Vec<String> {
    let mut found = Vec::new();
    for (module, _, _) in &host.sources {
        if module == "commands" {
            continue;
        }
        let Some(namespace) = host.declarations.get_module(module) else {
            continue;
        };
        if namespace.value(name).is_some() || namespace.ty(name).is_some() {
            found.push(module.clone());
        }
    }
    let Some(dir) = path.and_then(Path::parent) else {
        return found;
    };
    let own = path
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    let mut siblings: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fipe"))
        .collect();
    siblings.sort();
    for sibling in siblings {
        let Some(stem) = sibling.file_stem().and_then(|stem| stem.to_str())
        else {
            continue;
        };
        if Some(stem) == own || found.iter().any(|module| module == stem) {
            continue;
        }
        let module = analysis
            .files
            .iter()
            .find(|file| file.stem == stem)
            .map(|file| file.module.clone())
            .or_else(|| {
                std::fs::read_to_string(&sibling)
                    .ok()
                    .map(|text| parse_partial(lex(&text)))
            });
        let Some(module) = module else { continue };
        if exports(&module).any(|export| export == name) {
            found.push(String::from(stem));
        }
    }
    found
}

/// `use module.name` inserted after the document's last `use` line, or
/// at its top.
fn use_action(analysis: &Analysis, module: &str, name: &str) -> Action {
    let text = &analysis.text;
    let at = loader::uses(&analysis.module)
        .map(|(_, (_, end))| end)
        .max()
        .map_or(0, |end| {
            text[end..].find('\n').map_or(text.len(), |i| end + i + 1)
        });
    let line = format!("use {module}.{name}\n");
    let line = if at == text.len() && !text.ends_with('\n') && !text.is_empty()
    {
        format!("\n{line}")
    } else {
        line
    };
    Action {
        title: format!("Import `{module}.{name}`"),
        kind: "quickfix",
        edits: vec![((at, at), line)],
    }
}

/// The byte of the lone `|` just before `start..end` over white space,
/// the first inside it, or the one just after it.
fn pipe_before(text: &str, (start, end): (usize, usize)) -> Option<usize> {
    let bytes = text.as_bytes();
    let lone = |at: usize| {
        bytes.get(at) == Some(&b'|')
            && !matches!(bytes.get(at + 1), Some(b'|' | b':' | b'!'))
            && (at == 0 || bytes[at - 1] != b'|')
    };
    let before = text.get(..start)?;
    if let Some(at) = before
        .trim_end()
        .len()
        .checked_sub(1)
        .filter(|at| lone(*at))
    {
        return Some(at);
    }
    let end = end.min(text.len());
    if let Some(at) = (start..end).find(|at| lone(*at)) {
        return Some(at);
    }
    // The value entering the place is before the pipe: the next
    // non-space byte after it.
    let after =
        end + text.get(end..)?.len() - text.get(end..)?.trim_start().len();
    lone(after).then_some(after)
}

/// The last parenthesized group ending a stage: its `(` and `)`.
fn last_group(
    text: &str,
    (start, end): (usize, usize),
) -> Option<(usize, usize)> {
    let stage = text.get(start..end)?;
    let close = start + stage.trim_end().len().checked_sub(1)?;
    if text.as_bytes().get(close) != Some(&b')') {
        return None;
    }
    let mut depth = 0usize;
    for (index, byte) in text[start..=close].bytes().enumerate().rev() {
        match byte {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start + index, close));
                }
            }
            _ => {}
        }
    }
    None
}

/// Each top-level binding without an annotation: its name, the name's
/// span, and its scheme.
fn unannotated(analysis: &Analysis) -> Vec<(String, (usize, usize), String)> {
    let module = &analysis.module;
    module
        .entries
        .iter()
        .filter_map(|entry| {
            let Entry::Line(id) = entry else { return None };
            let ExprKind::Bind {
                pattern,
                annotation: None,
                ..
            } = &module.expr(*id)?.kind
            else {
                return None;
            };
            let PatternKind::Name(name) = &module.pattern(*pattern)?.kind
            else {
                return None;
            };
            let scheme = analysis.typed.binding(*id)?.to_string();
            Some((
                name.segments.join("."),
                (name.span.start, name.span.end),
                scheme,
            ))
        })
        .collect()
}

/// The names a module exports: its `pub` bindings, declarations and
/// types.
fn exports(module: &fipe::ast::Module) -> impl Iterator<Item = String> + '_ {
    module.entries.iter().filter_map(|entry| match entry {
        Entry::Line(id) => match &module.expr(*id)?.kind {
            ExprKind::Bind {
                public: true,
                pattern,
                ..
            } => match &module.pattern(*pattern)?.kind {
                PatternKind::Name(name) => Some(name.segments.join(".")),
                _ => None,
            },
            _ => None,
        },
        Entry::Item(item) if item.public => match &item.kind {
            ItemKind::Type { name, .. } | ItemKind::Declare { name, .. } => {
                Some(name.segments.join("."))
            }
            _ => None,
        },
        Entry::Item(_) => None,
    })
}
