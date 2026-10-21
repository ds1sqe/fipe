//! Completion at a byte offset.
//!
//! After `use`, the loader's modules; after a dot, a module's members
//! or a value's fields and `self` functions; for a word starting with
//! `-`, the flags of the stage's command; otherwise the names in scope.

use std::path::Path;

use fipe::ast::{Entry, Node};
use fipe::ast::{
    ExprId, ExprKind, ImplMember, ItemKind, Member, Module, PatternId,
    PatternKind,
};
use fipe::decl::{Kind as Declared, Namespace};
use fipe::lex::lex;
use fipe::token::{Kind as Token, Symbol};
use lsp_types::{CompletionItem, CompletionItemKind};

use crate::analysis::Analysis;
use crate::loader::Host;

/// What a completion offers, before it becomes an LSP item.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Offer {
    label: String,
    kind: CompletionItemKind,
    detail: Option<String>,
}

impl Offer {
    fn new(label: &str, kind: CompletionItemKind) -> Self {
        Self {
            label: String::from(label),
            kind,
            detail: None,
        }
    }
}

/// The completions at `offset` of the analysed document, which lies in
/// `dir` when it has a path. `reanalyse` analyses another text of the
/// same document, for the type of a value before a dot.
#[must_use]
pub fn complete(
    analysis: &Analysis,
    host: &Host,
    dir: Option<&Path>,
    offset: usize,
    reanalyse: &dyn Fn(&str) -> Analysis,
) -> Vec<CompletionItem> {
    let text = analysis.text.as_str();
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    if in_text(text, offset) {
        return Vec::new();
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let prefix = &text[line_start..offset];
    let word_start = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace() || "([{,|".contains(*c))
        .map_or(0, |(at, c)| at + c.len_utf8());
    let word = &prefix[word_start..];
    let mut offers = Vec::new();
    let partial;
    if word.starts_with('-') && word.contains('=') {
        // A flag's value: any name that may stand there.
        partial = &word[word.find('=').map_or(word.len(), |at| at + 1)..];
        offers = in_scope(analysis, host, offset);
    } else if let Some(dot) = word.rfind('.') {
        partial = &word[dot + 1..];
        let head = &word[..dot];
        let head_end = line_start + word_start + dot;
        offers = members(analysis, host, head, head_end, offset, reanalyse);
    } else if word.starts_with('-') {
        partial = word;
        if let Some(command) = command(&prefix[..word_start]) {
            offers = flags(analysis, host, &command);
        }
    } else if is_use(&prefix[..word_start]) {
        partial = word;
        offers = modules(host, dir);
    } else {
        partial = word;
        offers = in_scope(analysis, host, offset);
    }
    let mut seen = std::collections::BTreeSet::new();
    offers
        .into_iter()
        .filter(|offer| offer.label.starts_with(partial))
        .filter(|offer| seen.insert(offer.label.clone()))
        .map(|offer| CompletionItem {
            label: offer.label,
            kind: Some(offer.kind),
            detail: offer.detail,
            ..CompletionItem::default()
        })
        .collect()
}

/// What hovering a flag at `offset` shows: the spot it names, or for a
/// chained flag the Bool spots its letters expand to (`-lr`), each with
/// its type and its doc line when the declaration has one.
#[must_use]
pub fn spot_hover(
    analysis: &Analysis,
    host: &Host,
    offset: usize,
) -> Option<String> {
    let text = analysis.text.as_str();
    let lexed = lex(text);
    let token = lexed.tokens.iter().find(|token| {
        token.span.start <= offset
            && offset <= token.span.end
            && matches!(token.kind, Token::Flag(_))
    })?;
    let Token::Flag(written) = &token.kind else {
        return None;
    };
    let label = written.trim_start_matches('-').split('=').next()?;
    let line_start =
        text[..token.span.start].rfind('\n').map_or(0, |at| at + 1);
    let command = command(&text[line_start..token.span.start])?;
    let spots = spot_details(analysis, host, &command);
    let named: Vec<&Spot> = match spots.iter().find(|spot| spot.label == label)
    {
        Some(spot) => vec![spot],
        // A chained flag: each letter the one Bool spot it starts.
        None => label
            .chars()
            .map(|letter| {
                let mut bools = spots.iter().filter(|spot| {
                    spot.ty == "Bool" && spot.label.starts_with(letter)
                });
                match (bools.next(), bools.next()) {
                    (Some(spot), None) => Some(spot),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>()?,
    };
    let shown: Vec<String> = named
        .iter()
        .map(|spot| {
            let head = format!("{}: {}", spot.label, spot.ty);
            spot.doc
                .as_ref()
                .map_or_else(|| head.clone(), |doc| format!("{head}\n{doc}"))
        })
        .collect();
    (!shown.is_empty()).then(|| shown.join("\n"))
}

/// A labeled spot: its label, its type as written and its doc line.
struct Spot {
    label: String,
    ty: String,
    doc: Option<String>,
}

/// The labeled spots of `command`, with their types and doc lines.
fn spot_details(
    analysis: &Analysis,
    host: &Host,
    command: &[String],
) -> Vec<Spot> {
    let declared = |namespace: &Namespace, text: &str, name: &str| {
        let value = namespace.value(name)?;
        let Declared::Function { spots, .. } = &value.kind else {
            return None;
        };
        let start = value.span.start.min(text.len());
        let above = &text[..text[..start].rfind('\n').unwrap_or(0)];
        let docs: Vec<&str> = above
            .lines()
            .rev()
            .take_while(|line| line.trim_start().starts_with("##"))
            .collect();
        let doc_of = |label: &str| {
            docs.iter()
                .map(|line| line.trim_start().trim_start_matches('#').trim())
                .find(|line| line.starts_with(&format!("- {label}:")))
                .map(String::from)
        };
        Some(
            spots
                .iter()
                .filter_map(|spot| {
                    let label = spot.name.clone()?;
                    let span = namespace.module.ty(spot.ty)?.span;
                    let ty = text.get(span.start..span.end)?.trim().to_string();
                    let doc = doc_of(&label);
                    Some(Spot { label, ty, doc })
                })
                .collect::<Vec<_>>(),
        )
    };
    let found = match command {
        [name] => host
            .declarations
            .commands
            .as_ref()
            .and_then(|commands| {
                declared(commands, host.source("commands")?.1, name)
            })
            .or_else(|| {
                declared(
                    &host.declarations.prelude,
                    fipe::decl::prelude_source(),
                    name,
                )
            })
            .or_else(|| document_spots_typed(analysis, name)),
        [module, name] => {
            host.declarations.get_module(module).and_then(|namespace| {
                declared(namespace, host.source(module)?.1, name)
            })
        }
        _ => None,
    };
    found.unwrap_or_default()
}

/// The spots of a function the document binds, typed as its inference
/// gives them.
fn document_spots_typed(analysis: &Analysis, name: &str) -> Option<Vec<Spot>> {
    let module = &analysis.module;
    for entry in &module.entries {
        let Entry::Line(id) = entry else { continue };
        let Some(ExprKind::Bind { pattern, value, .. }) =
            module.expr(*id).map(|expr| &expr.kind)
        else {
            continue;
        };
        let Some(PatternKind::Name(bound)) =
            module.pattern(*pattern).map(|pattern| &pattern.kind)
        else {
            continue;
        };
        if bound.segments != [name] {
            continue;
        }
        let ExprKind::Arms(arms) = &module.expr(*value)?.kind else {
            return None;
        };
        let spots = arms
            .first()?
            .patterns
            .iter()
            .filter_map(|id| match &module.pattern(*id)?.kind {
                PatternKind::Name(spot)
                | PatternKind::Default { name: spot, .. } => Some(Spot {
                    label: spot.segments.join("."),
                    ty: analysis.typed.pattern(*id)?.to_string(),
                    doc: None,
                }),
                _ => None,
            })
            .collect();
        return Some(spots);
    }
    None
}

/// Whether byte `offset` lies inside a quote, a comment or a doc,
/// where no name is written; an unclosed quote runs to the line's end.
fn in_text(text: &str, offset: usize) -> bool {
    let lexed = lex(text);
    let closed =
        |span: fipe::token::Span| span.start < offset && offset < span.end;
    let open =
        |span: fipe::token::Span| span.start < offset && offset <= span.end;
    lexed.comments.iter().any(|span| open(*span))
        || lexed.tokens.iter().any(|token| match token.kind {
            Token::Str { .. } | Token::Affixed(_) => closed(token.span),
            Token::Doc(_) | Token::InnerDoc(_) => open(token.span),
            _ => false,
        })
        || lexed.errors.iter().any(|error| {
            open(error.span)
                && text[error.span.start..].starts_with(['"', '\''])
        })
}

/// Whether the line so far is a `use` waiting for a module name.
fn is_use(before: &str) -> bool {
    let words: Vec<&str> = before.split_whitespace().collect();
    matches!(words.as_slice(), ["use", ..] | ["pub", "use", ..])
}

/// The host's modules and the file modules beside the document.
fn modules(host: &Host, dir: Option<&Path>) -> Vec<Offer> {
    let mut offers: Vec<Offer> = host
        .declarations
        .modules
        .iter()
        .map(|module| Offer::new(&module.name, CompletionItemKind::MODULE))
        .collect();
    if let Some(dir) = dir {
        let mut stems: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()? == "fipe")
                    .then(|| path.file_stem()?.to_str().map(String::from))?
            })
            .collect();
        stems.sort();
        offers.extend(
            stems
                .iter()
                .map(|stem| Offer::new(stem, CompletionItemKind::MODULE)),
        );
    }
    offers
}

/// The command heading the stage that `before` ends inside, by its
/// name's segments.
fn command(before: &str) -> Option<Vec<String>> {
    let tokens: Vec<Token> = lex(before)
        .tokens
        .into_iter()
        .map(|token| token.kind)
        .filter(|kind| !matches!(kind, Token::Newline | Token::Eof))
        .collect();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, kind) in tokens.iter().enumerate().rev() {
        match kind {
            Token::Symbol(
                Symbol::RParen | Symbol::RBrace | Symbol::RBracket,
            ) => {
                depth += 1;
            }
            Token::Symbol(
                Symbol::LParen | Symbol::LBrace | Symbol::LBracket,
            ) => {
                if depth == 0 {
                    start = index + 1;
                    break;
                }
                depth -= 1;
            }
            Token::Symbol(
                Symbol::Pipe
                | Symbol::PipeGood
                | Symbol::PipeBad
                | Symbol::Or
                | Symbol::And
                | Symbol::Assign
                | Symbol::FatArrow
                | Symbol::Comma,
            ) if depth == 0 => {
                start = index + 1;
                break;
            }
            _ => {}
        }
    }
    match tokens.get(start)? {
        Token::Name(segments) => Some(segments.clone()),
        _ => None,
    }
}

/// The flags of `command`: `-label` for every labeled spot, and
/// `-label=` for one that is not a Bool.
fn flags(analysis: &Analysis, host: &Host, command: &[String]) -> Vec<Offer> {
    let mut offers = Vec::new();
    for (label, is_bool) in spots(analysis, host, command) {
        let mut offer =
            Offer::new(&format!("-{label}"), CompletionItemKind::PROPERTY);
        offer.detail = Some(String::from(if is_bool {
            "flag"
        } else {
            "named argument"
        }));
        offers.push(offer);
        if !is_bool {
            offers.push(Offer::new(
                &format!("-{label}="),
                CompletionItemKind::PROPERTY,
            ));
        }
    }
    offers
}

/// The labeled spots of `command`, each with whether it is a Bool.
fn spots(
    analysis: &Analysis,
    host: &Host,
    command: &[String],
) -> Vec<(String, bool)> {
    match command {
        [name] => document_spots(&analysis.module, name)
            .or_else(|| {
                let commands = host.declarations.commands.as_ref()?;
                let text = host.source("commands")?.1;
                declared_spots(commands, text, name)
            })
            .or_else(|| {
                declared_spots(
                    &host.declarations.prelude,
                    fipe::decl::prelude_source(),
                    name,
                )
            })
            .unwrap_or_default(),
        [module, name] => host
            .declarations
            .get_module(module)
            .and_then(|namespace| {
                declared_spots(namespace, host.source(module)?.1, name)
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The spots of a function the document binds at its top level: the
/// names of its first arm's head.
fn document_spots(module: &Module, name: &str) -> Option<Vec<(String, bool)>> {
    for entry in &module.entries {
        let Entry::Line(id) = entry else { continue };
        let Some(ExprKind::Bind { pattern, value, .. }) =
            module.expr(*id).map(|expr| &expr.kind)
        else {
            continue;
        };
        let Some(PatternKind::Name(bound)) =
            module.pattern(*pattern).map(|pattern| &pattern.kind)
        else {
            continue;
        };
        if bound.segments != [name] {
            continue;
        }
        let ExprKind::Arms(arms) = &module.expr(*value)?.kind else {
            return Some(Vec::new());
        };
        let head = arms.first()?;
        return Some(
            head.patterns
                .iter()
                .filter_map(|id| match &module.pattern(*id)?.kind {
                    PatternKind::Name(name)
                    | PatternKind::Default { name, .. } => {
                        Some((name.segments.join("."), false))
                    }
                    _ => None,
                })
                .collect(),
        );
    }
    None
}

/// The labeled spots of a declared function of `namespace`, whose
/// source is `text`.
fn declared_spots(
    namespace: &Namespace,
    text: &str,
    name: &str,
) -> Option<Vec<(String, bool)>> {
    let Declared::Function { spots, .. } = &namespace.value(name)?.kind else {
        return Some(Vec::new());
    };
    Some(
        spots
            .iter()
            .filter_map(|spot| {
                let label = spot.name.clone()?;
                let span = namespace.module.ty(spot.ty)?.span;
                let ty = text.get(span.start..span.end).unwrap_or_default();
                Some((label, ty.trim() == "Bool"))
            })
            .collect(),
    )
}

/// The members after `head.`: a host or file module's names, or the
/// fields and `self` functions of the value's type.
fn members(
    analysis: &Analysis,
    host: &Host,
    head: &str,
    head_end: usize,
    offset: usize,
    reanalyse: &dyn Fn(&str) -> Analysis,
) -> Vec<Offer> {
    if let Some(namespace) = host.declarations.get_module(head) {
        return namespace_names(namespace);
    }
    if let Some(index) = analysis.files.iter().position(|f| f.stem == head) {
        return analysis
            .file_exports(host, index)
            .into_iter()
            .map(|name| Offer::new(&name, kind_of(&name)))
            .collect();
    }
    // The value's type, from the document with the partial member taken
    // out, so the line parses.
    let text = &analysis.text;
    let shortened = format!("{}{}", &text[..head_end], &text[offset..]);
    let again = reanalyse(&shortened);
    let Some(Node::Expr(id)) = again.module.at(head_end.saturating_sub(1))
    else {
        return Vec::new();
    };
    let Some(ty) = again.typed.expr(id) else {
        return Vec::new();
    };
    let name = crate::analysis::type_head(&ty.to_string());
    let name = name.as_str();
    if name.is_empty() {
        return Vec::new();
    }
    let mut offers = type_members(&again.module, &again.text, name);
    for file in &again.files {
        offers.extend(type_members(&file.module, &file.text, name));
    }
    for (namespace, _, source) in &host.sources {
        let module = if namespace == "commands" {
            host.declarations.commands.as_ref().map(|ns| &ns.module)
        } else {
            host.declarations.get_module(namespace).map(|ns| &ns.module)
        };
        if let Some(module) = module {
            offers.extend(type_members(module, source, name));
        }
    }
    offers.extend(type_members(
        &host.declarations.prelude.module,
        fipe::decl::prelude_source(),
        name,
    ));
    offers
}

/// The public names of a declared namespace.
fn namespace_names(namespace: &Namespace) -> Vec<Offer> {
    namespace
        .values
        .iter()
        .chain(&namespace.types)
        .filter(|declared| declared.public || namespace.name == "prelude")
        .map(|declared| Offer::new(&declared.name, kind_of(&declared.name)))
        .collect()
}

/// The fields and impl members of the type `name` in `module`, whose
/// source is `text`.
fn type_members(module: &Module, text: &str, name: &str) -> Vec<Offer> {
    let mut offers = Vec::new();
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
                        let mut offer = Offer::new(
                            &field.name.segments.join("."),
                            CompletionItemKind::FIELD,
                        );
                        offer.detail = module
                            .ty(field.ty)
                            .and_then(|ty| text.get(ty.span.start..ty.span.end))
                            .map(String::from);
                        offers.push(offer);
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
                if crate::analysis::target_head(target) != name {
                    continue;
                }
                for member in members {
                    if let ImplMember::Named { name, .. } = member {
                        offers.push(Offer::new(
                            &name.segments.join("."),
                            CompletionItemKind::METHOD,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    offers
}

/// The kind a name's case suggests.
fn kind_of(name: &str) -> CompletionItemKind {
    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        CompletionItemKind::CLASS
    } else {
        CompletionItemKind::FUNCTION
    }
}

/// The names in scope at `offset`: the document's own, lexical ones
/// around the offset, the file and host modules, the host's commands
/// and the prelude.
fn in_scope(analysis: &Analysis, host: &Host, offset: usize) -> Vec<Offer> {
    let module = &analysis.module;
    let mut offers = Vec::new();
    for entry in &module.entries {
        match entry {
            Entry::Item(item) => item_names(item, &mut offers),
            Entry::Line(id) => {
                let Some(expr) = module.expr(*id) else {
                    continue;
                };
                if let ExprKind::Bind { pattern, .. } = &expr.kind {
                    if expr.span.start < offset {
                        pattern_names(module, *pattern, &mut offers);
                    }
                }
                if expr.span.start <= offset && offset <= expr.span.end {
                    locals(module, *id, offset, &mut offers);
                }
            }
        }
    }
    offers.extend(
        analysis
            .files
            .iter()
            .map(|file| Offer::new(&file.stem, CompletionItemKind::MODULE)),
    );
    offers.extend(modules(host, None));
    if let Some(commands) = &host.declarations.commands {
        offers.extend(namespace_names(commands));
    }
    offers.extend(namespace_names(&host.declarations.prelude));
    offers
}

/// The names an item puts in scope.
fn item_names(item: &fipe::ast::Item, offers: &mut Vec<Offer>) {
    match &item.kind {
        ItemKind::Type { name, members, .. } => {
            offers.push(Offer::new(
                &name.segments.join("."),
                CompletionItemKind::CLASS,
            ));
            for member in members.iter().flatten() {
                if let Member::Constructor { name, .. } = member {
                    offers.push(Offer::new(
                        &name.segments.join("."),
                        CompletionItemKind::CONSTRUCTOR,
                    ));
                }
            }
        }
        ItemKind::Use { names, .. } => {
            for name in names {
                if let Some(last) = name.segments.last() {
                    let kind = if name.segments.len() == 1 {
                        CompletionItemKind::MODULE
                    } else {
                        kind_of(last)
                    };
                    offers.push(Offer::new(last, kind));
                }
            }
        }
        ItemKind::Declare { name, .. } => {
            offers.push(Offer::new(
                &name.segments.join("."),
                CompletionItemKind::FUNCTION,
            ));
        }
        ItemKind::Impl { .. } => {}
    }
}

/// The names a pattern binds.
fn pattern_names(module: &Module, id: PatternId, offers: &mut Vec<Offer>) {
    let Some(pattern) = module.pattern(id) else {
        return;
    };
    match &pattern.kind {
        PatternKind::Name(name) | PatternKind::Default { name, .. } => {
            if !name.is_uppercase() {
                offers.push(Offer::new(
                    &name.segments.join("."),
                    CompletionItemKind::VARIABLE,
                ));
            }
        }
        PatternKind::Constructor { payload, .. } => {
            pattern_names(module, *payload, offers);
        }
        PatternKind::Group(inner) => pattern_names(module, *inner, offers),
        PatternKind::Tuple(parts) | PatternKind::List(parts) => {
            for part in parts {
                pattern_names(module, *part, offers);
            }
        }
        PatternKind::Outcome {
            payload: Some(payload),
            ..
        } => pattern_names(module, *payload, offers),
        PatternKind::Record { fields, .. } => {
            for field in fields {
                match field.pattern {
                    Some(inner) => pattern_names(module, inner, offers),
                    None => offers.push(Offer::new(
                        &field.name.segments.join("."),
                        CompletionItemKind::VARIABLE,
                    )),
                }
            }
        }
        PatternKind::Wildcard
        | PatternKind::Literal(_)
        | PatternKind::Outcome { payload: None, .. } => {}
    }
}

/// The lexical names of expression `id` visible at `offset`, which it
/// holds.
fn locals(module: &Module, id: ExprId, offset: usize, offers: &mut Vec<Offer>) {
    let Some(expr) = module.expr(id) else { return };
    let holds = |id: ExprId| {
        module
            .expr(id)
            .is_some_and(|e| e.span.start <= offset && offset <= e.span.end)
    };
    match &expr.kind {
        ExprKind::Arms(arms) => {
            for arm in arms {
                if arm.span.start <= offset && offset <= arm.span.end {
                    for pattern in &arm.patterns {
                        pattern_names(module, *pattern, offers);
                    }
                    for inner in arm.guard.iter().chain([&arm.body]) {
                        if holds(*inner) {
                            locals(module, *inner, offset, offers);
                        }
                    }
                }
            }
        }
        ExprKind::Block(lines) => {
            for line in lines {
                let Some(inner) = module.expr(*line) else {
                    continue;
                };
                if let ExprKind::Bind { pattern, .. } = &inner.kind {
                    if inner.span.end <= offset {
                        pattern_names(module, *pattern, offers);
                    }
                }
                if holds(*line) {
                    locals(module, *line, offset, offers);
                }
            }
        }
        kind => {
            for child in children(kind) {
                if holds(child) {
                    locals(module, child, offset, offers);
                }
            }
        }
    }
}

/// The direct sub-expressions of an expression other than arms and
/// blocks.
fn children(kind: &ExprKind) -> Vec<ExprId> {
    match kind {
        ExprKind::Group(ids) | ExprKind::Tuple(ids) | ExprKind::List(ids) => {
            ids.clone()
        }
        ExprKind::Outcome { payload, .. } => payload.iter().copied().collect(),
        ExprKind::Record { fields, .. } => {
            fields.iter().map(|field| field.value).collect()
        }
        ExprKind::Call {
            callee, arguments, ..
        } => {
            let mut ids = vec![*callee];
            for argument in arguments {
                match &argument.kind {
                    fipe::ast::ArgumentKind::Positional(id) => ids.push(*id),
                    fipe::ast::ArgumentKind::Flag { value, .. } => {
                        ids.extend(value.iter().copied());
                    }
                }
            }
            ids
        }
        ExprKind::Field { value, .. } | ExprKind::Not { value, .. } => {
            vec![*value]
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipe { left, right, .. } => vec![*left, *right],
        ExprKind::Bind { value, .. } => vec![*value],
        ExprKind::With { value, body, .. } => vec![*value, *body],
        ExprKind::If {
            condition,
            then_body,
            else_body,
        } => {
            let mut ids = vec![*condition, *then_body];
            ids.extend(else_body.iter().copied());
            ids
        }
        ExprKind::Block(ids) => ids.clone(),
        ExprKind::Arms(arms) => arms.iter().map(|arm| arm.body).collect(),
        ExprKind::Literal(_)
        | ExprKind::Name(_)
        | ExprKind::Word(_)
        | ExprKind::Path(_)
        | ExprKind::Selector(_)
        | ExprKind::Instance { .. } => Vec::new(),
    }
}
