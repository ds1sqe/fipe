# Fipe

Functional Pipeline Language: embedded, extensible, functional, typed,
compiled. Fipe is its own Rust crate with no dependencies or editor coupling.

This development version implements lexing, parsing, name resolution and
type inference with explicit claims and members, and static limit checks with
gates and a plan of open entries, using `core` and `alloc` and no unsafe code.
Timing, holdings, narrowing, general casts and execution follow. The
[specification](Documentation/spec/README.md) defines the language; the
prior implementation is version 0.1.0, Git `7606ff7`.

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

Lexing collects errors and resumes; parsing rejects any unit with errors.

~~~rust
use fipe::{
    ast::{Entry, ExprKind, PipeOp},
    lex::lex,
    parse::parse,
};

let module = parse(lex("ls -l /etc | map .name")).unwrap();
let [Entry::Line(id)] = module.entries.as_slice() else {
    panic!("expected one pipeline");
};
assert!(matches!(
    &module.expr(*id).unwrap().kind,
    ExprKind::Pipe { op: PipeOp::Plain, .. }
));
~~~

The [parser API](Documentation/api/02-parser.md) keeps spans, argument
order, docs and comment trivia without resolving names or types.

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

Warning denial lives in these checks, not the manifest, so a new compiler
warning never breaks a consumer's build. The APIs and language are
experimental.

## License

Licensed under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
