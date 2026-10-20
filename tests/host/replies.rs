//! The conformance host's replies: a fixed file system and fixed
//! answers, keyed by each call's `namespace.label` and its arguments,
//! and the sink policy that heads `tests/fixtures/host/commands.fipe`:
//! writes to `out` are the success output, writes to any other sink
//! are in-memory files a later `cat` reads. Replies are Rust because
//! they are typed values of the host's declared types (records of
//! `Entry`, outcomes); a text format would need a value reader of its
//! own.
use fipe::{
    embed::Request,
    ir::{self, Tables},
    value::{Prelude, Record, Value},
};
use std::rc::Rc;

/// A listed entry: its name, size and age in seconds.
type Listed = (&'static str, i64, i64);

/// The file system: each directory's entries, and each file's text.
const DIRECTORIES: &[(&str, &[Listed])] = &[
    (
        "/var/log",
        &[
            ("syslog", 2_400_000, 172_800),
            ("auth.log", 300, 864_000),
            ("kern.log", 1_500_000, 3_600),
        ],
    ),
    ("/tmp", &[("a.txt", 10, 691_200), ("b.txt", 20, 3_600)]),
    ("/etc", &[("hosts", 120, 86_400), ("fipe.conf", 40, 7_200)]),
    ("./", &[("notes.md", 64, 600), ("names", 16, 600)]),
];

const FILES: &[(&str, &str)] = &[
    ("./notes.md", "# Notes\n\nbuy milk\n\ncall home\n"),
    ("./config.toml", "port = 8080\n"),
    ("./book.txt", "Call me Ishmael.\nSome years ago.\n"),
    ("./names", "zoe\nadam\nmia\n"),
];

fn meaning(tables: &Tables, label: &str) -> usize {
    tables
        .meanings
        .iter()
        .position(|meaning| meaning.label == label)
        .unwrap_or_else(|| panic!("no meaning {label}"))
}

fn entry(tables: &Tables, path: &str, size: i64, age: i64) -> Value {
    let name = path.rsplit('/').next().unwrap_or(path);
    Value::Record(Rc::new(Record {
        meaning: meaning(tables, "Entry"),
        constructor: 0,
        fields: vec![
            Value::text(name),
            Value::text(path),
            Value::Int(size),
            Value::Int(age),
        ],
    }))
}

/// The entry at a path, when the file system has it: a listed entry,
/// or a file, sized by its text.
fn lookup(tables: &Tables, path: &str, written: &Files) -> Option<Value> {
    let size = |len: usize| i64::try_from(len).unwrap();
    let file = written
        .iter()
        .find(|(name, _)| name == path)
        .map(|(name, bytes)| entry(tables, name, size(bytes.len()), 0));
    let file = file.or_else(|| {
        FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(name, text)| entry(tables, name, size(text.len()), 60))
    });
    file.or_else(|| listed(tables, path))
}

/// The files the run wrote, by path.
type Files = Vec<(String, Vec<u8>)>;

/// The bytes of a `Vec<U8>` value.
fn bytes(value: Option<&Value>) -> Vec<u8> {
    let Some(Value::Seq(parts)) = value else {
        panic!("a write takes bytes, not {value:?}")
    };
    parts
        .iter()
        .map(|part| match part {
            Value::Int(byte) => u8::try_from(*byte).unwrap(),
            other => panic!("a byte, not {other:?}"),
        })
        .collect()
}

fn listed(tables: &Tables, path: &str) -> Option<Value> {
    DIRECTORIES.iter().find_map(|(dir, entries)| {
        entries.iter().find_map(|(name, size, age)| {
            let full = if dir.ends_with('/') {
                format!("{dir}{name}")
            } else {
                format!("{dir}/{name}")
            };
            (full == path).then(|| entry(tables, &full, *size, *age))
        })
    })
}

fn listing(tables: &Tables, dir: &str) -> Value {
    let entries = DIRECTORIES
        .iter()
        .find(|(name, _)| *name == dir)
        .map_or(&[][..], |(_, entries)| *entries);
    Value::seq(
        entries
            .iter()
            .map(|(name, size, age)| {
                let path = if dir.ends_with('/') {
                    format!("{dir}{name}")
                } else {
                    format!("{dir}/{name}")
                };
                entry(tables, &path, *size, *age)
            })
            .collect(),
    )
}

/// The label of the `Ok` side of a `String -> Result<T, _>` reader.
fn read_target(tables: &Tables, ty: &ir::Type) -> String {
    if let ir::Type::Function { result, .. } = ty {
        if let ir::Type::Meaning { arguments, .. } = result.as_ref() {
            if let Some(ir::Type::Meaning { index, .. }) = arguments.first() {
                return tables.meanings[*index].label.clone();
            }
        }
    }
    String::from("?")
}

/// The conformance host's state over one run: the files it wrote and
/// the success output.
#[derive(Default)]
pub struct Host {
    written: Files,
    pub success: Vec<u8>,
}

impl Host {
    /// Apply a write to its sink: `out` is the success output, any
    /// other sink a file `put` replaces and `append` extends.
    fn write(&mut self, sink: &str, bytes: Vec<u8>, replace: bool) {
        if sink == "out" {
            self.success.extend(bytes);
            return;
        }
        if let Some((_, file)) =
            self.written.iter_mut().find(|(name, _)| name == sink)
        {
            if replace {
                file.clear();
            }
            file.extend(bytes);
        } else {
            self.written.push((String::from(sink), bytes));
        }
    }

    /// The reply to a host call.
    pub fn answer(
        &mut self,
        tables: &Tables,
        request: &Request,
    ) -> Result<Value, String> {
        let Request::Call {
            declaration,
            arguments,
            ..
        } = request
        else {
            panic!("a run asks only for host calls")
        };
        let prelude = Prelude::new(tables);
        let declared = &tables.declarations[*declaration];
        let key = format!(
            "{}.{}",
            tables.namespaces[declared.namespace], declared.label
        );
        let text = |at: usize| match arguments.get(at) {
            Some(Value::Text(text)) => text.as_str().to_owned(),
            Some(Value::Record(record)) => match record.fields.get(1) {
                Some(Value::Text(path)) => path.as_str().to_owned(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        Ok(match key.as_str() {
            // `Src`, `Sink` and `Entry` read from a word: a source or a sink
            // is its path; an entry is the file system's, or absent.
            "commands.read" => match read_target(tables, &declared.ty).as_str()
            {
                "Entry" => match lookup(tables, &text(0), &self.written) {
                    Some(entry) => prelude.ok(entry),
                    None => return Err(format!("no entry {}", text(0))),
                },
                _ => prelude.ok(Value::text(&text(0))),
            },
            "commands.ls" => listing(tables, &text(0)),
            "fs.list" => listing(tables, &text(1)),
            "commands.find" => lookup(tables, &text(0), &self.written)
                .map_or_else(|| prelude.nothing(), |entry| prelude.just(entry)),
            "commands.cat" => self
                .written
                .iter()
                .find(|(path, _)| *path == text(0))
                .map(|(_, bytes)| Value::text(&String::from_utf8_lossy(bytes)))
                .or_else(|| {
                    FILES
                        .iter()
                        .find(|(path, _)| *path == text(0))
                        .map(|(_, body)| Value::text(body))
                })
                .ok_or_else(|| format!("cannot read {}", text(0)))?,
            "commands.grep" => Value::text(""),
            "commands.rm" | "fs.remove" => prelude.ok(Value::Unit),
            "commands.put" | "commands.append" => {
                let replace = key == "commands.put";
                self.write(&text(0), bytes(arguments.get(1)), replace);
                Value::Unit
            }
            "commands.root" => Value::text("/"),
            "commands.out" => Value::text("out"),
            "commands.Rd" => Value::text("rd"),
            "sys.lookup" => Value::Int(7),
            "fs.size" => match arguments.first() {
                Some(Value::Record(record)) => record.fields[2].clone(),
                _ => Value::Int(0),
            },
            "cli.port" => Value::Int(8080),
            "cli.seed" => Value::Int(42),
            "cli.primes" => Value::seq(vec![Value::Int(2), Value::Int(3)]),
            "text.trim" => Value::text(text(0).trim()),
            "text.wrap" => Value::text(&text(1)),
            other => {
                return Err(format!("the conformance host has no {other}"))
            }
        })
    }
}
