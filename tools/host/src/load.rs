//! A script and the file modules it `use`s, read, checked against the
//! std host's declarations and compiled.
use fipe::{
    ast::{Entry, ItemKind, Module},
    check,
    lex::lex,
    mir::Program,
    parse::parse,
    token::{Source, Span},
};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// A checked, compiled script.
#[derive(Debug)]
pub struct Script {
    pub program: Program,
    /// The source of each namespace a Fault may name: `main`, then each
    /// file module by its stem.
    pub sources: Vec<(String, String)>,
}

/// A diagnostic located in the file it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.message
        )
    }
}

/// Why a script cannot run.
#[derive(Debug)]
pub enum LoadError {
    /// A file that cannot be read.
    Read { path: PathBuf, error: io::Error },
    /// The script or a module it uses does not check.
    Check(Vec<Diagnostic>),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => {
                write!(f, "{}: cannot read: {error}", path.display())
            }
            Self::Check(diagnostics) => {
                for (at, diagnostic) in diagnostics.iter().enumerate() {
                    if at > 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{diagnostic}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// A file read and parsed: its path, text and module.
struct File {
    stem: String,
    path: PathBuf,
    text: String,
}

/// Load a script: read it and the file modules its `use` lines reach
/// beside it (`<stem>.fipe`, a host module's name excepted), check the
/// whole unit against the std host's declarations, and compile it.
///
/// # Errors
/// [`LoadError`] when a file cannot be read or the unit does not
/// check.
pub fn load(path: &Path) -> Result<Script, LoadError> {
    let read = |path: &Path| {
        fs::read_to_string(path).map_err(|error| LoadError::Read {
            path: path.to_path_buf(),
            error,
        })
    };
    let text = read(path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let declarations = crate::declarations();
    let main = parsed(path, &text)?;
    let mut files: Vec<File> = Vec::new();
    let mut modules: Vec<(String, Module)> = Vec::new();
    let mut pending: Vec<String> = uses(&main).collect();
    while let Some(stem) = pending.pop() {
        let file = dir.join(format!("{stem}.fipe"));
        if declarations.get_module(&stem).is_some()
            || files.iter().any(|loaded| loaded.stem == stem)
            || !file.exists()
        {
            continue;
        }
        let text = read(&file)?;
        let module = parsed(&file, &text)?;
        pending.extend(uses(&module));
        modules.push((stem.clone(), module));
        files.push(File {
            stem,
            path: file,
            text,
        });
    }
    let unit =
        check::check(&main, &declarations, &modules, check::Options::default())
            .map_err(|errors| {
                LoadError::Check(
                    errors
                        .iter()
                        .map(|error| diagnostic(error, path, &text, &files))
                        .collect(),
                )
            })?;
    let mut sources = vec![(String::from("main"), text)];
    sources.extend(files.into_iter().map(|file| (file.stem, file.text)));
    Ok(Script {
        program: fipe::compile::compile(&unit),
        sources,
    })
}

/// A file parsed whole, or its syntax errors.
fn parsed(path: &Path, text: &str) -> Result<Module, LoadError> {
    parse(lex(text)).map_err(|errors| {
        LoadError::Check(
            errors
                .iter()
                .map(|error| {
                    located(path, text, error.span, error.reason.to_string())
                })
                .collect(),
        )
    })
}

/// The stems a module's `use` lines name.
fn uses(module: &Module) -> impl Iterator<Item = String> + '_ {
    module.entries.iter().flat_map(|entry| {
        let names = match entry {
            Entry::Item(item) => match &item.kind {
                ItemKind::Use { names, .. } => names.as_slice(),
                _ => &[],
            },
            Entry::Line(_) => &[],
        };
        names
            .iter()
            .filter_map(|name| name.segments.first().cloned())
    })
}

fn located(path: &Path, text: &str, span: Span, message: String) -> Diagnostic {
    let (line, column) = Source::new(text).line_col(span.start);
    Diagnostic {
        path: path.to_path_buf(),
        line,
        column,
        message,
    }
}

/// A checker diagnostic in the file it names: the script, or a file
/// module by its stem.
fn diagnostic(
    error: &check::Error,
    path: &Path,
    text: &str,
    files: &[File],
) -> Diagnostic {
    let in_file = |stem: &str, span: Span, message: String| {
        files.iter().find(|file| file.stem == stem).map_or_else(
            || located(path, text, span, message.clone()),
            |file| located(&file.path, &file.text, span, message.clone()),
        )
    };
    match error {
        check::Error::Syntax(error) => {
            located(path, text, error.span, error.reason.to_string())
        }
        check::Error::Resolve(error) => {
            located(path, text, error.span, error.reason.to_string())
        }
        check::Error::Type(error) => {
            located(path, text, error.span, error.reason.to_string())
        }
        check::Error::File { stem, error } => {
            in_file(stem, error.span, error.reason.to_string())
        }
        check::Error::FileResolve { stem, error } => {
            in_file(stem, error.span, error.reason.to_string())
        }
        check::Error::FileType { stem, error } => {
            in_file(stem, error.span, error.reason.to_string())
        }
        check::Error::Compile { span, what, reason } => {
            located(path, text, *span, what.describe(reason))
        }
    }
}
