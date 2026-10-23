# Fipe

Fipe means Functional Pipeline Language. Five goals govern its design:

1. Embedded: a host supplies declarations, environment, an optional budget
   and input/output. The language owns no runtime beyond its prelude.
2. Extensible: files are modules, `pub` exports bindings and types, and a
   meaning's `impl` holds its members. Host APIs are ordinary modules.
3. Functional: functions are values, application curries, and every line has
   a value. There is no assignment or statement-only syntax.
4. Typed: inference and generalization follow Haskell with Rust-like spelling.
   Checking finishes before execution and produces a typed intermediate form.
5. Compiled: backends consume that form. The VM precedes a native backend;
   both must agree. An optional LLVM backend is a separate package,
   `tools/llvm`, held to the same agreement.

The [specification](spec/README.md) defines the language and its open questions.
The [tour](tour/README.md) teaches scripts on the standard host; the
[examples index](examples/README.md) includes runnable Fipe scripts and Rust
embedding examples. The [API chapters](api/) describe the crate boundary for
lexing, parsing, checking, compilation and execution.
The [native library boundary](api/10-native-libraries.md) separates existing
embedding from proposed C/Rust provider integration.

The implementation includes type inference, static limit checks, a VM,
native and LLVM backends, a REPL, and editor tooling. The core library uses
only `core` and `alloc` and depends on no host. The current prerelease is
`0.2.0-dev.2`; language rules and public Rust APIs are experimental. Published
`0.1.0` is the earlier language.

For contributors and integrators:

- [Development](development.md): workspace layout, builds and verification.
- [Agent guidance](agents.md): references and a check/run loop for coding agents.
- [Publishing](publishing.md): package boundaries and the crates.io release.
