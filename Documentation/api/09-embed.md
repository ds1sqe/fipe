# Embedding API

[Embedding](../spec/09-host.md) owns what a host supplies; this chapter owns the Rust
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
strict(&mut self, enabled: bool)
checks(&self, name: Option<&str>) -> Vec<typed::Check>
retract(&mut self) -> Option<String>
source(&self) -> &str
lines(&self) -> impl Iterator<Item = &str>
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
its text. `strict(true)` makes each open implicit entry a check error in
the line and its file modules; explicit gates remain allowed. It defaults
to false. The compile budget has no Rust form until evaluation exists.
The source is the accepted lines, each followed by a blank
line; spans are in the source followed by the line being checked.
`lines` yields the accepted units in order, keeping embedded blank lines.

A `use` of a module that is neither a host module nor loaded, in the line
or a loaded file, yields `Request::File`. A replied text is parsed and kept
for the session, and its resolve and type errors are reported once, with
the line that loaded it; `None` or text that does not parse leaves the
`use` as `UnknownModule` for that line, and a later line requests the stem
again. `line` or `query` while a reply is owed abandons the owed line;
`reply` with none owed panics. Each file module is resolved and typed once
per checked unit; every use in that unit shares the result.

## Report

~~~text
Report { module: Option<ast::Module>, offset: usize,
         diagnostics: Vec<Diagnostic>, resolved: Option<resolve::Resolved>,
         typed: Option<typed::Typed>, checks: Vec<typed::Check>,
         warnings: Vec<typed::Warning> }
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
Resolve and type diagnostics within a file follow source order.

`warnings` never reject a line. `Warning { span, style: ast::CallStyle }`
marks a call whose argument-list and tuple readings both bind its spots;
double parentheses around a tuple suppress this warning. `Typed::warnings`
uses module spans; `Report::warnings` uses line spans. A failed written
reading whose alternative binds adds `Reason::CallReading { style }`.

`Type::Tuple` keeps its parts, including nested tuples. Array lengths are
boxed number types: `Number`, variables and canonical `Sum` nodes.
`Subst::settle_lengths` retries delayed sums; an unresolved equation is an
ambiguous length at generalization. A tuple part is `Place::Part(index)`.

Supplied claim member types are validated when declarations load. Inference
reports private fields with `PrivateField { field }`; a `Conversion` reason
keeps `source: Option<Type>` beside its target and candidates, distinguishing
an unknown source or target from a known source with no matching candidate.
`TypeDepth` reports an inferred type deeper than `parse::MAX_NESTING`;
substitution unification exposes this as `types::Mismatch::DepthLimit`.

`Session::documentation(&Report, ExprId)` returns the doc of a binding or
resolved declaration from the report, accepted lines, prelude, host or
loaded module. It reads syntax only and returns `None` without an owner.

## Open checks and known facts

`Typed::checks` and `Report::checks` list the open entries described by
[checks](../spec/07-checks.md), in entry order:

~~~text
typed::Check { place: Span, place_kind: typed::Place, limit: Span,
               phase, reason, frequency, test_count }
typed::Place = Binding(String) | Parameter(Option<String>) | Result
             | Field { owner: String, name: String } | Part(usize) | Element | Payload
~~~

The phase is compile, init or eval. `test_count` counts the owning type's
written tests, excluding its bases; when it exceeds one, `:checks` appends
the written test from `limit` to the type name. Reasons distinguish
an unknown input, no rule, compile-time evaluation and narrowing. Frequency
is once, per call or per element. The place kind names a binding, parameter,
result, field, element or payload. Spans identify the place itself where
written and belong to the source containing
the place or limit; imported declarations retain their source identity.
An entry refuted by the known facts is an error in every session.

A binding also retains its earliest phase and holding across session
lines and file modules. `:t` prints `known at compile` or `known at init`
and a holding such as `held Size<3>` or `held Size<1>, 3 of 8 bits`.
Timings and holdings are checked and add no plan entry of their own.

Known facts are separate from the inferred type: exact values, Integer
ranges, names of limits passed, and construction parts. The REPL's `:t x`
prints only the facts held, one per line (`exact 300`, `range 200..300`,
`names Quad, U16`, `parts lo: 2, hi: ..`). `:checks [name]` prints the
open entries with the place in words, those same facts comma separated, and
the reason each remains open. Otherwise identical entries include the place's
line and column. Both REPL builds accept `--strict` beside `--host <dir>` and
`--cwd <dir>`; it enables strict checking before input and survives `:host`.
