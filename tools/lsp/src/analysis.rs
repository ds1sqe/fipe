//! One document's stages, from its text to its check diagnostics, and
//! the queries the actions ask of them.

use std::path::{Path, PathBuf};

use fipe::ast::{ExprKind, Module, Node};
use fipe::check::{check, Options};
use fipe::infer::infer_partial;
use fipe::lex::lex;
use fipe::parse::parse_partial;
use fipe::resolve::{resolve_partial, Origin, Resolved, Target};
use fipe::typed::Typed;
use fipe::types::Scheme;

use crate::loader::{self, File, Host};
use crate::words::{self, Placed};

/// Where a span lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The document itself.
    This,
    /// A file module the document uses, by index into `files`.
    File(usize),
    /// A host declaration file, by namespace.
    Host(String),
    /// The prelude, which has no file.
    Prelude,
}

/// A located span.
pub type Located = (Source, (usize, usize));

/// A document analysed against the host and the files it uses.
pub struct Analysis {
    pub text: String,
    pub module: Module,
    pub files: Vec<File>,
    /// `files` as the crate takes them.
    pairs: Vec<(String, Module)>,
    pub resolved: Resolved,
    pub typed: Typed,
    /// Each diagnostic of the document, by byte span.
    pub diagnostics: Vec<((usize, usize), String)>,
}

impl Analysis {
    /// Analyse `text`, read from `path` when it has one: its file
    /// modules are looked up beside it, through `open` first.
    #[must_use]
    pub fn new(
        text: &str,
        path: Option<&Path>,
        host: &Host,
        open: &dyn Fn(&Path) -> Option<String>,
    ) -> Self {
        let module = parse_partial(lex(text));
        let dir = path.and_then(Path::parent).map(PathBuf::from);
        let files = dir.map_or_else(Vec::new, |dir| {
            loader::files(&dir, &module, &host.declarations, open)
        });
        let pairs: Vec<(String, Module)> = files
            .iter()
            .map(|file| (file.stem.clone(), file.module.clone()))
            .collect();
        let (resolved, _) =
            resolve_partial(&module, &host.declarations, &pairs);
        let (typed, _) =
            infer_partial(&module, &resolved, &host.declarations, &pairs);
        let mut analysis = Self {
            text: String::from(text),
            module,
            files,
            pairs,
            resolved,
            typed,
            diagnostics: Vec::new(),
        };
        let stem = path
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str());
        analysis.diagnostics = analysis.check(host, stem);
        analysis
    }

    fn check(
        &self,
        host: &Host,
        stem: Option<&str>,
    ) -> Vec<((usize, usize), String)> {
        let mut diagnostics: Vec<_> = host
            .errors
            .iter()
            .map(|error| ((0, 0), format!("host declarations: {error}")))
            .collect();
        let Err(errors) = check(
            &self.module,
            &host.declarations,
            &self.pairs,
            Options::default(),
        ) else {
            return diagnostics;
        };
        let mut here = Vec::new();
        // Each used file's diagnostics: the first one's words, and how
        // many more, shown once at the file's `use`.
        let mut files: Vec<(String, (usize, usize), String, usize)> =
            Vec::new();
        for error in &errors {
            match words::check(error) {
                Placed::Here(span, message) => here.push((span, message)),
                // A file module that is the document itself, reached
                // through a cycle: its words belong here.
                Placed::File(file, span, message)
                    if Some(file.as_str()) == stem =>
                {
                    here.push((span, message));
                }
                Placed::File(file, (start, _), message) => {
                    if let Some(entry) =
                        files.iter_mut().find(|entry| entry.0 == file)
                    {
                        entry.3 += 1;
                        continue;
                    }
                    let at = loader::uses(&self.module)
                        .find(|(used, _)| *used == file)
                        .map_or((0, 0), |(_, span)| span);
                    let line = self
                        .files
                        .iter()
                        .find(|known| known.stem == file)
                        .map_or(0, |known| {
                            known.text[..start.min(known.text.len())]
                                .matches('\n')
                                .count()
                                + 1
                        });
                    let text =
                        format!("in {file}.fipe, line {line}: {message}");
                    files.push((file, at, text, 0));
                }
            }
        }
        // What the document says of a broken file's names follows from
        // the file's own errors.
        here.retain(|((start, end), _)| {
            let said = self.text.get(*start..*end).unwrap_or_default();
            !files.iter().any(|(file, ..)| {
                said == file || said.starts_with(&format!("{file}."))
            })
        });
        diagnostics.extend(here);
        for (_, at, text, more) in files {
            let text = match more {
                0 => text,
                1 => format!("{text} (and 1 more)"),
                n => format!("{text} (and {n} more)"),
            };
            diagnostics.push((at, text));
        }
        diagnostics
    }

    /// The span of a node of `module`.
    #[must_use]
    pub fn span(module: &Module, node: Node) -> Option<(usize, usize)> {
        let span = match node {
            Node::Expr(id) => module.expr(id)?.span,
            Node::Pattern(id) => module.pattern(id)?.span,
            Node::Type(id) => module.ty(id)?.span,
        };
        Some((span.start, span.end))
    }

    /// The start of the name segment at `offset`, or `offset` when no
    /// name is there: a cursor just after a name is on the name.
    #[must_use]
    pub fn on_name(&self, offset: usize) -> usize {
        name_at(&self.text, offset).map_or(offset, |(start, _)| start)
    }

    /// Whether `name` is written in the document as a name or the
    /// first segment of one.
    #[must_use]
    pub fn mentions(&self, name: &str) -> bool {
        lex(&self.text).tokens.iter().any(|token| {
            matches!(&token.kind, fipe::token::Kind::Name(segments)
                if segments.first().is_some_and(|first| first == name))
        })
    }

    /// The node under byte `offset` and what it names.
    #[must_use]
    pub fn target_at(&self, offset: usize) -> Option<(Node, Target)> {
        let node = self.module.at(offset)?;
        let target = match node {
            Node::Expr(id) => self.resolved.expr(id),
            Node::Pattern(id) => self.resolved.pattern(id),
            Node::Type(id) => self.resolved.ty(id),
        }?;
        Some((node, target.clone()))
    }

    /// What the `use` segment under byte `offset` names: its module, or
    /// what its imported name names in each namespace.
    #[must_use]
    pub fn import_at(&self, offset: usize) -> Option<Vec<Target>> {
        let within = |span: fipe::token::Span| {
            span.start <= offset && offset <= span.end
        };
        self.resolved.imports.iter().find_map(|import| {
            if within(import.module_span) {
                Some(vec![Target::Module(import.module.clone())])
            } else if import.name_span.is_some_and(within)
                && !import.targets.is_empty()
            {
                Some(import.targets.clone())
            } else {
                None
            }
        })
    }

    /// What byte `offset` names: the node's target, or a `use`
    /// segment's targets.
    #[must_use]
    pub fn targets_at(&self, offset: usize) -> Vec<Target> {
        self.target_at(offset).map_or_else(
            || self.import_at(offset).unwrap_or_default(),
            |(_, target)| vec![target],
        )
    }

    /// What the declaration under byte `offset` defines: a top-level
    /// value, type or constructor of the document, or a local's binder.
    #[must_use]
    pub fn declaration_at(&self, offset: usize) -> Option<Target> {
        let (start, end) = name_at(&self.text, offset)?;
        let word = self.text.get(start..end)?;
        let mut candidates = vec![
            Target::Value {
                origin: Origin::This,
                name: String::from(word),
            },
            Target::Type {
                origin: Origin::This,
                name: String::from(word),
            },
        ];
        for entry in &self.module.entries {
            let fipe::ast::Entry::Item(item) = entry else {
                continue;
            };
            let fipe::ast::ItemKind::Type {
                name,
                members: Some(members),
                ..
            } = &item.kind
            else {
                continue;
            };
            for member in members {
                if let fipe::ast::Member::Constructor { name: built, .. } =
                    member
                {
                    if built.segments == [word] {
                        candidates.push(Target::Constructor {
                            origin: Origin::This,
                            ty: name.segments.join("."),
                            name: String::from(word),
                        });
                    }
                }
            }
        }
        let at =
            |span: fipe::token::Span| span.start <= start && end <= span.end;
        if let Some(found) = candidates
            .into_iter()
            .find(|candidate| self.resolved.site(candidate).is_some_and(at))
        {
            return Some(found);
        }
        let Some(Node::Pattern(id)) = self.module.at(offset) else {
            return None;
        };
        match &self.module.pattern(id)?.kind {
            fipe::ast::PatternKind::Name(name)
            | fipe::ast::PatternKind::Default { name, .. } => {
                Some(Target::Local { site: name.span })
            }
            _ => None,
        }
    }

    fn file_index(&self, stem: &str) -> Option<usize> {
        self.files.iter().position(|file| file.stem == stem)
    }

    /// The resolution of file module `index`, as a used module.
    fn file_resolved(&self, host: &Host, index: usize) -> Resolved {
        let module = &self.files[index].module;
        resolve_partial(module, &host.declarations, &self.pairs).0
    }

    /// The names file module `index` exports.
    #[must_use]
    pub fn file_exports(&self, host: &Host, index: usize) -> Vec<String> {
        self.file_resolved(host, index)
            .exports
            .into_iter()
            .map(|export| export.name)
            .collect()
    }

    /// Where `target` is defined.
    #[must_use]
    pub fn definition(&self, host: &Host, target: &Target) -> Vec<Located> {
        if let Target::Local { site } = target {
            return vec![(Source::This, (site.start, site.end))];
        }
        match target {
            Target::TypeMember { ty, member } => {
                return match ty.as_ref() {
                    Target::Type { name, .. } => {
                        self.member_sites(host, name, member)
                    }
                    _ => Vec::new(),
                };
            }
            Target::Members { head, members } => {
                let [member] = members.as_slice() else {
                    return Vec::new();
                };
                return self
                    .head_type(head)
                    .map(|name| self.member_sites(host, &name, member))
                    .unwrap_or_default();
            }
            _ => {}
        }
        match target {
            Target::Module(Origin::File(stem)) => {
                return self
                    .file_index(stem)
                    .map(|index| (Source::File(index), (0, 0)))
                    .into_iter()
                    .collect();
            }
            Target::Module(Origin::Host(name)) => {
                return host
                    .source(name)
                    .map(|_| (Source::Host(name.clone()), (0, 0)))
                    .into_iter()
                    .collect();
            }
            _ => {}
        }
        let Some(origin) = origin(target) else {
            return Vec::new();
        };
        match origin {
            Origin::This => self
                .resolved
                .site(target)
                .map(|span| (Source::This, (span.start, span.end)))
                .into_iter()
                .collect(),
            Origin::File(stem) => {
                let Some(index) = self.file_index(stem) else {
                    return Vec::new();
                };
                let local = with_origin(target, Origin::This);
                let resolved = self.file_resolved(host, index);
                // A value binding is a local of its own module; its
                // export names its site.
                resolved
                    .site(&local)
                    .or_else(|| export_site(&resolved, &local))
                    .map(|span| (Source::File(index), (span.start, span.end)))
                    .into_iter()
                    .collect()
            }
            Origin::Commands | Origin::Host(_) | Origin::Prelude => {
                declared(host, origin, target).into_iter().collect()
            }
            Origin::Line(_) => Vec::new(),
        }
    }

    /// Every use of `target` in the document and the file modules it
    /// uses, and its definition when `declaration` is set.
    #[must_use]
    pub fn references(
        &self,
        host: &Host,
        target: &Target,
        declaration: bool,
    ) -> Vec<Located> {
        let defined_in = match origin(target) {
            Some(Origin::File(stem)) => self.file_index(stem),
            _ => None,
        };
        let mut found = Vec::new();
        for node in self.resolved.uses(target) {
            if let Some(span) = Self::span(&self.module, node) {
                found.push((Source::This, span));
            }
        }
        for span in import_spans(&self.resolved, target) {
            found.push((Source::This, span));
        }
        // A file module sees a target of its own as `This`, and one of
        // another file module by that module's stem.
        let file_module = matches!(target, Target::Module(Origin::File(_)));
        if file_module || matches!(origin(target), Some(Origin::File(_))) {
            for (index, file) in self.files.iter().enumerate() {
                let local = if Some(index) == defined_in {
                    with_origin(target, Origin::This)
                } else {
                    target.clone()
                };
                let resolved = self.file_resolved(host, index);
                let own = (Some(index) == defined_in)
                    .then(|| export_site(&resolved, &local))
                    .flatten()
                    .map(|site| Target::Local { site });
                let mut nodes: Vec<Node> = resolved.uses(&local).collect();
                if let Some(own) = &own {
                    nodes.extend(resolved.uses(own));
                }
                for node in nodes {
                    if let Some(span) = Self::span(&file.module, node) {
                        found.push((Source::File(index), span));
                    }
                }
                for span in import_spans(&resolved, &local) {
                    found.push((Source::File(index), span));
                }
            }
        }
        if declaration {
            for located in self.definition(host, target) {
                if !found.contains(&located) {
                    found.push(located);
                }
            }
        }
        found
    }

    /// The scheme of the binding whose name pattern spans `site`.
    fn scheme_at(&self, site: (usize, usize)) -> Option<&Scheme> {
        self.typed.bindings().find_map(|(id, scheme)| {
            let ExprKind::Bind { pattern, .. } = &self.module.expr(id)?.kind
            else {
                return None;
            };
            let span = self.module.pattern(*pattern)?.span;
            ((span.start, span.end) == site).then_some(scheme)
        })
    }

    /// What hovering byte `offset` shows: a binding's scheme, an
    /// expression's inferred type, or a host or prelude declaration.
    #[must_use]
    pub fn hover(&self, host: &Host, offset: usize) -> Option<String> {
        if let Some(start) = self.item_name_at(offset) {
            return Some(doc_and_lines(&self.text, start));
        }
        if self.module.at(offset).is_none() {
            return self.import_hover(host, offset);
        }
        let node = self.module.at(offset)?;
        let target = self.target_at(offset).map(|(_, target)| target);
        let word = Self::span(&self.module, node)
            .and_then(|(start, end)| self.text.get(start..end))
            .unwrap_or_default();
        if let Some(target) = &target {
            if let Some(
                origin @ (Origin::Commands | Origin::Host(_) | Origin::Prelude),
            ) = origin(target)
            {
                if let Some(text) = declaration_text(host, origin, target) {
                    return Some(text);
                }
            }
            let site = match target {
                Target::Local { site } => Some((site.start, site.end)),
                Target::Value {
                    origin: Origin::This,
                    ..
                } => self
                    .resolved
                    .site(target)
                    .map(|span| (span.start, span.end)),
                _ => None,
            };
            if let Some(scheme) = site.and_then(|site| self.scheme_at(site)) {
                return Some(format!("{word}: {scheme}"));
            }
        }
        match node {
            Node::Pattern(id) => {
                let span = Self::span(&self.module, node)?;
                if let Some(scheme) = self.scheme_at(span) {
                    return Some(format!("{word}: {scheme}"));
                }
                let ty = self.typed.pattern(id)?;
                Some(format!("{word}: {ty}"))
            }
            Node::Expr(id) => {
                let ty = self.typed.expr(id)?;
                Some(format!("{word}: {ty}"))
            }
            Node::Type(_) => None,
        }
    }
}

impl Analysis {
    /// The start of the declared name under byte `offset` in one of the
    /// document's items: a type, a constructor, a field, an impl member
    /// or a declaration, which are names, not nodes.
    #[must_use]
    pub fn item_name_at(&self, offset: usize) -> Option<usize> {
        let on = |name: &fipe::ast::Name| {
            name.span.start <= offset && offset <= name.span.end
        };
        for entry in &self.module.entries {
            let fipe::ast::Entry::Item(item) = entry else {
                continue;
            };
            let mut names: Vec<&fipe::ast::Name> = Vec::new();
            match &item.kind {
                fipe::ast::ItemKind::Type { name, members, .. } => {
                    names.push(name);
                    for member in members.iter().flatten() {
                        match member {
                            fipe::ast::Member::Field(field) => {
                                names.push(&field.name);
                            }
                            fipe::ast::Member::Constructor {
                                name,
                                fields,
                                ..
                            } => {
                                names.push(name);
                                names.extend(fields.iter().map(|f| &f.name));
                            }
                        }
                    }
                }
                fipe::ast::ItemKind::Impl { members, .. } => {
                    for member in members {
                        if let fipe::ast::ImplMember::Named { name, .. } =
                            member
                        {
                            names.push(name);
                        }
                    }
                }
                fipe::ast::ItemKind::Declare { name, .. } => names.push(name),
                fipe::ast::ItemKind::Use { .. } => {}
            }
            if let Some(name) = names.into_iter().find(|name| on(name)) {
                return Some(name.span.start);
            }
        }
        None
    }

    /// The name of the type of a member chain's head, from its typing.
    fn head_type(&self, head: &Target) -> Option<String> {
        let shown = match head {
            // A top-level binding is a local of its own module, typed by
            // its scheme; a lexical one by its pattern.
            Target::Local { site } => {
                match self.scheme_at((site.start, site.end)) {
                    Some(scheme) => scheme.to_string(),
                    None => match self.module.at(site.start)? {
                        Node::Pattern(id) => {
                            self.typed.pattern(id)?.to_string()
                        }
                        _ => return None,
                    },
                }
            }
            Target::Value {
                origin: Origin::This,
                ..
            } => {
                let site = self.resolved.site(head)?;
                self.scheme_at((site.start, site.end))?.to_string()
            }
            _ => return None,
        };
        let name = type_head(&shown);
        (!name.is_empty()).then_some(name)
    }

    /// Where the type named `ty` declares `member`, as a field, a
    /// constructor or an impl member, in the document, its file
    /// modules, the host's declaration files and the prelude.
    #[must_use]
    pub fn member_sites(
        &self,
        host: &Host,
        ty: &str,
        member: &str,
    ) -> Vec<Located> {
        let mut found = Vec::new();
        let mut search = |module: &Module, text: &str, source: Source| {
            for span in member_spans(module, text, ty, member) {
                found.push((source.clone(), span));
            }
        };
        search(&self.module, &self.text, Source::This);
        for (index, file) in self.files.iter().enumerate() {
            search(&file.module, &file.text, Source::File(index));
        }
        for (namespace, _, text) in &host.sources {
            let declared = if namespace == "commands" {
                host.declarations.commands.as_ref()
            } else {
                host.declarations.get_module(namespace)
            };
            if let Some(declared) = declared {
                search(&declared.module, text, Source::Host(namespace.clone()));
            }
        }
        search(
            &host.declarations.prelude.module,
            fipe::decl::prelude_source(),
            Source::Prelude,
        );
        found
    }

    /// What hovering a `use` segment shows: the module it names, or the
    /// declaration of the name it imports.
    fn import_hover(&self, host: &Host, offset: usize) -> Option<String> {
        let target = self.import_at(offset)?.into_iter().next()?;
        match &target {
            Target::Module(Origin::File(stem)) => {
                Some(format!("module {stem}: {stem}.fipe"))
            }
            Target::Module(Origin::Host(name)) => {
                Some(format!("host module {name}"))
            }
            Target::Module(_) => None,
            _ => match origin(&target)? {
                origin @ (Origin::Commands
                | Origin::Host(_)
                | Origin::Prelude) => declaration_text(host, origin, &target),
                Origin::File(_) => {
                    let (Source::File(index), (start, _)) =
                        self.definition(host, &target).into_iter().next()?
                    else {
                        return None;
                    };
                    Some(doc_and_lines(&self.files[index].text, start))
                }
                _ => None,
            },
        }
    }
}

/// The spans in `resolved`'s `use` items that name `target`: a module's
/// segment, or an imported name's.
#[must_use]
pub fn import_spans(
    resolved: &Resolved,
    target: &Target,
) -> Vec<(usize, usize)> {
    resolved
        .imports
        .iter()
        .filter_map(|import| {
            let span = match target {
                Target::Module(module) => {
                    (import.module == *module).then_some(import.module_span)
                }
                _ => import
                    .targets
                    .contains(target)
                    .then_some(import.name_span)
                    .flatten(),
            }?;
            Some((span.start, span.end))
        })
        .collect()
}

/// The site of the export of `resolved` that names `target`.
#[must_use]
pub fn export_site(
    resolved: &Resolved,
    target: &Target,
) -> Option<fipe::token::Span> {
    resolved
        .exports
        .iter()
        .find(|export| export.target == *target)
        .map(|export| export.span)
}

/// The origin of a declared target.
const fn origin(target: &Target) -> Option<&Origin> {
    match target {
        Target::Value { origin, .. }
        | Target::Constructor { origin, .. }
        | Target::Type { origin, .. } => Some(origin),
        _ => None,
    }
}

/// `target` with its origin replaced.
#[must_use]
pub fn with_origin(target: &Target, origin: Origin) -> Target {
    match target {
        Target::Value { name, .. } => Target::Value {
            origin,
            name: name.clone(),
        },
        Target::Constructor { ty, name, .. } => Target::Constructor {
            origin,
            ty: ty.clone(),
            name: name.clone(),
        },
        Target::Type { name, .. } => Target::Type {
            origin,
            name: name.clone(),
        },
        other => other.clone(),
    }
}

/// The namespace of a host or prelude origin, and where it lies.
fn namespace<'a>(
    host: &'a Host,
    origin: &Origin,
) -> Option<(&'a fipe::decl::Namespace, Source)> {
    let declarations = &host.declarations;
    match origin {
        Origin::Prelude => Some((&declarations.prelude, Source::Prelude)),
        Origin::Commands => declarations
            .commands
            .as_ref()
            .map(|commands| (commands, Source::Host(String::from("commands")))),
        Origin::Host(name) => declarations
            .get_module(name)
            .map(|module| (module, Source::Host(name.clone()))),
        _ => None,
    }
}

/// The declaration of a host or prelude target.
fn declared(
    host: &Host,
    origin: &Origin,
    target: &Target,
) -> Option<(Source, (usize, usize))> {
    let (namespace, source) = namespace(host, origin)?;
    let found = match target {
        Target::Type { name, .. } => namespace.ty(name),
        Target::Value { name, .. } | Target::Constructor { name, .. } => {
            namespace.value(name).or_else(|| namespace.ty(name))
        }
        _ => None,
    }?;
    Some((source, (found.span.start, found.span.end)))
}

/// The text of a host or prelude declaration: its doc lines and the
/// lines of its signature.
fn declaration_text(
    host: &Host,
    origin: &Origin,
    target: &Target,
) -> Option<String> {
    let (source, (start, _)) = declared(host, origin, target)?;
    let text = match source {
        Source::Prelude => fipe::decl::prelude_source(),
        Source::Host(name) => host.source(&name)?.1,
        _ => return None,
    };
    Some(doc_and_lines(text, start))
}

/// The doc lines above the line holding byte `start` of `text`, that
/// line, and the lines a trailing `->` continues it onto.
fn doc_and_lines(text: &str, start: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let line = text[..start.min(text.len())].matches('\n').count();
    let mut first = line;
    while first > 0 && lines[first - 1].trim_start().starts_with("##") {
        first -= 1;
    }
    let mut last = line;
    while last + 1 < lines.len() && lines[last].trim_end().ends_with("->") {
        last += 1;
    }
    lines[first..=last.min(lines.len().saturating_sub(1))].join("\n")
}

/// The span of the name segment of `text` under byte `offset`, by the
/// crate's tokens: a dotted name's segment, or the whole name. A
/// position at a name's end is on it.
#[must_use]
pub fn name_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    let lexed = lex(text);
    let token = lexed.tokens.iter().find(|token| {
        matches!(token.kind, fipe::token::Kind::Name(_))
            && token.span.start <= offset
            && offset <= token.span.end
    })?;
    let (start, end) = (token.span.start, token.span.end);
    let mut at = start;
    for segment in text.get(start..end)?.split('.') {
        let segment_end = at + segment.len();
        if at <= offset && offset <= segment_end {
            return Some((at, segment_end));
        }
        at = segment_end + 1;
    }
    Some((start, end))
}

/// The spans of the names of `member` that the type named `ty` declares
/// in `module`, whose text is `text`.
fn member_spans(
    module: &Module,
    text: &str,
    ty: &str,
    member: &str,
) -> Vec<(usize, usize)> {
    use fipe::ast::{Entry, ImplMember, ItemKind, Member};
    let named = |name: &fipe::ast::Name| name.segments == [member];
    let mut spans = Vec::new();
    for entry in &module.entries {
        let Entry::Item(item) = entry else { continue };
        match &item.kind {
            ItemKind::Type {
                name,
                members: Some(members),
                ..
            } if name.segments.last().is_some_and(|last| last == ty) => {
                for declared in members {
                    let name = match declared {
                        Member::Field(field) => &field.name,
                        Member::Constructor { name, .. } => name,
                    };
                    if named(name) {
                        spans.push((name.span.start, name.span.end));
                    }
                }
            }
            ItemKind::Impl {
                target, members, ..
            } => {
                let written = module
                    .ty(*target)
                    .and_then(|t| text.get(t.span.start..t.span.end))
                    .unwrap_or_default();
                if target_head(written) != ty {
                    continue;
                }
                for declared in members {
                    if let ImplMember::Named { name, .. } = declared {
                        if named(name) {
                            spans.push((name.span.start, name.span.end));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

/// The name heading a printed type, past its limits (`Literal a =>
/// Vec<a>` is `Vec`); `[]` for an array, whose impls are written on
/// `[T; n]`; empty for a tuple (`a * b`), a function or a variable's
/// punctuation.
#[must_use]
pub fn type_head(shown: &str) -> String {
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
#[must_use]
pub fn target_head(written: &str) -> &str {
    if written.trim_start().starts_with('[') {
        "[]"
    } else {
        written.split(['<', ' ']).next().unwrap_or_default().trim()
    }
}
