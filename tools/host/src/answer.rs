//! A host call's answer: each declaration of `decl/` performed on the
//! real machine, its reply of the declared shape.
use crate::Host;
use fipe::{
    embed::{Reply, Request},
    ir::{self, Tables},
    value::{Prelude, Record, Value},
};
use std::{
    fs::{self, Metadata},
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

impl Host {
    /// Answer a request: a host call performed, or a file module's
    /// source read from the working directory (`<stem>.fipe`).
    ///
    /// A call's failure the declared result cannot carry is
    /// `Reply::Value(Err(reason))`, which Faults `Host` at the call.
    pub fn answer(&mut self, request: &Request, tables: &Tables) -> Reply {
        match request {
            Request::File { stem } => Reply::File(
                fs::read_to_string(
                    self.policy.cwd().join(format!("{stem}.fipe")),
                )
                .ok(),
            ),
            Request::Call {
                declaration,
                arguments,
                environment,
            } => {
                let base = self.base(environment, tables);
                Reply::Value(self.call(tables, *declaration, arguments, &base))
            }
        }
    }

    /// The directory paths resolve against: the innermost `with cwd`
    /// open at the call, or the working directory.
    fn base(&self, environment: &[(usize, Value)], tables: &Tables) -> PathBuf {
        environment
            .iter()
            .rev()
            .find_map(|(key, value)| match value {
                Value::Text(dir) if tables.keys[*key] == "cwd" => {
                    Some(self.policy.cwd().join(dir.as_str()))
                }
                _ => None,
            })
            .unwrap_or_else(|| self.policy.cwd().to_path_buf())
    }

    fn call(
        &mut self,
        tables: &Tables,
        declaration: usize,
        arguments: &[Value],
        base: &Path,
    ) -> Result<Value, String> {
        let prelude = Prelude::new(tables);
        let declared = &tables.declarations[declaration];
        let key = format!(
            "{}.{}",
            tables.namespaces[declared.namespace], declared.label
        );
        let text = |at: usize| text(arguments.get(at));
        let flag = |at: usize| {
            arguments
                .get(at)
                .is_some_and(|value| prelude.is_true(value))
        };
        let number = |at: usize| match arguments.get(at) {
            Some(Value::Int(n)) => usize::try_from(*n).unwrap_or(0),
            _ => 0,
        };
        let entries = Entries { tables };
        Ok(match key.as_str() {
            // A source or a sink is its path as written; an entry is the
            // file system's; a missing one Faults, as the conformance
            // host's does.
            "commands.read" => match read_target(tables, &declared.ty) {
                Some("Entry") => prelude.ok(entries
                    .at(base, &text(0))?
                    .ok_or_else(|| format!("no entry {}", text(0)))?),
                _ => prelude.ok(Value::text(&text(0))),
            },
            "commands.ls" => entries.list(
                base,
                &text(0),
                flag(2).then(|| number(3)),
                &text(4),
            )?,
            "fs.list" => {
                let written = Path::new(&text(0)).join(text(1));
                let written = written.to_string_lossy();
                entries.list(base, &written, None, "*")?
            }
            "commands.find" => entries
                .at(base, &text(0))?
                .map_or_else(|| prelude.nothing(), |entry| prelude.just(entry)),
            "commands.cat" => {
                let path = base.join(text(0));
                let bytes = fs::read(&path).map_err(|error| {
                    format!("cannot read {}: {error}", text(0))
                })?;
                let body = String::from_utf8(bytes)
                    .map_err(|_| format!("{} is not UTF-8 text", text(0)))?;
                Value::text(&body)
            }
            "commands.grep" => {
                let pattern = text(0);
                let lines: String = text(1)
                    .lines()
                    .filter(|line| line.contains(pattern.as_str()))
                    .flat_map(|line| [line, "\n"])
                    .collect();
                Value::text(&lines)
            }
            "commands.rm" => self.remove(tables, &prelude, base, &text(0)),
            "fs.remove" => {
                let written = Path::new(&text(0)).join(text(1));
                let written = written.to_string_lossy();
                self.remove(tables, &prelude, base, &written)
            }
            "commands.put" | "commands.append" => {
                let bytes = bytes(arguments.get(1))?;
                self.write(base, &text(0), &bytes, key == "commands.put")?;
                Value::Unit
            }
            "commands.root" => Value::text("/"),
            "commands.out" => Value::text("out"),
            "commands.Rd" => self.resource(number(0))?,
            "sys.lookup" => self.lookup(&base.join(text(0)).join(text(1)))?,
            "fs.size" => match arguments.first() {
                Some(Value::Record(record)) => record.fields[2].clone(),
                _ => Value::Int(0),
            },
            "cli.args" => Value::seq(
                self.args.iter().map(|arg| Value::text(arg)).collect(),
            ),
            "cli.env" => std::env::var(text(0)).map_or_else(
                |_| prelude.nothing(),
                |value| prelude.just(Value::text(&value)),
            ),
            "text.trim" => Value::text(text(0).trim()),
            "text.wrap" => Value::text(&wrap(&text(1), number(0))),
            other => return Err(format!("the std host has no {other}")),
        })
    }

    /// Remove a file, a link or an empty directory under the write
    /// roots: `Ok ()`, or `Err fs.Error` with the reason.
    fn remove(
        &self,
        tables: &Tables,
        prelude: &Prelude,
        base: &Path,
        written: &str,
    ) -> Value {
        let removed =
            self.policy
                .writable(base, written, false)
                .and_then(|target| {
                    target.remove().map_err(|error| {
                        format!("cannot remove {written}: {error}")
                    })
                });
        match removed {
            Ok(()) => prelude.ok(Value::Unit),
            Err(message) => {
                prelude.err(meaning(tables, "fs", "Error").map_or_else(
                    || Value::text(&message),
                    |meaning| {
                        Value::Record(Rc::new(Record {
                            meaning,
                            constructor: 0,
                            fields: vec![Value::text(&message)],
                        }))
                    },
                ))
            }
        }
    }

    /// The resource a `sys.lookup` numbered: its path.
    fn resource(&self, number: usize) -> Result<Value, String> {
        let path = self
            .resources
            .get(number)
            .ok_or_else(|| format!("no resource numbered {number}"))?;
        Ok(Value::text(&path.to_string_lossy()))
    }

    /// Number an existing path as a resource.
    fn lookup(&mut self, path: &Path) -> Result<Value, String> {
        let path = fs::canonicalize(path).map_err(|error| {
            format!("no resource {}: {error}", path.display())
        })?;
        self.resources.push(path);
        Ok(Value::Int(
            i64::try_from(self.resources.len() - 1).unwrap_or(i64::MAX),
        ))
    }

    /// Write bytes to a sink: `out` is the success output, any other a
    /// file under the write roots, replaced or extended.
    fn write(
        &mut self,
        base: &Path,
        sink: &str,
        bytes: &[u8],
        replace: bool,
    ) -> Result<(), String> {
        if sink == "out" {
            self.sink.success.extend(bytes);
            return Ok(());
        }
        let target = self.policy.writable(base, sink, true)?;
        target
            .write(bytes, replace)
            .map_err(|error| format!("cannot write {sink}: {error}"))
    }
}

/// The text of a source, a sink, a resource or an entry's path.
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Text(text)) => text.as_str().to_owned(),
        Some(Value::Record(record)) => match record.fields.get(1) {
            Some(Value::Text(path)) => path.as_str().to_owned(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// The bytes of a `Vec<U8>` value.
fn bytes(value: Option<&Value>) -> Result<Vec<u8>, String> {
    let Some(Value::Seq(parts)) = value else {
        return Err(String::from("a write takes bytes"));
    };
    parts
        .iter()
        .map(|part| match part {
            Value::Int(byte) => u8::try_from(*byte).ok(),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(|| String::from("a write takes bytes"))
}

/// A meaning by its namespace and label.
fn meaning(tables: &Tables, namespace: &str, label: &str) -> Option<usize> {
    tables.meanings.iter().position(|meaning| {
        meaning.label == label
            && tables.namespaces[meaning.namespace] == namespace
    })
}

/// The label of the `Ok` side of a `String -> Result<T, _>` reader.
fn read_target<'t>(tables: &'t Tables, ty: &ir::Type) -> Option<&'t str> {
    let ir::Type::Function { result, .. } = ty else {
        return None;
    };
    let ir::Type::Meaning { arguments, .. } = result.as_ref() else {
        return None;
    };
    let Some(ir::Type::Meaning { index, .. }) = arguments.first() else {
        return None;
    };
    Some(tables.meanings[*index].label.as_str())
}

/// Entries of the command namespace's `Entry` type.
struct Entries<'a> {
    tables: &'a Tables,
}

impl Entries<'_> {
    /// The entry at a path as written, when it exists.
    fn at(&self, base: &Path, written: &str) -> Result<Option<Value>, String> {
        fs::metadata(base.join(written))
            .ok()
            .map(|meta| self.entry(written, &meta))
            .transpose()
    }

    /// The entry at a path as written, with its metadata.
    fn entry(&self, written: &str, meta: &Metadata) -> Result<Value, String> {
        let meaning = meaning(self.tables, "commands", "Entry")
            .ok_or_else(|| String::from("the unit names no Entry"))?;
        let trimmed = written.trim_end_matches('/');
        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let age = meta
            .modified()
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .map_or(0, |age| age.as_secs());
        let int = |n: u64| Value::Int(i64::try_from(n).unwrap_or(i64::MAX));
        Ok(Value::Record(Rc::new(Record {
            meaning,
            constructor: 0,
            fields: vec![
                Value::text(name),
                Value::text(written),
                int(meta.len()),
                int(age),
            ],
        })))
    }

    /// The entries of a directory as written, by path: a file lists
    /// itself; `depth`, when recursive, is how many levels to visit.
    fn list(
        &self,
        base: &Path,
        written: &str,
        depth: Option<usize>,
        pattern: &str,
    ) -> Result<Value, String> {
        let path = base.join(written);
        let meta = fs::metadata(&path)
            .map_err(|error| format!("cannot list {written}: {error}"))?;
        let mut found = Vec::new();
        if meta.is_dir() {
            self.visit(
                &path,
                written,
                depth.unwrap_or(1).max(1),
                pattern,
                &mut found,
            )?;
        } else {
            found.push((written.to_owned(), self.entry(written, &meta)?));
        }
        found.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(Value::seq(
            found.into_iter().map(|(_, entry)| entry).collect(),
        ))
    }

    fn visit(
        &self,
        dir: &Path,
        written: &str,
        depth: usize,
        pattern: &str,
        found: &mut Vec<(String, Value)>,
    ) -> Result<(), String> {
        let listing = fs::read_dir(dir)
            .map_err(|error| format!("cannot list {written}: {error}"))?;
        for item in listing {
            let item = item
                .map_err(|error| format!("cannot list {written}: {error}"))?;
            let name = item.file_name().to_string_lossy().into_owned();
            let path = if written.ends_with('/') {
                format!("{written}{name}")
            } else {
                format!("{written}/{name}")
            };
            let full = item.path();
            let meta = fs::metadata(&full)
                .or_else(|_| fs::symlink_metadata(&full))
                .map_err(|error| format!("cannot list {path}: {error}"))?;
            if matches(pattern.as_bytes(), name.as_bytes()) {
                found.push((path.clone(), self.entry(&path, &meta)?));
            }
            // A link to a directory is listed, never entered.
            let real_dir = item.file_type().is_ok_and(|kind| kind.is_dir());
            if depth > 1 && real_dir {
                self.visit(&full, &path, depth - 1, pattern, found)?;
            }
        }
        Ok(())
    }
}

/// Whether a name matches a pattern of `*` (any run) and `?` (any one
/// byte).
fn matches(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            matches(&pattern[1..], name)
                || (!name.is_empty() && matches(pattern, &name[1..]))
        }
        (Some(b'?'), Some(_)) => matches(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => matches(&pattern[1..], &name[1..]),
        _ => false,
    }
}

/// Each line of a text wrapped at word boundaries to `width`; a word
/// longer than the width stands on a line of its own.
fn wrap(text: &str, width: usize) -> String {
    text.split('\n')
        .map(|line| {
            let mut out = String::new();
            let mut column = 0;
            for word in line.split_whitespace() {
                let len = word.chars().count();
                if column > 0 && column + 1 + len > width {
                    out.push('\n');
                    column = 0;
                } else if column > 0 {
                    out.push(' ');
                    column += 1;
                }
                out.push_str(word);
                column += len;
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{matches, wrap};

    #[test]
    fn a_pattern_matches_runs_and_single_bytes() {
        assert!(matches(b"*", b""));
        assert!(matches(b"*.conf", b"fipe.conf"));
        assert!(!matches(b"*.conf", b"fipe.confx"));
        assert!(matches(b"a?c", b"abc"));
        assert!(!matches(b"a?c", b"ac"));
    }

    #[test]
    fn wrapping_breaks_at_words_and_keeps_line_feeds() {
        assert_eq!(wrap("aa bb cc", 5), "aa bb\ncc");
        assert_eq!(wrap("a\nb c\n", 72), "a\nb c\n");
        assert_eq!(wrap("toolongword x", 4), "toolongword\nx");
    }
}
