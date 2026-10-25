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

## Opaque resource references

This subsection describes the working checkout's qualified resource VM API,
not a guarantee about earlier published package contents. Native and LLVM
currently refuse marked programs; Codec data and scalar FFI gain no resource
transport.

A trusted Host/commands interface may declare `pub type resource Table` and
operations returning or accepting `Table`. A loaded script file cannot declare
a resource type. Scripts can copy references, put them in typed containers and
capture them in functions, but cannot manufacture them from records, integers,
bytes, literals, `default` or ordinary Codec data. `Packet` remains ordinary data.
Metadata or schema is ordinary typed data supplied alongside a reference; a
reference is not its payload or a memory address.

~~~text
value::resource::{ResourceDomain, ResourceType, ResourceRef, ResourceError, ResourceStep}
Value::Resource(Rc<ResourceRef>)
ResourceDomain::new() -> ResourceDomain
ResourceDomain::issue(&self, ty: &ResourceType, key: Rc<[u8]>) -> Value
ResourceType::registered(tables, namespace, name) -> Result<ResourceType, ResourceError>
Machine::pending_resource_type(&self) -> Result<ResourceType, ResourceError>
ResourceRef::{domain, resource_type, key}
~~~

At a `Request::Call`, `pending_resource_type` returns the exact frozen direct
resource result, including its generic arguments. The Host validates or
acquires its own entry, then wraps its opaque entry key with
`domain.issue(&ty, key)` and returns the value through the usual reply. Unit
arguments, full module markers and ordinary generic arguments retain their
checked identities. A composite result returns `NotResourceResult`: this
accessor does not guess a nested resource position. For new composite issuance
use [`pending_resource_type_at`](#new-resources-inside-composite-replies).
Already issued references may be returned inside compatible typed tuples,
sequences, records and packed values. A typed function may capture references;
an opaque or wildcard data position cannot hide one, including in captures or
partially applied arguments. This shape check does not validate an arbitrary
supplied function body's behavior.

`registered` finds an already materialized marked zero-parameter meaning by
exact namespace and name. An unused declaration may return `NotFound`; a
generic type returns `NotMonomorphic`. It does not register a type, lower
source eagerly or accept an arbitrary `ir::Type`. Registered, direct and
projected witnesses describe language type identity only. Ordinary nominal
arguments carry identity too; matching labels or reused numeric slots are
insufficient.

The call's resource-relevant expected fields, arguments and diagnostic site
are frozen before its request. Nonconcrete resource identity refuses before
that request. Host input uses the same admission rules before module
initialization or the main body; invalid input arity still leaves a previous
run untouched. New resource type expansion and matching use the Machine
budget. Budget refusal before a request prevents that request; refusal while
admitting a reply follows an effect the Host has already performed and does
not roll it back. An unbudgeted Machine remains unbounded. Trusted Rust
registration and entry-key work are outside Machine accounting.
Guard/predicate copying has its separately documented logical work schedule;
this is not a bound on every historical VM traversal or external Host
operation.

A Tables context keeps identity across runs, including a new Machine using the
same tables. A Session fork or fresh Tables context gets fresh identity even
when its stamp cache was empty; withdrawal retires removed identities while
preserving surviving ones. To reuse a still-live Host entry in another checked
context, the Host explicitly wraps it with that context's validated witness.
This is not another acquisition and cannot revive a disposed entry.

The Host owns payloads, grants, liveness, entry generations and cleanup. Domain
identity and key access let it reject an old owner's alias before payload lookup,
even when a new run uses the same language Session and recycles a numeric slot.
Core type fit deliberately does not prove owner liveness. Cloning a wrapper
preserves aliases; dropping the last wrapper closes nothing. On Fault or
abandonment, the driver revokes that run's new entries without invalidating live
entries retained from earlier successful runs. See [abandonment](#abandon-unfinished-execution)
for the execution boundary; physical cleanup remains the Host's action.

Rust equality compares domain, exact checked type and key; it is not a liveness
or authority check. Structural output is `<resource>` and Debug redacts entry
keys, domains, addresses and payloads. There is no intrinsic language equality,
ordering or construction for resources; supplied Host members remain the route
for meaningful operations.

This prerelease adds a `Value` variant, so exhaustive Rust matches need updating.
`BodyFrame` and `value::Closure` now drop materialized type witnesses iteratively.
Because they implement `Drop`, moving fields directly out of an owned frame or
closure no longer compiles; use `core::mem::take(&mut owner.field)` when transferring
an owned field. Native frames also drop their private result witness iteratively.

`value::Guard` also implements `Drop` to dispose of its predicate tree iteratively.
Its public fields and value/wire representation are unchanged, but moving its
non-`Copy` fields out of an owned guard, including by destructuring or struct
update (`Guard { spot, ..guard }`), may now fail with Rust E0509. Borrow fields
(for example, `let predicate = &guard.predicate;`), or clone individual fields
when a separate owned value is needed. To change a field, mutate the guard in
place instead of using struct update. The public derived `Guard::clone` still
recursively copies predicate/type trees outside VM budget accounting. It is not
a substitute for runtime metered copying; iterative destruction does not promise
stack safety for arbitrary deep cloning or predicate execution.

The hidden `Value::Meaning` payload is now `Rc<value::Meaning>`: wrap an existing
`ir::Type` with `value::Meaning::new(ty)` and borrow it with `as_type()`. It keeps one
`Rc` ownership layer and drops the type tree iteratively on the last alias. This
hidden witness is not a checked resource identity or Host authorization. Structural
equality and underlying type Debug remain; arbitrary deep Type equality and cloning
are not made iterative by this destructor change.
`Tables` also has a private runtime identity cache: external struct literals are
no longer source-compatible. Read checked tables through `Unit::tables` instead
of constructing a checked resource witness from a hand-built table.

### New resources inside composite replies

`Machine::pending_resource_type_at(&mut self, path: &[ResourceStep])` selects
from the **same immutable pending expectation** used to validate the reply.
`ResourceStep` is in `value::resource`:

~~~text
Tuple(index)
Field { constructor, field }
Element
Packed(index)
~~~

Ordinals are local to the checked result schema, not global Meaning indices or
portable handles. Records use their sole constructor ordinal 0; variants choose
one explicitly. `Element` selects a homogeneous Vec/Deque/Array type element,
not a value index. `Packed` selects one explicit alternative. These operations
never execute a getter, constructor, default, provider or written member.
Resource arguments form identity; they are not payload children. Function captures,
opaque payloads and wildcard data have no constructive child path. Repeated nominal
steps are allowed and may change generic arguments through frozen field templates.

For example, a trusted `make: () -> Envelope<Rd<a>>` called under an
`Envelope<Rd<Integer>>` annotation has no direct root witness and its generic `Rd`
cannot use `registered`. After receiving that real Call, select its field:

```rust,no_run
use fipe::value::resource::{ResourceError, ResourceStep, ResourceType};
use fipe::vm::Machine;
fn pending_leaf(machine: &mut Machine) -> Result<ResourceType, ResourceError> {
    machine.pending_resource_type_at(&[
        ResourceStep::Field { constructor: 0, field: 0 },
    ])
}
```

Only after this succeeds may the Host acquire/register a NEW private entry and
call `domain.issue(&witness, key)`. Put that reference in the ordinary checked
Envelope shape and reply normally. The full concrete argument structure and nominal
stamps come from the snapshot, not current declarations or a late registration.
A witness is neither an ownership transfer nor a fresh grant. Same-typed occurrences
can reuse a still-valid alias under Host policy.

A fresh resource-bearing Packed needs a checked association, not descriptive Types.
`Packed::from_data` still refuses resource payloads. A Host can precompile a lawful
same-context converter, for example `Rd<Integer> -> (Rd<Integer> | String)`, **before**
the pending factory Call. Obtain the resource witness by a `Packed` path, acquire
and issue the NEW entry, then use another Machine's `run_with_input` on that
prechecked converter. Return its exact checked Packed to the original pending
Machine. All Programs must share the same checked Tables/context and the result
must pass the original frozen reply admission. This is not an arbitrary builder
for unseen types; a generic composite factory never falls back to an auxiliary
direct-resource call to obtain its witness.

The empty path reuses the cached direct accessor without work. `NoPendingCall`
also costs nothing. A whole frozen expectation proven unable to expose resources
returns `NotResourceResult` before validating any nonempty path. This includes
ordinary/data-only results, a whole zero-length resource array, and Function-only
or opaque-only roots, independently of unrelated registered resources. In a
resource-bearing tuple/record, however, selecting a zero array and then asking for
its `Element` is `InvalidPath` at that step. Other wrong kinds or ordinals report
`InvalidPath { step }` at the first zero-based bad step. An ordinary/data-only
exhausted leaf is `NotResourceResult`; missing required concrete resource identity
is `NonConcrete`, never a guessed witness. Such required unresolved identities
normally refuse the Host request during capture already.

A nonempty resource-relevant path pays 1 before each step inspection, then the
existing charged materialization of the selected subtree (including field formal
substitution). It pays 1 before inspecting the final leaf, followed by the existing
concrete-type and frozen-descriptor work. It does not copy unselected siblings or
expand recursive layouts eagerly. Successful and invalid lookups retain all spent
charges and leave the Call pending, so a corrected path can be retried.

On exhaustion the getter returns `ResourceError::Budget`, consumes the remaining
allowance, and latches that refusal on the pending Call. **Stop backend/effect work.**
Every subsequent root/path getter returns Budget without new work, even after
changing the configured budget. Supply the ordinary `Reply::Value(Ok(..))` or
`Reply::Value(Err(..))` (or stepped `answer`) to observe the captured original-site
Budget terminal; the supplied result cannot replace it or resume later effects.
The getter itself does not end the Call. Wrong-kind File replies remain driver
errors. Abandoning instead drops the pending refusal; a new run starts clean.

This prerelease adds `ResourceStep`, the method and exhaustive `ResourceError`
variants `InvalidPath` and `Budget`; Rust matches must account for them. No new
native/FFI wire representation, live schema API or backend cleanup hook follows.

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

A compiled `mir::Program` also retains receiver-first adapter locators and
body-indexed invocation contexts. These are trusted compiler output, like its
bodies; hosts constructing Program literals must supply both collections.
Partial methods keep the original receiver in the ordinary prefix-filled
closure. On full application the captured receiver expectation is checked
before guards or body effects. Operation witnesses retain their original order
following the adapter's additional receiver witness. This does not transfer
ownership or replace Host access and liveness checks.

Programs sharing the same `Tables` `Rc` share one Machine runtime context:
globals, captured function bodies and initialized modules persist across those
session lines. Opening a program with a different `Tables` `Rc` starts a fresh
runtime context, clearing old indexed globals, initialization flags and caches;
returning to an earlier independent program is fresh again. Runtime closures are
not migrated between contexts. This does not change canonical file-module nominal
identity, undo effects or close Host resources. The driver still handles an active
or undelivered terminal run before switching programs.

## Abandon unfinished execution

A direct driver establishes its own fresh run identity before `Machine::run` or
`Machine::begin`. `Outcome::Done` and `Stop::Done` are execution terminals;
`Session` returning `Done(Report)` only completes checking. A successful VM result
includes a language `Err` value; it is distinct from a VM Fault.

`Machine::abandon() -> bool` stops an unfinished run, clearing transient frames,
environment, pending reply and stepping stops without calling the Host. First
invalidate the driver's pending-call association. On `true`, the driver can revoke
that run's newly created resources, then pair `Session::withdraw(accepted)` with
`Machine::withdraw(&withdrawn)` before accepting a replacement line. Earlier
committed globals and module initialization remain. Abandonment alone does not
roll back partial bindings, output, writes, explicit closes or any other effect;
line values and charges remain available for inspection, not as a resource ledger.

`false` changes nothing. If a stepped terminal is still pending, drain its
remaining stops to `Stop::Done` and handle the actual success/Fault;
a completed run must not be mislabeled abandoned. Calling `next_stop` again after
its terminal violates the stepping precondition. Repeated abandonment and an idle
machine also return false.

Reply matching remains the direct driver's responsibility. A reply with no owed
call panics; after a replacement run suspends, the VM cannot distinguish old reply
data from a current answer. Reject old-generation answers before `reply`/`answer`.
`Accepted` serials and cloning a Session do not supply run or authority identities.
Use explicit abandonment/withdrawal rather than treating `run`, `begin` or
`withdraw` as a cancellation protocol. No automatic Host cleanup or async token
API is implied; REPL lifecycle delivery is a separate contract.

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

`unit` is present only when all checking stages succeed without any diagnostic
and the code it depends on has an emitted definition or checked source. Recovery
typing may accept a line without a unit; acceptance alone does not make that
binding executable. Later lines that depend on its unavailable code also have no
unit, without repeating the file's diagnostics.
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

`fipe-repl-core::Repl::host_directory(&str) -> Result<Vec<Out>, DriveError>`
starts loading host files directly from a directory, without interpreting the
path as source or commands. Drivers call it before feeding session input and
service each `Out::File { id, path }` with `Repl::file(&id, &path, text)`.
`Out::Call { id }` is answered with `Repl::answer(&id, callback)`. These
methods return a `Result`; a stale or wrong-kind identity, or wrong file path,
returns `DriveError::WrongRequest` without changing the request or invoking
the callback. Ordinary executable units yield `Out::Start { run, resume }`
before runtime execution and `Out::End { run, result, resume }` before
presentation, failed-line withdrawal or next buffered acceptance. Handle each
boundary and process `Repl::resume(&resume)` outputs in order; stale/foreign
continuations return `WrongContinuation` unchanged. `RunEnd::Success` includes
language `Err` data; Fault carries the actual VM fault; Abandoned reports an
unfinished run stopped by interrupt/close. Interrupt preserves an existing
terminal or Reset. File/Call IDs and boundary continuations are distinct.
Valid host replacement yields `Out::Reset` before old language state is
discarded; acknowledge after owner disposal, then replay proceeds through
Start/End using cached sources and refusing Host calls. Invalid declarations
preserve the old session. `close()` yields Exit Reset, possibly after an
Abandoned End; computed End/Reset returns Busy unchanged. `flush()` only
drains input: at EOF drain it, then close and drain Reset through Out::Exit.
Built-in EOF/error paths perform explicit closing; read errors retain exit2
after terminal restoration. After Exit, Vec APIs are inert, close is
idempotent and host replacement returns Closed. Start excludes bounded pure
checking/compile evaluation. The built-in Host adapter auto-acknowledges and
has no cleanup hook; actual resource owners need a lifecycle-aware repl-core
driver. No GC callback, physical cleanup, native preemption, rollback or owner
authorization is implied. The lifecycle conformance driver demonstrates actual
resource ownership using these boundaries; it does not install a universal
owner in the built-in adapter. Interior NUL in a host-directory path is
rejected.