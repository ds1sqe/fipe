# fipe-host: the std host

The std host runs a Fipe script against the real machine: its files,
its arguments and its environment. `fipe run`, `fipe repl`, a live DAP
launch and a native executable answer through it, so one script gives
the same bytes and exit code under each. This README is the host's
documented contract ([host](../../Documentation/spec/09-host.md)
§Input, output and observation): its declarations, sink policy, input,
exit codes and write roots. The conformance host
(`tests/fixtures/host`, `tests/host/replies.rs`) stays the crate's test
host; the std host keeps its names and value shapes and gives real
answers.

## Declarations

`decl/` holds one file per module, `commands.fipe` for the command
namespace; `fipe_host::sources()` gives them, `commands` first, and
`fipe_host::declarations()` loads them. A path below resolves against
the working directory, or against the innermost `with cwd D` open at the
call (`D` itself resolved against the working directory).

| Name | Shape | Answer |
| --- | --- | --- |
| `ls` | `(path: Src = ./) -> (long: Bool = False) -> (recursive: Bool = False) -> (depth: U64 = 1) -> (name: String = "*") -> Vec<Entry>` | the entries of directory `path` (a file lists itself), by path; `-r` descends `depth` levels; `name` is a pattern of `*` and `?` on the entry name |
| `cat` | `Entry -> String` | the file's text; not UTF-8 or unreadable Faults |
| `find` | `Src -> Maybe<Entry>` | `Just` the entry when the path exists, else `Nothing` |
| `grep` | `(pattern: String) -> String -> String` | the lines holding `pattern`, each with its line feed |
| `rm` | `Src -> Result<(), fs.Error>` | removes a file, a link or an empty directory; a refusal or failure is `Err` |
| `put` | `(to: Sink) -> Vec<U8> -> ()` | `out` writes the success output; a path is created or replaced |
| `append` | `(to: Sink) -> Vec<U8> -> ()` | as `put`, extending the file |
| `out` | `Sink` | the success output |
| `root` | `Rd<sys>` | the resource `/` |
| `Rd` | `U64 -> Rd<api>` | the resource a `sys.lookup` numbered |
| `Src`, `Sink`, `Entry` read | `String -> Result<Self, ParseError>` | a source or sink is its path as written; an entry is the path's, and a missing path Faults, as the conformance host's does |
| `fs.list` | `(dir: Rd<fs>) -> (path: Src) -> (long: Bool = False) -> Vec<Entry>` | `ls` of `path` under `dir`, the entries' paths under `dir` |
| `fs.remove` | `(dir: Rd<fs>) -> (path: Src) -> Result<(), Error>` | `rm` of `path` under `dir` |
| `fs.size` | `Entry -> U64` | the entry's size |
| `sys.lookup` | `(from: Rd<sys>) -> (path: Src) -> U64` | numbers the existing path `path` under `from`; a missing one Faults |
| `text.wrap` | `(width: U64 = 72) -> String -> String` | each line wrapped at word boundaries to `width` |
| `text.trim` | `String -> String` | without surrounding white space |
| `cli.args` | `Vec<String>` | the arguments after the script, in order |
| `cli.env` | `String -> Maybe<String>` | the process environment variable, `Nothing` when unset or not UTF-8 |

`fs.Error` and `cli.Error` are records of one `message`. An `Entry` is
`name` (the last path component), `path` (the listed path as written,
then `/` and the name), `size` in bytes and `age` in whole seconds since
its last modification; every entry carries all four, so `long` changes
nothing. A resource value is its path. A failure that the declared
result cannot carry, such as an unreadable directory, Faults `Host` at
the call with its reason. Nothing else is declared until a script needs
it.

## Runners

`fipe run`, `fipe repl` and `fipe build` ([`tools/fipe`](../fipe/README.md))
run through `Host::run`, `Host::answer` and a built executable's
in-process host ([`tools/native`](../native/README.md) §Running); a
runner streams the output as the run makes it (`Host::streaming`),
after each host call and at the end. `fipe repl` shows each line's value
as the REPL does and performs its calls here; the sink policy below is
`run`'s and an executable's.

## Write roots

Reads go anywhere the process may read. Writes, appends and removals
stay under the write roots: the canonicalized working directory and
each directory the runner allows (`--allow-write <dir>`). A destination
is inside when its parent directory, with every link and `..` resolved,
is a write root or below one, and its name is a plain name; a `put` or
`append` to an existing link must also resolve inside, while `rm` of a
link removes the link and never its target. A write root itself is
never removed. A destination outside, as `../x`, an absolute path
elsewhere or a link leaving a root, is refused: `Err fs.Error` where the
declaration returns a `Result` (`rm`, `fs.remove`), a `Host` Fault
otherwise (`put`, `append`). The host spawns no process.

## Sink policy

Each main expression line's value, when its line ends, goes to an output
with a line feed: a `String` as its text, any other value as the
language spells it, a host module's `Error` as its message; `Ok v` and
`Just v` write `v`; `()` and `Nothing` write nothing; `Err e` writes `e`
to the error output. `put out` and
`append out` write their bytes to the success output, in order with the
lines. A Fault writes `fault in <namespace> at <line>:<column>:
<reason>` and a line feed to the error output, `main` naming the script.
A script that does not check writes each diagnostic as `<file>:<line>:
<column>: <reason>` to the error output and does not run.

## Input

Standard input is host input when the script's last line is a function
of one open spot of type `String` (the input as UTF-8 text) or
`Vec<U8>` (its bytes; any spot that lowers to a vector of integers, as
`U8`'s limit is an entry, not a type). A last line that is not a function takes no
input. Any other open spot, or several, is refused before any effect,
as is input that is not UTF-8 for a `String` spot; the refusal is
written as a `Host` Fault at the last line.

## Exit codes

| Code | When |
| --- | --- |
| 0 | the run ended with a value that is not an `Err` |
| 1 | the run ended with `Err e`, or Faulted |
| 2 | the script could not run: it does not check, or its input is refused |

## Development

```sh
cargo fmt --check
cargo test --locked --offline
cargo test --locked --offline --release
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
```
