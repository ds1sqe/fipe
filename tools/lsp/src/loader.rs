//! Module loading, as a host prepares it: the host's declaration files
//! from one directory, and the file modules a document's `use` lines
//! name, each `<stem>.fipe` beside the document.

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use fipe::ast::{Entry, ItemKind, Module};
use fipe::decl::Declarations;
use fipe::lex::lex;
use fipe::parse::parse_partial;

/// The host's declarations and the sources they came from.
#[derive(Clone, Debug, Default)]
pub struct Host {
    pub declarations: Declarations,
    /// Each declaration file: its namespace (`commands` or a module
    /// name), path and text.
    pub sources: Vec<(String, PathBuf, String)>,
    /// Declaration files that did not load, as words.
    pub errors: Vec<String>,
}

/// One explicitly selected offline declaration source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub module: String,
    pub path: PathBuf,
}

/// A selected interface failed without changing the admitted host.
#[derive(Debug)]
pub struct InterfaceError {
    pub input: Interface,
    pub reason: InterfaceProblem,
}

#[derive(Debug)]
pub enum InterfaceProblem {
    Invalid(&'static str),
    Duplicate {
        first: PathBuf,
    },
    Read(io::Error),
    Declarations {
        text: String,
        errors: Vec<fipe::decl::Error>,
    },
}

impl fmt::Display for InterfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "interface {} ({}): ",
            self.input.module,
            self.input.path.display()
        )?;
        match &self.reason {
            InterfaceProblem::Invalid(reason) => f.write_str(reason),
            InterfaceProblem::Duplicate { first } => write!(
                f,
                "duplicate module; first selected from {}",
                first.display()
            ),
            InterfaceProblem::Read(error) => write!(f, "cannot read: {error}"),
            InterfaceProblem::Declarations { text, errors } => {
                for (index, error) in errors.iter().enumerate() {
                    if index != 0 {
                        f.write_str("; ")?;
                    }
                    let at = crate::position::position(
                        text,
                        error.span.start,
                        crate::position::Encoding::Utf8,
                    );
                    write!(
                        f,
                        "{}:{}: {}",
                        u64::from(at.line) + 1,
                        u64::from(at.character) + 1,
                        error.reason
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for InterfaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.reason {
            InterfaceProblem::Read(error) => Some(error),
            _ => None,
        }
    }
}

impl Host {
    /// Add explicit `.fipi` inputs as one immutable snapshot, in dependency order.
    ///
    /// # Errors
    /// Invalid identities/paths, duplicate selected modules, read or declaration errors.
    /// The original host remains unchanged on every failure.
    pub fn with_interfaces(
        &self,
        inputs: &[Interface],
    ) -> Result<Self, InterfaceError> {
        self.with_interfaces_open(inputs, &|_| None)
    }

    /// Stage interfaces using open text before disk, preserving selected source paths.
    ///
    /// # Errors
    /// The same identity, duplicate, read and declaration failures as `with_interfaces`.
    pub fn with_interfaces_open(
        &self,
        inputs: &[Interface],
        open: &dyn Fn(&Path) -> Option<String>,
    ) -> Result<Self, InterfaceError> {
        for (index, input) in inputs.iter().enumerate() {
            let fail = |reason| InterfaceError {
                input: input.clone(),
                reason,
            };
            if !input.path.is_absolute()
                || input.path.extension().is_none_or(|ext| ext != "fipi")
            {
                return Err(fail(InterfaceProblem::Invalid(
                    "expected an absolute .fipi filesystem path",
                )));
            }
            if Declarations::new().module(&input.module, "").is_err() {
                return Err(fail(InterfaceProblem::Invalid(
                    "invalid or reserved module identity",
                )));
            }
            let first = self
                .sources
                .iter()
                .map(|(name, path, _)| (name, path))
                .chain(
                    inputs[..index]
                        .iter()
                        .map(|prior| (&prior.module, &prior.path)),
                )
                .find(|(name, _)| **name == input.module);
            if let Some((_, first)) = first {
                return Err(fail(InterfaceProblem::Duplicate {
                    first: first.clone(),
                }));
            }
        }
        let mut candidate = self.clone();
        for input in inputs {
            let text = open(&input.path)
                .map_or_else(|| fs::read_to_string(&input.path), Ok)
                .map_err(|error| InterfaceError {
                    input: input.clone(),
                    reason: InterfaceProblem::Read(error),
                })?;
            candidate
                .declarations
                .module(&input.module, &text)
                .map_err(|errors| InterfaceError {
                    input: input.clone(),
                    reason: InterfaceProblem::Declarations {
                        text: text.clone(),
                        errors,
                    },
                })?;
            candidate.sources.push((
                input.module.clone(),
                input.path.clone(),
                text,
            ));
        }
        Ok(candidate)
    }
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
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            host.errors
                .push(format!("{}: cannot enumerate: {error}", dir.display()));
            return host;
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                host.errors.push(format!(
                    "{}: cannot read directory entry: {error}",
                    dir.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "fipe") {
            continue;
        }
        if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
            names.push(name.to_owned());
        } else {
            host.errors
                .push(format!("{}: invalid module filename", path.display()));
        }
    }
    names.sort();
    // The command namespace first, as a host declares it.
    names.sort_by_key(|name| name != "commands");
    for name in names {
        let path = dir.join(format!("{name}.fipe"));
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                host.errors
                    .push(format!("{}: cannot read: {error}", path.display()));
                continue;
            }
        };
        let loaded = if name == "commands" {
            host.declarations.commands(&text)
        } else {
            host.declarations.module(&name, &text)
        };
        if let Err(errors) = loaded {
            for error in errors {
                let at = crate::position::position(
                    &text,
                    error.span.start,
                    crate::position::Encoding::Utf8,
                );
                host.errors.push(format!(
                    "{}:{}:{}: {}",
                    path.display(),
                    u64::from(at.line) + 1,
                    u64::from(at.character) + 1,
                    error.reason
                ));
            }
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
