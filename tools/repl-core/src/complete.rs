//! Tab completion of the word before the cursor.
//!
//! A word is a name's characters and dots. At a line's start after `:`
//! the commands; after a dot, a host or loaded module's public names,
//! or the fields and impl members of the type of the value before the
//! dot; otherwise the names in scope: the session's bindings, types,
//! constructors and imports, the modules, the host's commands and the
//! prelude.

use crate::{edit::Completion, Repl, COMMANDS};
use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};
use fipe::{
    ast::{
        Entry, ExprKind, ImplMember, Item, ItemKind, Member, Module, PatternId,
        PatternKind,
    },
    decl::Namespace,
    embed::{Outcome, Reply, Request},
    lex::lex,
    parse::parse_partial,
};

impl Repl {
    /// The completion of the word that ends `before`, the line's text
    /// before the cursor; no candidate inside a quote or a comment, or
    /// for an empty word outside a member.
    #[must_use]
    pub fn complete(&self, before: &str) -> Completion {
        let start = before
            .trim_end_matches(|c: char| {
                c.is_ascii_alphanumeric() || c == '_' || c == '.'
            })
            .len();
        let word = &before[start..];
        let (partial, candidates) = if in_text(before) {
            (word, Vec::new())
        } else if before[..start].trim_start() == ":" {
            let commands = COMMANDS.iter().filter_map(|(usage, _)| {
                usage.split(' ').next().map(String::from)
            });
            (before.trim_start(), commands.collect())
        } else if let Some(dot) = word.rfind('.') {
            let head = &word[..dot];
            // `..` or `x..` has no head to take members of.
            let members = if head.is_empty() || head.ends_with('.') {
                Vec::new()
            } else {
                self.members(head)
            };
            (&word[dot + 1..], members)
        } else if word.is_empty() {
            (word, Vec::new())
        } else {
            (word, self.in_scope())
        };
        let candidates: BTreeSet<String> = candidates
            .into_iter()
            .filter(|candidate| candidate.starts_with(partial))
            .collect();
        Completion {
            partial: partial.to_string(),
            candidates: candidates.into_iter().collect(),
        }
    }

    fn in_scope(&self) -> Vec<String> {
        let mut names = Vec::new();
        let held = (!self.entry.is_empty()).then_some(self.entry.as_str());
        for text in self.session.lines().chain(held) {
            let module = parse_partial(lex(text));
            for entry in &module.entries {
                match entry {
                    Entry::Item(item) => item_names(item, &mut names),
                    Entry::Line(id) => {
                        if let Some(ExprKind::Bind { pattern, .. }) =
                            module.expr(*id).map(|expr| &expr.kind)
                        {
                            pattern_names(&module, *pattern, &mut names);
                        }
                    }
                }
            }
        }
        names.extend(self.files.iter().map(|(stem, _)| stem.clone()));
        let declarations = &self.declarations;
        names.extend(declarations.modules.iter().map(|ns| ns.name.clone()));
        if let Some(commands) = &declarations.commands {
            names.extend(public_names(commands));
        }
        names.extend(public_names(&declarations.prelude));
        names
    }

    /// The members after `head.`.
    fn members(&self, head: &str) -> Vec<String> {
        let expanded = self.session.lines().find_map(|line| {
            let module = parse_partial(lex(line));
            module.entries.iter().find_map(|entry| {
                let Entry::Item(Item {
                    kind: ItemKind::Use { names, .. },
                    ..
                }) = entry
                else {
                    return None;
                };
                names.iter().find_map(|name| {
                    let (first, rest) =
                        head.split_once('.').unwrap_or((head, ""));
                    let imported = name.segments.join(".");
                    (name.segments.last().is_some_and(|name| name == first)
                        && self.declarations.get_module(&imported).is_some())
                    .then(|| {
                        if rest.is_empty() {
                            imported
                        } else {
                            alloc::format!("{imported}.{rest}")
                        }
                    })
                })
            })
        });
        let head = expanded.as_deref().unwrap_or(head);
        if let Some(namespace) = self.declarations.get_module(head) {
            let mut names = public_names(namespace);
            names.extend(self.declarations.submodules(head).filter_map(
                |child| child.name.rsplit('.').next().map(String::from),
            ));
            return names;
        }
        if let Some(text) = self.cached(head) {
            return exports(&parse_partial(lex(text)));
        }
        let Some(ty) = self.type_of(head) else {
            return Vec::new();
        };
        let name = type_head(&ty);
        if name.is_empty() {
            return Vec::new();
        }
        let mut members = Vec::new();
        let texts =
            self.session.lines().chain(self.files.iter().map(|f| &*f.1));
        for text in texts {
            type_members(&parse_partial(lex(text)), text, &name, &mut members);
        }
        let declarations = &self.declarations;
        let prelude = fipe::decl::prelude_source();
        type_members(
            &declarations.prelude.module,
            prelude,
            &name,
            &mut members,
        );
        let host = declarations.commands.iter().chain(&declarations.modules);
        for namespace in host {
            if let Some((_, text)) = self
                .host_texts
                .iter()
                .find(|(name, _)| *name == namespace.name)
            {
                type_members(&namespace.module, text, &name, &mut members);
            }
        }
        members
    }

    /// The type of `head` as a query prints it, on a copy of the session
    /// so nothing is accepted; a `use` it needs is answered from the
    /// files already read.
    fn type_of(&self, head: &str) -> Option<String> {
        let mut session = self.session.clone();
        let mut outcome = session.query(head);
        let report = loop {
            match outcome {
                Outcome::Request(Request::File { stem }) => {
                    let text = self.cached(&stem).map(String::from);
                    outcome = session.reply(Reply::File(text));
                }
                Outcome::Request(Request::Call { .. }) => return None,
                Outcome::Done(report) => break report,
            }
        };
        if !report.diagnostics.is_empty() {
            return None;
        }
        let typed = report.typed.as_ref()?;
        let Some(Entry::Line(id)) = report.entries().first() else {
            return None;
        };
        typed
            .binding(*id)
            .map(ToString::to_string)
            .or_else(|| typed.expr(*id).map(ToString::to_string))
    }
}

/// Whether `before` ends inside a quote or a comment.
fn in_text(before: &str) -> bool {
    let lexed = lex(before);
    let end = before.len();
    lexed.comments.iter().any(|span| span.end == end)
        || lexed.errors.iter().any(|error| {
            error.span.end == end
                && before
                    .get(error.span.start..)
                    .is_some_and(|text| text.starts_with(['"', '\'']))
        })
}

/// The public names of a declared namespace; all of the prelude's.
fn public_names(namespace: &Namespace) -> Vec<String> {
    namespace
        .values
        .iter()
        .chain(&namespace.types)
        .filter(|declared| declared.public || namespace.name == "prelude")
        .map(|declared| declared.name.clone())
        .collect()
}

/// The public names a loaded file module exports.
fn exports(module: &Module) -> Vec<String> {
    let mut names = Vec::new();
    for entry in &module.entries {
        match entry {
            Entry::Item(item) if item.public => item_names(item, &mut names),
            Entry::Line(id) => {
                if let Some(ExprKind::Bind {
                    public: true,
                    pattern,
                    ..
                }) = module.expr(*id).map(|expr| &expr.kind)
                {
                    pattern_names(module, *pattern, &mut names);
                }
            }
            Entry::Item(_) => {}
        }
    }
    names
}

/// The names an item puts in scope.
fn item_names(item: &Item, names: &mut Vec<String>) {
    match &item.kind {
        ItemKind::Type { name, members, .. } => {
            names.push(name.segments.join("."));
            for member in members.iter().flatten() {
                if let Member::Constructor { name, .. } = member {
                    names.push(name.segments.join("."));
                }
            }
        }
        ItemKind::Use { names: used, .. } => {
            names.extend(
                used.iter().filter_map(|name| name.segments.last().cloned()),
            );
        }
        ItemKind::Declare { name, .. } => names.push(name.segments.join(".")),
        ItemKind::Impl { .. } => {}
    }
}

/// The names a pattern binds.
fn pattern_names(module: &Module, id: PatternId, names: &mut Vec<String>) {
    let Some(pattern) = module.pattern(id) else {
        return;
    };
    match &pattern.kind {
        PatternKind::Name(name) | PatternKind::Default { name, .. } => {
            if !name.is_uppercase() {
                names.push(name.segments.join("."));
            }
        }
        PatternKind::Constructor { payload, .. } => {
            pattern_names(module, *payload, names);
        }
        PatternKind::Group(inner) => pattern_names(module, *inner, names),
        PatternKind::Tuple(parts) | PatternKind::List(parts) => {
            for part in parts {
                pattern_names(module, *part, names);
            }
        }
        PatternKind::Outcome {
            payload: Some(payload),
            ..
        } => pattern_names(module, *payload, names),
        PatternKind::Record { fields, .. } => {
            for field in fields {
                match field.pattern {
                    Some(inner) => pattern_names(module, inner, names),
                    None => names.push(field.name.segments.join(".")),
                }
            }
        }
        PatternKind::Wildcard
        | PatternKind::Literal(_)
        | PatternKind::Outcome { payload: None, .. } => {}
    }
}

/// The fields and impl members of the type `name` in `module`, whose
/// source is `text`.
fn type_members(
    module: &Module,
    text: &str,
    name: &str,
    out: &mut Vec<String>,
) {
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Type {
                name: declared,
                members: Some(members),
                ..
            } if declared.segments.last().map(String::as_str) == Some(name) => {
                for member in members {
                    if let Member::Field(field) = member {
                        out.push(field.name.segments.join("."));
                    }
                }
            }
            ItemKind::Impl {
                target, members, ..
            } => {
                let target = module
                    .ty(*target)
                    .and_then(|ty| text.get(ty.span.start..ty.span.end))
                    .unwrap_or_default();
                if target_head(target) != name {
                    continue;
                }
                for member in members {
                    if let ImplMember::Named { name, .. } = member {
                        out.push(name.segments.join("."));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The name heading a printed type, past its limits (`Literal a =>
/// Vec<a>` is `Vec`); `[]` for an array, whose impls are written on
/// `[T; n]`; empty for a tuple, a function or a type variable.
fn type_head(shown: &str) -> String {
    let shown = shown.rsplit("=>").next().unwrap_or_default().trim();
    let mut depth = 0usize;
    for (at, c) in shown.char_indices() {
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth = depth.saturating_sub(1),
            _ if depth == 0
                && (shown[at..].starts_with(" * ")
                    || shown[at..].starts_with(" -> ")) =>
            {
                return String::new();
            }
            _ => {}
        }
    }
    if shown.starts_with('[') {
        return String::from("[]");
    }
    shown
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect()
}

/// The head of an impl's written target, as [`type_head`] names it.
fn target_head(written: &str) -> &str {
    if written.trim_start().starts_with('[') {
        "[]"
    } else {
        written.split(['<', ' ']).next().unwrap_or_default().trim()
    }
}
