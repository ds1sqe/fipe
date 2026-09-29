# Embedding API

[Embedding](host.md) owns what a host supplies; this chapter owns the Rust
boundary through which a driver supplies it.

## Requests and replies

~~~text
embed::Outcome<T> = Request(Request) | Done(T)
embed::Request    = File { stem: String }
embed::Reply      = File(Option<String>)
~~~

The crate calls nothing: no trait object, callback or I/O. Every effect is
a returned `Request`; the driver performs it and passes the answer to
`reply`, which resumes the step until `Done`.

| Request | Reply |
| --- | --- |
| `File { stem }`: the source of file module `stem`; the driver maps the stem to a path | the text, or `None` when absent |

## Session

~~~text
session::Session::new(declarations: decl::Declarations) -> Session
line(&mut self, text: &str) -> Outcome<Report>
query(&mut self, text: &str) -> Outcome<Report>
reply(&mut self, reply: Reply) -> Outcome<Report>
source(&self) -> &str
~~~

A session holds the prelude and the host's declaration files, given at
construction, the accepted lines and the file modules loaded so far. Its
source is the accepted lines, each followed by a blank line, so a new line
never continues an accepted one. `line` checks a line together with that
source, so hoisting, shadowing and `use` behave as in one file, and
accepts it when it has no diagnostic; `query` checks without accepting.

A `use` of a module that is neither a host module nor loaded yields
`Request::File`. A replied text is parsed and kept for the session; `None`
or text that does not parse leaves the `use` as `UnknownModule` for that
line, and a later line requests the stem again. `line` or `query` while a
reply is owed abandons the owed line; `reply` with none owed panics.

## Report

~~~text
Report { module: Option<ast::Module>, offset: usize,
         diagnostics: Vec<Diagnostic>, resolved: Option<resolve::Resolved>,
         typed: Option<typed::Typed> }
Diagnostic = Syntax(parse::Error) | Resolve(resolve::Error)
           | Type(infer::Error) | File { stem: String, error: parse::Error }
~~~

`module` is the session's source followed by the line, parsed, with spans
in that text; the line starts at `offset`, and `entries()` returns its own
entries. `resolved` and `typed` index `module` once it resolves and types.
`diagnostics` are the line's own, empty exactly when it checks, spanned in
the line's text, one before the line clamped to its start; `File` carries a
span of the replied file, and a reason's span stays in the combined text.
