# Development

Run commands from the repository root. All Rust crates are members of one
Cargo workspace with one `Cargo.lock` and `target/` directory. `cargo test`
selects the core language crate; `--workspace` selects all packages, and
`-p fipe-cli` (for example) selects one. Commands from a member directory
still select that member and use the shared target directory.

## Build and try

Use Rust 1.98.1 or newer and Cargo. A C compiler is required for
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

## Continuous integration

[CI](../.github/workflows/ci.yml) calls `bash scripts/ci.sh MODE [debug|release]`.
The modes are `quality`, `linux`, `vm`, `features` and `package`; use Rust 1.98.1
with the required components/dependencies already available. The script does not
install toolchains or system packages. Linux mode needs `cc` and `clang-23`
(or `FIPE_CLANG`); features mode needs the matching bare-metal and Windows target
components. Fetching dependencies requires network access unless already cached.

The workflow separates debug/release scratch on Linux and native-host Windows,
ARM64 macOS and Intel macOS runners. The foreign-OS VM recipe includes the Host
library, run, write-root and declaration suites, plus all-target Host strict
checks in each profile. Linux-native CLI cases remain filtered; full
path/security/terminal/editor qualification remains a platform release gate.
A workflow definition is not an observed CI pass or completed platform
qualification. Hosted Linux provisioning
uses the official signed LLVM repository; running that installation outside the
reviewed delivery requires explicit authorization.

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

## Compatibility and support

All 13 workspace packages use a supported Rust floor of **1.98.1**. This is a
chosen support floor, not a claim that it is the oldest compiler that can build
Fipe. The floor stays fixed through final `0.2.x` patch releases.

Final `0.2.x` patches preserve public Rust APIs (including public fields, enums
and modules), language and host namespace/permission semantics, CLI flags and
exit behavior, and advertised LSP/DAP behavior. Published dev.2 tools and
Core/REPL-core dev.3 are prereleases; migrations remain explicit and consumers
should pin exact versions. See the [package publication status](publishing.md).
No stable Rust memory ABI, persisted-session format or cross-version native
codec is promised.

| Surface | Selected release targets | Qualification |
| --- | --- | --- |
| Core/embedding/VM, CLI, LSP and DAP | `x86_64-unknown-linux-gnu` | Linux floor compilation; release runtime checks remain required |
| Core/embedding/VM, CLI, LSP and DAP | `x86_64-pc-windows-msvc` | Windows runtime qualification pending |
| Core/embedding/VM, CLI, LSP and DAP | `aarch64-apple-darwin`, `x86_64-apple-darwin` | Both macOS runtime qualifications pending |
| Native and LLVM executables | `x86_64-unknown-linux-gnu` only | Match compiler and runtime versions exactly; release artifact checks required |
| `fipe` and `fipe-repl-core` libraries | `no_std` + `alloc` | Bare-metal compilation checks; allocator, executable and hardware execution are host responsibilities |
| `fipe-repl` alternative `linux` feature | x86-64 Linux only | Separate executable build, `panic = "abort"` required |
| `fipe-ffi` experimental ABI 2 | VM scalar proof on `x86_64-unknown-linux-gnu` | C/Rust proof only; no resource, native/LLVM execution or other-target guarantee |
| `tree-sitter-fipe` | C compiler required | Editor integration is qualified separately |

Selected targets are release requirements, not claims that every platform has
already passed runtime tests. Other architectures, libc variants and Windows
GNU targets are not qualified by these rows. The std REPL needs real terminal
and cooked-input checks on each OS; Unix `stty` behavior does not establish
Windows interactive behavior. Core and REPL-core do not depend on Standard;
native-core and the native runtime do not share that `no_std` promise.

Native generated artifacts and runtime archives require the **exact same Fipe
version** as the compiler. This is a pairing requirement, not a claim that every
mismatched artifact is mechanically rejected. Internal package dependencies stay
exactly pinned together. LLVM uses `clang-23` or `FIPE_CLANG`; its executables
share the Linux native runtime/linker scope.

The repository uses `Cargo.lock` with `--locked` for reproducible checks.
`cargo install --locked` uses the package's shipped lockfile and is a different
gate from a fresh external consumer resolving dependencies. For fresh consumer
qualification, prefer compatible dependencies explicitly:

```sh
cargo +1.98.1 generate-lockfile --config 'resolver.incompatible-rust-versions="fallback"'
cargo +1.98.1 check --locked
```

Cargo's [fallback preference](https://doc.rust-lang.org/cargo/reference/config.html#resolverincompatible-rust-versions)
is not a guarantee: if no Rust-compatible version satisfies the constraints,
resolution can still select an incompatible dependency. A passing clean-consumer
check is required; configuration alone does not establish the floor. See Cargo's
[rust-version reference](https://doc.rust-lang.org/cargo/reference/rust-version.html)
and [publishing checks](publishing.md). Offline checks qualify only the cached
registry state, not future unconstrained dependency resolution.

Floor verification includes `cargo +1.98.1 check --workspace --all-targets
--locked`, the std tool builds and separate exclusive `linux` REPL build above.
The two-package C/Rust provider proof is a separate workspace; use its
[documented command](../tools/ffi/README.md). Verify a clean standalone `no_std`
library depending only on `fipe` and `fipe-repl-core` on an installed matching
bare-metal target; inspecting `cargo tree --edges normal` must show no host/tool
edge. Checking the root workspace alone does not establish that consumer boundary.
Neither/both REPL features must fail with the intended feature diagnostic, and
the `linux` feature must reject non-Linux/x86-64 targets. Missing target components
are prerequisites, not successful negative tests. Do not use workspace
`--all-features` as a passing gate.
