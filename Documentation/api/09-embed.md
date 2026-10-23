# Embedding API

[Embedding](../spec/09-host.md) owns what a host supplies; this chapter owns the Rust
boundary through which a driver supplies it. The [layer ownership guide](../architecture.md)
distinguishes this boundary from an application Host and optional Standard.

## Start in another repository

Add `fipe = "=0.2.0-dev.2"` to your Cargo dependencies. The
[README](../../README.md#embed-in-rust) contains a minimal checked evaluation;
[`examples/embed.rs`](../../examples/embed.rs) runs a pipeline and
[`examples/custom_host.rs`](../../examples/custom_host.rs) supplies a host
function through the request/reply protocol below. Run them in this workspace
with `cargo run --example embed` and `cargo run --example custom_host`.

For the standard filesystem/environment host, depend on
`fipe-host = "=0.2.0-dev.2"`; its [contract](../../tools/host/README.md) and
`load`, `Policy`, `Host::run` API cover real scripts. Applications defining their
own capabilities can depend on `fipe` alone. Pin matching prerelease versions
across the packages you use.

## Register nested host modules

```rust
use fipe::decl::Declarations;

let mut declarations = Declarations::new();
declarations.module("text.open", "## Open text.\npub new: Integer -> Integer").unwrap();
declarations.module("client.open", "## Show a region.\npub new: Integer -> Integer").unwrap();
assert!(declarations.get_module("text").is_some());
```

Scripts can use `use text.open` followed by `open.new 1` or `text.open.new 1`;
`use text` also permits `text.open.new 1`. Import the function directly with
`use text.open.new`. The call request's declaration index refers to the full
`text.open` or `client.open` namespace in the program tables. The
[host specification](../spec/09-host.md#nested-host-namespaces) defines naming,
privacy and collision rules. `Declarations::submodules` lists immediate
children for tooling.

## Requests and replies

~~~text
embed::Outcome<T> = Request(Request) | Done(T)
embed::Request    = File { stem: String }
                  | Call { declaration: usize, arguments: Vec<Value>,
                           environment: Vec<(usize, Value)> }
embed::Reply      = File(Option<String>) | Value(Result<Value, String>)
~~~

The crate calls nothing: no trait object, callback or I/O. Every effect is
a returned `Request`; the driver performs it and passes the answer to
`reply`, which resumes the step until `Done`.

| Request | Reply |
| --- | --- |
| `File { stem }`: the source of file module `stem`; the driver resolves the stem to source text | the text, or `None` when absent |
| `Call { declaration, arguments, environment }`: a run calls the host declaration `tables.declarations[declaration]` with its arguments in spot order, under the `with` keys (`tables.keys`) and values open, outermost first | `Value(Ok(value))`, a value of the declared result's shape ([MIR](08-mir.md) §Running), or `Value(Err(message))`, which Faults the run with `Host` at the call |

A source stem need not be an OS path: an application Host may resolve it from
memory or an application resource. Core and Embed perform no source I/O.
To load a file module, check `use <stem>` and answer its `Request::File`.
Giving the file's source directly to `Session::line` instead defines session-line
declarations; it does not assign that file's module identity.

A file type's `types::TypeName` retains `Origin::File(stem)` and its declared
name. For a runtime `Value::Record`, resolve `meaning` through the unit's
`tables.meanings`, then its `namespace` through `tables.namespaces`; together
the namespace and meaning label identify, for example, `geometry.Point`.
Function results share that identity across imports and fresh sessions.
Numeric table indices may differ, and normal type display may omit the module;
neither is a portable identity or a source-content fingerprint.

Sink policy is the host's: it reads each main expression line's value (a lift's
lines too) from `vm::Machine::lines` as lines end, at a request or the run's
end; the conformance host's policy heads `tests/fixtures/host/commands.fipe`.

~~~text
vm::input_spot(program) -> Result<ir::Type, InputError { spots: usize }>
Machine::run_with_input(&mut self, program, input: Value) -> Result<Outcome<
    Result<Value, Fault>>, InputError>
~~~

Host input ([host](../spec/09-host.md) §Input, output and observation) enters
the one open spot of the function the last top-level line makes, of the type
`input_spot` gives; `run_with_input` runs as `run`, then applies it, its result
the run's value and the line's in `lines`. Another open arity is `InputError`,
before any effect; input without the spot's shape Faults `Host` "the input is
not a T" at the line before the run starts, and its limits are the host's.

## Stepping

~~~text
vm::Stop = Before(node) | After(node, Value) | Request(Request)
         | Done(Result<Value, Fault>)
Machine::begin(&mut self, program: &mir::Program)
Machine::next_stop(&mut self) -> Stop
Machine::answer(&mut self, reply: Reply)
Machine::globals(&self) -> impl Iterator<Item = (usize, &Value)>
~~~

A stepped run is `run` handed out stop by stop: `begin` starts it,
`next_stop` advances it to `Before` the next statement or terminator of the top
body frame (a block's charge is not one), `After` a write into a frame (an
assignment, or a call's result reaching its destination; a marked pipe's,
`||`'s and `&&`'s result whichever side ran), a host request, which `answer`
replies to, or `Done`. Between stops a driver reads `frames()` (each body
frame's locals and captures by slot), `globals()` (each stored value
definition's value), `lines()`, `charged()` and `budget()`; the trace so far is
the requests it answered. Stops are not observations: a stepped run makes the
same requests and charges as `run`, and an unstepped run pays nothing for the
boundary. `next_stop` without a stepped run, or with a reply owed, panics.

## Session

~~~text
session::Session::new(declarations: decl::Declarations) -> Session
line(&mut self, text: &str) -> Outcome<Report>
query(&mut self, text: &str) -> Outcome<Report>
reply(&mut self, reply: Reply) -> Outcome<Report>
withdraw(&mut self, line: Accepted) -> Option<Withdrawn>
withdrawn(&self, name: &str) -> bool
strict(&mut self, enabled: bool)
compile_budget(&mut self, units: u64)
checks(&self, name: Option<&str>) -> Vec<typed::Check>
settled(&self, name: Option<&str>) -> Vec<check::Settled>
source(&self) -> &str
lines(&self) -> impl Iterator<Item = &str>
~~~

A session holds the prelude and the host's declaration files, given at
construction, the accepted lines and the file modules loaded so far. A session
is a sequence of lines, each checked against the definitions accepted before it,
so a later definition of a name shadows for the lines after it and never changes
an accepted line. Within one line the rules are a file's; an accepted line's
declarations are the session's own, so a later line may write an impl of its
type or read its private fields. `line` accepts a line when it has no diagnostic
of its own; `query` checks without accepting, and generalizes an expression line
where `line` defaults it, so its scheme is the line's `typed.binding`. A host
that edits an accepted line starts a new session over the lines before it, from
`lines`. The one line taken back is one whose run Faults
([host](../spec/09-host.md) §Compilation units): the host passes its
`Report::accepted` to `withdraw`, which leaves the session as if the line had
not been given, files it loaded and absent answers included, and gives the
`Withdrawn` that `vm::Machine::withdraw` takes. `withdraw` returns `None`,
changing nothing, unless the line is the last accepted one and not already
withdrawn; a refused line or a query in between leaves it the last. `withdrawn`
says whether a withdrawn line bound a name, so a host can word a later "unknown
name" for it; nothing else of the session remembers the line. A line awaiting a
reply is abandoned. `strict(true)` makes each open implicit entry a check error
in the line and its file modules; explicit gates remain allowed. It defaults to
false. `compile_budget(units)` sets the checker's steps each compile-time
evaluation of later lines may spend ([checking](07-check.md) §The compile
phase); it defaults to `check::COMPILE_BUDGET`. A line the compile phase refutes
is refused like any line with a diagnostic: the session is as before it.
`settled` lists the entries the phase proved, as `checks` lists the open ones.
The source is the accepted lines, each followed by a blank line; spans are in
the source followed by the line being checked. `lines` yields the accepted units
in order, keeping embedded blank lines.

A `use` of a module that is neither a host module nor loaded, in the line or a
loaded file, yields `Request::File`. A replied text is parsed and kept for the
session, and its resolve and type errors are reported once, with the line that
loaded it; `None` or text that does not parse leaves the `use` as
`UnknownModule` for that line. An absent or unparsable reply is remembered for
the session. A later line retries that stem only when it writes a `use` of it or
of a module whose imports reach it; other lines reuse the remembered answer and
report nothing new. `line` or `query` while a reply is owed abandons the owed
line; `reply` with none owed panics. Each file module is resolved and typed once
per checked unit; every use in that unit shares the result.

## Report

~~~text
Report { module: Option<ast::Module>, offset: usize,
         diagnostics: Vec<Diagnostic>, resolved: Option<resolve::Resolved>,
         typed: Option<typed::Typed>, checks: Vec<typed::Check>,
         settled: Vec<check::Settled>, warnings: Vec<typed::Warning>,
         unit: Option<ir::Unit>, accepted: Option<Accepted> }
Diagnostic = Syntax(parse::Error) | Resolve(resolve::Error)
           | Type(infer::Error) | File { stem: String, error: parse::Error }
           | FileResolve { stem: String, error: resolve::Error }
           | FileType { stem: String, error: infer::Error }
           | Compile { span: Span, what: check::Evaluated,
                       reason: value::Reason }
~~~

`module` is the line, parsed, with spans in the session's source followed
by the line; the line starts at `offset`, and `entries()` returns its
entries. `resolved` and `typed` index `module` once it resolves and types.
`diagnostics` are those of the files the line loaded, then the line's own,
which are empty exactly when it checks, spanned in the line's text, one
before the line clamped to its start. The three `File` forms carry a span
of the replied file; a reason's span stays in the text it was found in.
Resolve and type diagnostics within a file follow source order.

`unit` is present only when all checking stages succeed without any diagnostic.
`accepted` is present when `line` accepted the line, its token for `withdraw`.
It holds this line and indexes the tables retained by the session; values belong
to evaluation. [Checking](07-check.md) defines the intermediate form.

`warnings` never reject a line. `Warning { span, style: ast::CallStyle }`
marks a call whose argument-list and tuple readings both bind its spots;
double parentheses around a tuple suppress this warning. `Typed::warnings`
uses module spans; `Report::warnings` uses line spans. A failed written
reading whose alternative binds adds `Reason::CallReading { style }`.

`Type::Tuple` keeps its parts, including nested tuples. Array lengths are
boxed number types: `Number`, variables and canonical `Sum` nodes.
`Subst::settle_lengths` retries delayed sums; an unresolved equation is an
ambiguous length at generalization. A tuple part is `Place::Part(index)`.

`Session::documentation(&Report, ExprId)` returns the doc of a binding or
resolved declaration from the report, accepted lines, prelude, host or
loaded module. It reads syntax only and returns `None` without an owner.

## Open checks and known facts

`Typed::checks` and `Report::checks` list the open entries described by
[checks](../spec/07-checks.md), in entry order:

~~~text
typed::Check { place: Span, place_kind: typed::Place, limit: Span,
               phase, reason, frequency, test_count,
               place_origin, limit_origin, name, known, gate, when_integer, owner }
typed::Place = Binding(String) | Parameter(Option<String>) | Result
             | Field { owner: String, name: String } | Part(usize) | Element | Payload
~~~

The phase is compile, init or eval. `test_count` counts the owning type's
written tests, excluding its bases; when it exceeds one, `:checks` appends the
written test from `limit` to the type name. Reasons distinguish an unknown
input, no rule, compile-time evaluation and the budget. Frequency is once, per
call or per element. The place kind names a binding, parameter, result, field,
element or payload. Spans identify the place itself where written and belong to
the source containing the place or limit; imported declarations retain their
source identity. An entry refuted by the known facts is an error in every
session. `when_integer` marks a generic divisor check that runs only when its
instance meaning is `Integer`; it is listed once in the function, per call.
`owner` names the binding containing an entry, when one exists.

A binding also retains its earliest phase and holding across session
lines and file modules. `:t` prints `known at compile` or `known at init`
and a holding such as `held Size<3>` or `held Size<1>, 3 of 8 bits`.
Timings and holdings are checked and add no plan entry of their own.

Known facts are separate from the inferred type: exact values, Integer
ranges, names of limits passed, and construction parts. The REPL's `:t x`
prints only the facts held, one per line (`exact 300`, `range 200..300`,
`names Quad, U16`, `parts lo: 2, hi: ..`). `:checks [name]` prints the
open entries with the place in words, those same facts comma separated, and
the reason each remains open, after the entries the compile phase proved,
each with "proved at compile time on" and its value. Otherwise identical
entries include the place's line and column. Both REPL builds accept
`--strict` beside `--host <dir>` and `--cwd <dir>`; it enables strict
checking before input and survives `:host`. `--compile-budget <n>` sets
the session's compile budget, and survives `:host` too.

## Call binding

`Typed::bound` maps a call or stage's `(span.start, span.end)` to `Bound`.
`Bound::supplies` lists `(spot_index, Supply)` in source order; indices are
zero-based declaration positions. `Expression(ExprId)` is a written argument,
`Flag(Span)` a bare Bool flag, `Pipe(Span)` the input and `Head(Span)` the dot
receiver. Each `Element(ExprId)` appends one collected operand; interleaved
named arguments retain their source order. `Bound::defaults` lists `(spot_index,
DefaultValue)` in declaration order: `Declared { origin, value }` identifies the
expression in its declaring module, and `Type(Type)` selects the table default.
`Bound::open` lists unfilled positions in declaration order. A function value
keeps its types but no declaration's spot names or defaults. Evaluation consumes
this record; it does not bind again.

`decl::SpotProblem` distinguishes duplicate names, ambiguous Bool initials
and a supplied member's non-final `self`; declaration and inference reasons
carry it as `Spots { problem }`. `infer::Reason::NonBoolChaining` rejects
a valued chain or a chain naming a non-Bool spot. `Default { name }` reports
an invalid host default when a stage needs it. Program defaults are checked
at their declarations, with earlier spots in scope.

Written host values are typed in their home module and cached once per
checked unit alongside file modules. A failed value produces
`DeclarationValue { name, origin, declaration }` at its first use; the
origin and declaration span identify its binding. A default that needs the
failed value reports `Default { name }` for its own spot.

`Typed.casts` indexes expressions with optional `Cast { row, from, to }`.
`CastRow` is `Number`, `Read`, `Show`, `Codec`, `Bytes` or `Pack`; `can_fail()`
identifies number, read and codec casts for evaluation. Expression types retain
the source meaning; the cast gives the receiving place's type, whose limits are
checked after conversion. A fallible cast is not an open limit entry and does
not itself fail strict mode. `NothingPayload` and `OutcomeText { family }`
distinguish the outcome errors; the latter recommends `||` or `|:`.

## REPL driver

`fipe-repl-core::Repl::host_directory(&str) -> Vec<Out>` starts loading host
files directly from a directory, without interpreting the path as source or
commands. Drivers call it before feeding session input and service its
`Out::File` requests with `Repl::file`. Interior NUL is rejected.
