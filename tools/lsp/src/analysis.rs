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
        analysis.diagnostics = analysis.check(host);
        analysis
    }

    fn check(&self, host: &Host) -> Vec<((usize, usize), String)> {
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
        for error in &errors {
            match words::check(error) {
                Placed::Here(span, message) => {
                    diagnostics.push((span, message));
                }
                Placed::File(stem, (start, _), message) => {
                    let at = loader::uses(&self.module)
                        .find(|(used, _)| *used == stem)
                        .map_or((0, 0), |(_, span)| span);
                    let line = self
                        .files
                        .iter()
                        .find(|file| file.stem == stem)
                        .map_or(0, |file| {
                            file.text[..start.min(file.text.len())]
                                .matches('\n')
                                .count()
                                + 1
                        });
                    diagnostics.push((
                        at,
                        format!("in {stem}.fipe, line {line}: {message}"),
                    ));
                }
            }
        }
        diagnostics
    }

    /// The span of a node of `module`.
    fn span(module: &Module, node: Node) -> Option<(usize, usize)> {
        let span = match node {
            Node::Expr(id) => module.expr(id)?.span,
            Node::Pattern(id) => module.pattern(id)?.span,
            Node::Type(id) => module.ty(id)?.span,
        };
        Some((span.start, span.end))
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

    fn file_index(&self, stem: &str) -> Option<usize> {
        self.files.iter().position(|file| file.stem == stem)
    }

    /// The resolution of file module `index`, as a used module.
    fn file_resolved(&self, host: &Host, index: usize) -> Resolved {
        let module = &self.files[index].module;
        resolve_partial(module, &host.declarations, &self.pairs).0
    }

    /// Where `target` is defined.
    #[must_use]
    pub fn definition(&self, host: &Host, target: &Target) -> Vec<Located> {
        if let Target::Local { site } = target {
            return vec![(Source::This, (site.start, site.end))];
        }
        if let Target::Module(Origin::File(stem)) = target {
            return self
                .file_index(stem)
                .map(|index| (Source::File(index), (0, 0)))
                .into_iter()
                .collect();
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
                self.file_resolved(host, index)
                    .site(&local)
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
        // A file module sees a target of its own as `This`, and one of
        // another file module by that module's stem.
        if matches!(origin(target), Some(Origin::File(_))) {
            for (index, file) in self.files.iter().enumerate() {
                let local = if Some(index) == defined_in {
                    with_origin(target, Origin::This)
                } else {
                    target.clone()
                };
                let resolved = self.file_resolved(host, index);
                for node in resolved.uses(&local) {
                    if let Some(span) = Self::span(&file.module, node) {
                        found.push((Source::File(index), span));
                    }
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
            Node::Pattern(_) => {
                let span = Self::span(&self.module, node)?;
                let scheme = self.scheme_at(span)?;
                Some(format!("{word}: {scheme}"))
            }
            Node::Expr(id) => {
                let ty = self.typed.expr(id)?;
                Some(format!("{word}: {ty}"))
            }
            Node::Type(_) => None,
        }
    }
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
fn with_origin(target: &Target, origin: Origin) -> Target {
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
    let lines: Vec<&str> = text.lines().collect();
    let line = text[..start.min(text.len())].matches('\n').count();
    let mut first = line;
    while first > 0 && lines[first - 1].trim_start().starts_with("##") {
        first -= 1;
    }
    let mut last = line;
    while last + 1 < lines.len() && lines[last].trim_end().ends_with("->") {
        last += 1;
    }
    Some(lines[first..=last].join("\n"))
}
