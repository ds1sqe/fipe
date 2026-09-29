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
retract(&mut self) -> Option<String>
source(&self) -> &str
~~~

A session holds the prelude and the host's declaration files, given at
construction, the accepted lines and the file modules loaded so far.
A session is a sequence of lines, each checked against the definitions
accepted before it, so a later definition of a name shadows for the lines
after it and never changes an accepted line. Within one line the rules
are a file's; an accepted line's declarations are the
session's own, so a later line may write an impl of its type or read its
private fields. `line` accepts a line when it has no diagnostic of its
own; `query` checks without accepting, and generalizes an expression line
where `line` defaults it, so its scheme is the line's `typed.binding`.
`retract` removes the last accepted line and what it defined, returning
its text. The source is the accepted lines, each followed by a blank line;
spans are in the source followed by the line being checked.

A `use` of a module that is neither a host module nor loaded, in the line
or a loaded file, yields `Request::File`. A replied text is parsed and kept
for the session, and its resolve and type errors are reported once, with
the line that loaded it; `None` or text that does not parse leaves the
`use` as `UnknownModule` for that line, and a later line requests the stem
again. `line` or `query` while a reply is owed abandons the owed line;
`reply` with none owed panics.

## Report

~~~text
Report { module: Option<ast::Module>, offset: usize,
         diagnostics: Vec<Diagnostic>, resolved: Option<resolve::Resolved>,
         typed: Option<typed::Typed> }
Diagnostic = Syntax(parse::Error) | Resolve(resolve::Error)
           | Type(infer::Error) | File { stem: String, error: parse::Error }
           | FileResolve { stem: String, error: resolve::Error }
           | FileType { stem: String, error: infer::Error }
~~~

`module` is the line, parsed, with spans in the session's source followed
by the line; the line starts at `offset`, and `entries()` returns its
entries. `resolved` and `typed` index `module` once it resolves and types.
`diagnostics` are those of the files the line loaded, then the line's own,
which are empty exactly when it checks, spanned in the line's text, one
before the line clamped to its start. The three `File` forms carry a span
of the replied file; a reason's span stays in the text it was found in.
