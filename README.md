# Fipe

**Fipe is an embeddable, typed functional pipeline language.** Write small
programs by composing values and functions; check the whole program before
it runs; let the embedding application decide which effects are available.

```fipe
[1, 2, 3] | map (n => n * 2) | fold 0 (a b => a + b)
```

This evaluates to `12`. Fipe has type inference, pattern matching, `Maybe`
and `Result`, checked limits, file modules, and a request/reply boundary for
host APIs, including nested namespaces such as `text.open` and `client.open`.
The core Rust crate uses only `core` and `alloc`, has no dependencies,
and forbids unsafe code. A VM, native backend, REPL, language server and debug
adapter are available as separate packages.

**Status:** `0.2.0-dev.2` is an experimental prerelease. Pin this version when
combining Fipe packages; language rules and Rust APIs may change. The earlier
`0.1.0` language is different. The [specification](Documentation/spec/README.md)
describes this implementation, with unresolved rules listed in
[open decisions](Documentation/spec/11-open.md).

## Try a script

Install the command with Rust and Cargo:

```sh
cargo install fipe-cli --version 0.2.0-dev.2 --locked
```

Save the example above as `hello.fipe`, then:

```sh
fipe check hello.fipe
fipe run hello.fipe
fipe repl
```

`check` reports diagnostics without running the script. `run` prints `12`.
In the REPL, submit an entry with a blank line; `:help` lists commands and
`:help map` shows a function's documentation and type.

The [tour](Documentation/tour/README.md) covers scripts, pipes, outcomes,
files and tools. The [CLI reference](tools/fipe/README.md) describes arguments,
stdin, exit codes and native builds. The standard host supplies filesystem
and environment operations; applications add other capabilities through
[host declarations](Documentation/api/09-embed.md).

## Embed in Rust

In your application's `Cargo.toml`:

```toml
[dependencies]
fipe = "=0.2.0-dev.2"
```

Compile and run a program without host effects:

```rust
use fipe::{
    check, compile, decl::Declarations, embed::Outcome,
    lex::lex, parse::parse, value::Value, vm::Machine,
};

let module = parse(lex("1 + 2")).expect("valid syntax");
let unit = check::check(
    &module,
    &Declarations::new(),
    &[],
    check::Options::default(),
).expect("valid types and limits");
let program = compile::compile(&unit);
let mut machine = Machine::new(Some(10_000));
let Outcome::Done(Ok(value)) = machine.run(&program) else {
    panic!("expected a completed pure program");
};
assert_eq!(value, Value::Int(3));
```

The budget bounds VM work; it is not a time or memory limit. A program that
calls a host function returns `Outcome::Request`; your application performs
the operation and resumes the VM with `Machine::reply`. Host declarations
determine the available operations, and replies are checked against them.

Runnable examples: [pure evaluation](examples/embed.rs) and
[a custom host function](examples/custom_host.rs). For scripts that need the
standard host, use [`fipe-host`](tools/host/README.md): `load`, `Policy` and
`Host::run` provide loading, write roots and execution. See the
[embedding API](Documentation/api/09-embed.md) for sessions and stepping.

## Choose a package

| Package | Use it for |
| --- | --- |
| `fipe` | Parse, check, compile and evaluate; embed your own host |
| [`fipe-host`](tools/host/README.md) | Run scripts with filesystem, arguments, environment and output |
| [`fipe-cli`](tools/fipe/README.md) | Install the `fipe` command |
| [`fipe-repl-core`](tools/repl-core/README.md) / [`fipe-repl`](tools/repl/README.md) | Embed a REPL without I/O / use its terminal driver |
| [`fipe-native`](tools/native/README.md) / [`fipe-native-rt`](tools/native/runtime/README.md) | Generate x86-64 Linux executables / supply their runtime |
| [`fipe-llvm`](tools/llvm/README.md) | Generate LLVM IR and compile it with Clang |
| [`fipe-native-core`](tools/native-core/README.md) | Shared native formats and operand layout |
| [`fipe-lsp`](tools/lsp/README.md) / [`fipe-dap`](tools/dap/README.md) | Editor language features / debugging |
| [`tree-sitter-fipe`](tools/tree-sitter/README.md) | Syntax trees and highlighting |

Install editor servers with `cargo install fipe-lsp fipe-dap --version
0.2.0-dev.2 --locked`. [Neovim](tools/editor/nvim/README.md) and
[Helix](tools/editor/helix/README.md) setup is included. The language server
needs the declarations of your host for host-specific completion and checks.

## Work on Fipe

All Rust packages share one Cargo workspace, lockfile and `target/` directory.
The root package is the default; use `--workspace` to check every package.
From a checkout:

```sh
cargo build -p fipe-cli --locked
cargo run -p fipe-cli -- run Documentation/examples/hello.fipe
cargo run --example embed
cargo run --example custom_host
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
```

The native tests need x86-64 Linux and `cc`; LLVM tests need `clang-23`
(or `FIPE_CLANG`). The tree-sitter package also needs a C compiler.
The core alone can be tested with `cargo test -p fipe --locked`.

[Development](Documentation/development.md) explains the workspace and checks.
[Agent guidance](Documentation/agents.md) gives coding agents the language
references and validation loop. [Publishing](Documentation/publishing.md)
describes the package contents and release process.

## License

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option.
