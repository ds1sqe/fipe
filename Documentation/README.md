# Fipe

Fipe means Functional Pipeline Language. Five goals govern its design:

1. Embedded: a host supplies declarations, environment, execution budget and
   input/output. The language owns no runtime beyond its prelude.
2. Extensible: files are modules, `pub` exports bindings and types, and a
   meaning's `impl` holds its members. Host APIs are ordinary modules.
3. Functional: functions are values, application curries, and every line has
   a value. There is no assignment or statement-only syntax.
4. Typed: inference and generalization follow Haskell with Rust-like spelling.
   Checking finishes before execution and produces a typed intermediate form.
5. Compiled: backends consume that form. The VM precedes a native backend;
   both must agree. An optional LLVM backend would be a separate crate.

These are goals, not implementation claims. The [specification](spec/README.md)
defines the language in eleven numbered chapters, from characters to the
host; [api](api/) is the crate's Rust boundary for each stage; the programs
under [examples](examples/) show the language as written. The frontend
implements lexing, parsing, name resolution and type inference with
explicit claims and members; limits, casts and execution follow. Their remaining questions are
recorded in [open rules](spec/11-open.md). The library is independent of every
host and uses only `core` and `alloc`.

Public Rust APIs are experimental. The published 0.1.0 implementation remains
available in Git at `7606ff7`; it has been replaced on the development branch.
