//! Module loading, as a host prepares it: the host's declaration files
//! from one directory, and the file modules a document's `use` lines
//! name, each `<stem>.fipe` beside the document.

use std::fs;
use std::path::{Path, PathBuf};

use fipe::ast::{Entry, ItemKind, Module};
use fipe::decl::Declarations;
use fipe::lex::lex;
use fipe::parse::parse_partial;

/// The host's declarations and the sources they came from.
#[derive(Debug, Default)]
pub struct Host {
    pub declarations: Declarations,
    /// Each declaration file: its namespace (`commands` or a module
    /// name), path and text.
    pub sources: Vec<(String, PathBuf, String)>,
    /// Declaration files that did not load, as words.
    pub errors: Vec<String>,
}

impl Host {
    /// The path and text of the declaration file of `namespace`.
    #[must_use]
    pub fn source(&self, namespace: &str) -> Option<(&Path, &str)> {
        self.sources
            .iter()
            .find(|(name, _, _)| name == namespace)
            .map(|(_, path, text)| (path.as_path(), text.as_str()))
    }
}

/// Load the declaration files of `dir`: `commands.fipe` is the command
/// namespace, every other `<name>.fipe` the host module `name`. With no
/// directory, the prelude alone.
#[must_use]
pub fn host(dir: Option<&Path>) -> Host {
    let mut host = Host::default();
    let Some(dir) = dir else { return host };
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "fipe")
                .then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    // The command namespace first, as a host declares it.
    names.sort_by_key(|name| name != "commands");
    for name in names {
        let path = dir.join(format!("{name}.fipe"));
        let Ok(text) = fs::read_to_string(&path) else {
            host.errors.push(format!("{}: cannot read", path.display()));
            continue;
        };
        let loaded = if name == "commands" {
            host.declarations.commands(&text)
        } else {
            host.declarations.module(&name, &text)
        };
        if let Err(errors) = loaded {
            host.errors.push(format!(
                "{}: {} declaration error(s)",
                path.display(),
                errors.len()
            ));
        }
        host.sources.push((name, path, text));
    }
    host
}

/// A file module a document uses.
#[derive(Debug)]
pub struct File {
    pub stem: String,
    pub path: PathBuf,
    pub text: String,
    pub module: Module,
}

/// The stems the `use` lines of `module` name, with each name's span.
pub fn uses(module: &Module) -> impl Iterator<Item = (&str, (usize, usize))> {
    module.entries.iter().flat_map(|entry| {
        let names = match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Use { names, .. } => names.as_slice(),
                _ => &[],
            },
            Entry::Line(_) => &[],
        };
        names.iter().filter_map(|name| {
            let stem = name.segments.first()?;
            Some((stem.as_str(), (name.span.start, name.span.end)))
        })
    })
}

/// The file modules `module` reaches through `use`, from `dir`, each
/// once.
///
/// `open` gives an open document's text in place of the disk's. A host
/// module's name is not a file, and a file that cannot be read is left
/// out, so the check reports its `use`.
pub fn files(
    dir: &Path,
    module: &Module,
    declarations: &Declarations,
    open: &dyn Fn(&Path) -> Option<String>,
) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    let mut pending: Vec<String> =
        uses(module).map(|(stem, _)| String::from(stem)).collect();
    while let Some(stem) = pending.pop() {
        if declarations.get_module(&stem).is_some()
            || files.iter().any(|file| file.stem == stem)
        {
            continue;
        }
        let path = dir.join(format!("{stem}.fipe"));
        let Some(text) = open(&path).or_else(|| fs::read_to_string(&path).ok())
        else {
            continue;
        };
        let module = parse_partial(lex(&text));
        pending.extend(uses(&module).map(|(stem, _)| String::from(stem)));
        files.push(File {
            stem,
            path,
            text,
            module,
        });
    }
    files
}
