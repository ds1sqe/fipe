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

The durable plan root is `~/docs/plans/fipe/language/`: read
`00-master-plan.md` and `flight-log.md` on resume. Majors `01-spec.md`,
`02-lexer.md` (committed at `df3a471`) and `03-parser.md` record the finished
stages; the checker is the next major. Lexical and syntax behavior are
defined; checking follows parsing. The design record (`design/`,
`research/`, `intentions.md`) sits in that root; Reovim's own plan covers
only the editor's adoption of the language.
Release 0.1.0's dynamic implementation is retained in Git at `7606ff7`.
The user approved replacing that implementation as the new frontend lands.
Adopt useful behavior and test cases per stage.
