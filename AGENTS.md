# Fipe

Fipe is the Functional Pipeline Language. Work in this standalone repository;
the editor is one future host. Read `Documentation/README.md` and the relevant
contract before changing an interface. The formal specification starts at
`Documentation/spec/README.md`; unresolved rules are listed in `open.md` there.
Specifications stay under 250 lines. State substantive choices in the contract
before implementing them. Follow the user's authorization to finish the active
plan; do not turn later-stage questions into blockers for independent work.

The new library must use `core` and `alloc`, with no dependency or unsafe code.
Keep host I/O and editor policy outside it. Prefer explicit ownership and small
private helpers. Public boundaries carry typed diagnostics and source spans.
Use rustfmt with an 80-column configuration for the new implementation.

For the new frontend, run `cargo fmt --check`, `cargo test --locked --offline`,
`cargo clippy --locked --offline --all-targets -- -D warnings
-D clippy::all -D clippy::pedantic -D clippy::nursery`, and
`cargo check --locked --offline --lib --no-default-features --target
x86_64-unknown-none` before a checkpoint. Test semantic results and malformed
input through public APIs. Keep README examples executable as doctests.
Keep blanket warning denial in check commands, not the library manifest or
crate attributes. Keep the unsafe-code prohibition in the library.

Durable execution plans and flight logs live under `~/docs/plans/fipe/`.
Start with `lexer/01-lexer.md` and its flight log for plan 01.0's implementation;
`spec/01-spec.md` records the formal-definition work. Read the applicable log
on resume. Lexical behavior is defined; parser decisions are the next stage.
Release 0.1.0's dynamic implementation is retained in Git at `7606ff7`.
The user approved replacing that implementation as the new frontend lands.
Adopt useful behavior and test cases per stage.
