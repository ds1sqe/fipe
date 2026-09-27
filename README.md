# Fipe

Functional Pipeline Language: embedded, extensible, functional, typed,
compiled. Fipe is its own Rust crate with no dependencies or editor coupling.

This development version implements the new language's lexer using
core + alloc, with no unsafe code. It provides owned tokens, UTF-8 byte spans,
scalar source positions and recoverable diagnostics. Parsing, checking and
execution are subsequent milestones. The prior dynamic implementation remains
in Git at commit 7606ff7 and on crates.io as version 0.1.0.

See the [specification](Documentation/spec/README.md) and its
[lexical contract](Documentation/spec/lexical.md). Lexical rules are defined
for this milestone; later-stage language questions are recorded separately.

## Library

~~~rust
use fipe::{
    lex::lex,
    token::{Kind, Source},
};

let text = "ls -l /etc | map .name";
let result = lex(text);
assert!(result.errors.is_empty());
assert_eq!(result.tokens[1].kind, Kind::Flag("l".into()));
assert_eq!(
    Source::new(text).slice(result.tokens[2].span),
    Some("/etc")
);
assert_eq!(
    Source::new(text).line_col(result.tokens[2].span.start),
    (1, 7)
);
~~~

Lexing collects errors and resumes so a host can show multiple diagnostics.
Reject a unit with any errors before parsing or executing it. No CLI, runtime
or host I/O is included in this version.

## Development

~~~sh
cargo fmt --check
cargo test --locked --offline
cargo test --locked --offline --release
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
cargo check --locked --offline --lib --no-default-features \
  --target x86_64-unknown-none
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
~~~

Tests exercise the public API and language corpus, numeric boundaries, string
decoding, error recovery, Unicode positions and malformed token combinations.
Warning denial belongs to these maintainer checks, so new compiler warnings
do not become build errors for consumers. Unsafe code remains forbidden.
The APIs and language remain experimental.

## License

Licensed under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
