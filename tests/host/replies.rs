//! The conformance host's replies: a fixed file system and fixed
//! answers, keyed by each call's `namespace.label` and its arguments.
//! Replies are Rust because they are typed values of the host's
//! declared types (records of `Entry`, outcomes); a text format would
//! need a value reader of its own.
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
fn lookup(tables: &Tables, path: &str) -> Option<Value> {
    let file = FILES.iter().find(|(name, _)| *name == path);
    let file = file.map(|(name, text)| {
        entry(tables, name, i64::try_from(text.len()).unwrap(), 60)
    });
    file.or_else(|| listed(tables, path))
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

/// The reply to a host call.
pub fn answer(tables: &Tables, request: &Request) -> Result<Value, String> {
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
        "commands.read" => match read_target(tables, &declared.ty).as_str() {
            "Entry" => match lookup(tables, &text(0)) {
                Some(entry) => prelude.ok(entry),
                None => return Err(format!("no entry {}", text(0))),
            },
            _ => prelude.ok(Value::text(&text(0))),
        },
        "commands.ls" => listing(tables, &text(0)),
        "fs.list" => listing(tables, &text(1)),
        "commands.find" => lookup(tables, &text(0))
            .map_or_else(|| prelude.nothing(), |entry| prelude.just(entry)),
        "commands.cat" => FILES
            .iter()
            .find(|(path, _)| *path == text(0))
            .map(|(_, body)| Value::text(body))
            .ok_or_else(|| format!("cannot read {}", text(0)))?,
        "commands.grep" => Value::text(""),
        "commands.rm" | "fs.remove" => prelude.ok(Value::Unit),
        "commands.put" | "commands.append" => Value::Unit,
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
        other => return Err(format!("the conformance host has no {other}")),
    })
}
