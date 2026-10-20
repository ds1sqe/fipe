//! Sources and positions: the text of each namespace a span may be in,
//! byte offsets as one-based lines and scalar columns, and the pipe
//! marks of a program, each with the column a stage breakpoint names.
use fipe::{
    decl,
    ir::{Kind, Tables},
    token::{Source, Span},
};
use std::path::{Path, PathBuf};

/// One source a span may be in: a namespace's name, its file when it
/// has one, and its text.
#[derive(Clone, Debug)]
pub struct File {
    pub name: String,
    pub path: Option<PathBuf>,
    pub text: String,
}

/// The sources of a launched program, by namespace name.
#[derive(Clone, Debug, Default)]
pub struct Files {
    files: Vec<File>,
}

impl Files {
    /// Add a source.
    pub fn push(&mut self, name: &str, path: Option<PathBuf>, text: String) {
        self.files.push(File {
            name: name.to_string(),
            path,
            text,
        });
    }

    /// The source of a namespace; the prelude's text is the crate's.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|file| file.name == name)
    }

    /// The text of a namespace, empty when unknown.
    #[must_use]
    pub fn text(&self, name: &str) -> &str {
        if name == "prelude" {
            return decl::prelude_source();
        }
        self.get(name).map_or("", |file| file.text.as_str())
    }

    /// The namespace whose file is `path`.
    #[must_use]
    pub fn named(&self, path: &Path) -> Option<&str> {
        self.files
            .iter()
            .find(|file| {
                file.path.as_deref().is_some_and(|own| same(own, path))
            })
            .map(|file| file.name.as_str())
    }

    /// The line and column of `offset` in a namespace's text.
    #[must_use]
    pub fn position(&self, name: &str, offset: usize) -> (usize, usize) {
        Source::new(self.text(name)).line_col(offset)
    }

    /// The exact text of a span in a namespace.
    #[must_use]
    pub fn slice(&self, name: &str, span: Span) -> Option<&str> {
        Source::new(self.text(name)).slice(span)
    }
}

/// Two paths name one file: equal, or equal once made canonical.
fn same(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

/// A pipe of the program: its node, the node on its left, its stage
/// (the right side of `||` and `&&`), its mark and where the mark is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipe {
    pub node: usize,
    pub left: usize,
    pub stage: usize,
    pub mark: &'static str,
    pub namespace: usize,
    pub line: usize,
    pub column: usize,
}

impl Pipe {
    /// Whether the stage runs on a left value on this side: `|` always;
    /// `|:` and `&&` on the good side; `|!` and `||` on the other.
    #[must_use]
    pub fn runs(&self, good: bool) -> bool {
        match self.mark {
            "|" => true,
            "|:" | "&&" => good,
            _ => !good,
        }
    }
}

/// Every pipe of the tables whose mark is found in its source, in node
/// order.
#[must_use]
pub fn pipes(tables: &Tables, files: &Files) -> Vec<Pipe> {
    let mut pipes = Vec::new();
    for (node, entry) in tables.nodes.iter().enumerate() {
        let (left, stage, mark) = match entry.kind {
            Kind::Pipe { left, stage } => (left, stage, "|"),
            Kind::Marked {
                good, left, stage, ..
            } => (left, stage, if good { "|:" } else { "|!" }),
            Kind::Or { left, right, .. } => (left, right, "||"),
            Kind::And { left, right, .. } => (left, right, "&&"),
            _ => continue,
        };
        let name = &tables.namespaces[entry.namespace];
        let text = files.text(name);
        let from = tables.nodes[left].span.end;
        let to = tables.nodes[stage].span.start;
        let Some(between) = text.get(from..to) else {
            continue;
        };
        let Some(at) = between.find(mark) else {
            continue;
        };
        let (line, column) = files.position(name, from + at);
        pipes.push(Pipe {
            node,
            left,
            stage,
            mark,
            namespace: entry.namespace,
            line,
            column,
        });
    }
    pipes
}

/// The pipe a node belongs to: the pipe itself, or the innermost pipe
/// whose stage holds the node's start, in the node's namespace.
#[must_use]
pub fn pipe_of<'a>(
    node: usize,
    tables: &Tables,
    pipes: &'a [Pipe],
) -> Option<&'a Pipe> {
    if let Some(pipe) = pipes.iter().find(|pipe| pipe.node == node) {
        return Some(pipe);
    }
    let entry = &tables.nodes[node];
    let at = entry.span.start;
    pipes
        .iter()
        .filter(|pipe| {
            let stage = tables.nodes[pipe.stage].span;
            pipe.namespace == entry.namespace
                && stage.start <= at
                && at < stage.end.max(stage.start + 1)
        })
        .min_by_key(|pipe| {
            let stage = tables.nodes[pipe.stage].span;
            stage.end - stage.start
        })
}
