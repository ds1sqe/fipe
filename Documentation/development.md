# Development

Run commands from the repository root. All Rust crates are members of one
Cargo workspace with one `Cargo.lock` and `target/` directory. `cargo test`
selects the core language crate; `--workspace` selects all packages, and
`-p fipe-cli` (for example) selects one. Commands from a member directory
still select that member and use the shared target directory.

## Build and try

Use a current stable Rust toolchain and Cargo. A C compiler is required for
tree-sitter and native linking; native execution tests require x86-64 Linux.
LLVM integration uses `clang-23`, or the compiler named by `FIPE_CLANG`.

```sh
cargo build -p fipe-cli -p fipe-lsp -p fipe-dap --release --locked
cargo run --example embed
cargo run --example custom_host
cargo run -p fipe-cli -- run Documentation/examples/hello.fipe
```

The binaries are `target/release/fipe`, `fipe-lsp` and `fipe-dap`.
`cargo install --path tools/fipe --locked` installs the command from this
checkout. The [examples index](examples/README.md) distinguishes runnable
standard-host scripts from the language/conformance examples.

## Verify a change

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --no-deps
```

Use `cargo fetch --locked` once before working offline, then add `--offline`
to the commands. Release tests use `cargo test --workspace --release --locked`.
For core-only changes, `cargo test -p fipe --locked` runs its tests and README
doctests. `cargo test -p fipe-cli --test tour` checks the README, tour and
agent guide's Fipe snippets against the standard host.

The default REPL platform feature is `std`. Its alternative `linux` feature
is exclusive, so `--all-features` is not a valid workspace check. On x86-64
Linux, check it separately:

```sh
cargo build -p fipe-repl --no-default-features --features linux --locked
```

The workspace sets `panic = "abort"` in development and release profiles
for native code and the no_std REPL. Cargo's test harness uses unwinding.
Consumers compiling the no_std REPL outside this workspace must set the same
panic policy (for installation, `CARGO_PROFILE_RELEASE_PANIC=abort`).

With the `x86_64-unknown-none` target installed, verify the core's no_std
boundary with:

```sh
cargo check -p fipe --lib --no-default-features --locked \
  --target x86_64-unknown-none
```

Native equivalence tests also run against LLVM with
`FIPE_BACKEND=llvm cargo test -p fipe-native --test equivalence --locked`.
The editor packages document their own editor-dependent smoke checks.

## Package boundaries

`fipe` owns language rules and has no dependencies. Hosts and tools depend
on it; the core never depends on tools. `fipe-native-core` owns the shared
native byte formats and operand layout. Both backends and the runtime use
it, and `fipe-native::codec` preserves the existing public import path.

A published package must contain everything needed to compile it. Refer to
another crate through a versioned dependency, not a relative `include_str!`
or `#[path]` outside the package. Keep repository-only integration tests and
fixtures in the checkout; the tool packages include their implementation,
README, license texts and required assets. See [publishing](publishing.md)
for verification of the archives themselves.
