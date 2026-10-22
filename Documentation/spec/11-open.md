# Open specification decisions

These are proposals for review, not approved behavior. Examples distinguish
choices; an implementation must not fill a gap without updating the owning
rule. T/E/H concern checking and execution. Lexical choices are defined by
[lexical structure](01-lexical.md). The user settled parser choices
S1–S3 on 2026-09-27 and T1–T3 on 2026-09-29; [syntax](02-syntax.md),
[types](03-types.md), [containers](05-containers.md), [limits](06-limits.md),
[checks](07-checks.md) and [members](04-members.md) own their rules.

## L1 — Written forms

Ruled 2026-09-30: a program's prefix and suffix on a number or quote
([lexical](01-lexical.md) §Numbers, §Quoted literals; [members](04-members.md)
§Literals). Open:

- The reader's spelling. **Proposed:** in the type's `impl`, exported by
  `pub` and brought in by `use`: `impl Regex { literal re"" = f => … }`,
  `impl Complex { literal ""i = f => … }`.
- Whether a path, a word and a bracket take a prefix or suffix.
- New operators from operator characters, precedence by the first
  character, pipe symbols reserved: a direction, not ruled.
- The `Form` of a quote whose bytes are not UTF-8, and of the digits
  after a program's prefix.

## L2 — Docs and quotes

- **Proposed, not ruled:** `dedent: String -> String`, removing common
  leading spacing from lines after the first in a raw quote.

## T1 — Outcome identity (ruled)

Three nominal prelude types with per-type operator eligibility;
[types](03-types.md) §Outcome types. Whether a backend shares their layout
is E2/H1's.

## T2 — Generalization, effects and recursion (ruled)

[Types](03-types.md) §Core judgments, "Generalization". Effects are not
typed in this draft; a host call is typed by its declared signature.
Purity for the compile phase is [checks](07-checks.md) §Phases.

Open, from the dogfood scripts: a binding inside an arm body
(`f = s => { body = n x => n + 1 … }`) generalizes over `Literal a`,
and its closure carries the witnesses of its creation, so `1` has none
at the call and the run Faults where the check passed. **Proposed, not
ruled:** a local binding fixes each variable carrying a member limit
at the binding, defaulting `Literal` there, as a binding whose value
is not a function does; or the check refuses a local binding whose
scheme keeps a member limit.

## T3 — Declarations

A trait is a type (ruled 2026-09-29), and every operation lives in a
type: [types](03-types.md), [limits](06-limits.md), [checks](07-checks.md) and
[members](04-members.md). Still open from T3: `Self<A>`, inferred Bool
arguments, default-expression evaluation and termination of recursive
type detection. Binding is ruled 2026-09-30: a function spot uses only its
declared default, a record field also its type default; a limited type
default requires proved entry. A trailing Vec's single operand tries the
whole list before one element; no operand leaves it for pipe and defaults.
`Maybe` and `Result` are valid parameter types. Host default errors are
reported at the first stage needing them, naming the spot; program defaults
are checked at their declaration ([evaluation](08-evaluation.md),
[types](03-types.md), [host](09-host.md)).

Ruled 2026-09-30: a host type that takes an API reference declares its
parameter uppercase, `type Rd<Api>`; the use site `Rd<fs>` stays
([types](03-types.md) §Type forms).

## T4 — Limits and checks

- Bit order and unused bits inside a record held by `Bits` and `Size`,
  and the layout of a sum with payloads.
- May literal patterns under a limit complete an arm set, as `0` and `1`
  under `Bit`? Not yet.
- A default for a tuple: the tuple of its parts' defaults, when each has
  one.
- The shape of `LimitError`, the `Err` of `try<T>` when a limit fails
  ([limits](06-limits.md) §Gates); the prelude declares it opaque.
- May a limit read a value known at init when it says `Init`?
- Whether an earlier evaluated session value is exact remains open,
  except a value a compile demand prepared, which is exact for the lines
  after it. The checked line indexes tables kept by the session;
  evaluation owns values.
- A hatch for hardware, in design: a trusted entry, and the holdings
  `Volatile`, `Pinned`, `At` and `Order`, byte order, offsets and
  addresses.

## T5 — Members

- `Items<T>` is `fold` (ruled 2026-09-30, [containers](05-containers.md)
  §Brackets). Open, for a later stage: a walk that stops early, and one
  held by a front and a back position, from either end.
- A bracket pattern for a program's own type; arrays and `Vec` only now.
- Marked pipes on a user meaning through `then`, `pure`, `or`, `empty`.

## E1 — Evaluation and outcome rebuilding (ruled)

Ruled 2026-10-08 as [evaluation](08-evaluation.md) §Order and §Marked
pipes state: strict left to right, each supplied expression once, defaults
in parameter order, the same-family reading of a marked pipe's return
preferred and recorded by the checker. Still check errors, not runtime
rules: `|:` and `|!` on Bool; a binding pattern neither total nor a good
side (`[x] = []`); a default naming a local of an enclosing function or
its own binding. Letting a default capture is open.

## E2 — Arithmetic (ruled)

Ruled 2026-10-08: [evaluation](08-evaluation.md) §Arithmetic. The default
is a panic: a float operation with a non-finite result Faults, a Fault is
uncatchable, and the language has no exception type and no NaN or
infinity value. Open: words under a width for machine arithmetic,
generated for a finite range, the one code a limited type's name holds
(`U8.wrap 256` is 0, `U8.sat 300` is 255); bit operations on `Integer`.

## E3 — Effects, state, laziness, parallelism, async

A direction, not a rule; nothing here becomes syntax or a type in the
core. Purity is one static bit per scheme ([checks](07-checks.md)
§Phases). Mutation is never in the language; state is a host resource.
Evaluation is strict; delay is `() => …`; a host `Stream<T>` is pulled on
demand. `map f` with pure `f` may run in parallel without syntax; impure
stages stay in order. No coloring: a host call is the host's to perform,
as a syscall is the kernel's. Open: whether the run blocks in the host or
suspends until its reply (the VM suspends,
[embedding](../api/09-embed.md); the native backend blocks on its
host's reply); a run left waiting on a reply that never
comes, and cancelling it; asynchronous hosts and threads, and with them
`Task<T>`. `Stream`, `Task` and `Cell` are declared by the host (09).

## P1 — Prelude breadth (from the dogfood scripts)

Ten everyday scripts (`Documentation/dogfood/`) wrote a `(n, acc)`
`fold` wherever a line or cell is numbered, and tested a substring with
`len (split pat l) > 1`. **Proposed, not ruled:** `enumerate` (a
`Vec<a>` to `Vec<Integer * a>`), `contains`, `starts_with`,
`ends_with`, `replace`, `zip`, `drop`, `last`, `nth`, `reverse`, `min`,
`max`, `any`, `all`, each a prelude declaration with its doc line, no
new syntax; and a `:prelude` listing in the REPL. A tuple part is read
by a pattern only; **proposed:** no projection syntax, the pattern form
stated in [types](03-types.md) §Tuples with an example.

## H1 — Host and backend observables

Ruled 2026-09-29: `use` runs a module once where it stands and a cycle is a
check error, and the compile budget and the strict session are host inputs
([host](09-host.md)). Ruled 2026-10-12: the budget is
optional, its unit is the backend's, and where it stops a run is not an
observation ([host](09-host.md) §Budget); the compile budget defaults to
2^20 steps. Open: a unit common to every
backend; limits on memory or time; host nondeterminism (a reply that
differs between runs). Ruled 2026-09-30: the lexer has no
bound on a number ([lexical](01-lexical.md) §Numbers). Ruled 2026-10-21:
module identity, closure capture of the environment, cleanup failure,
reply validation and codec failure ([host](09-host.md)); cancellation is
E3. Every
host-declared type admits the codec cast (ruled 2026-09-30). No Reovim
paths, wire hashes or placeholder budget numbers become language constants.
Tests replay one host response/effect trace on each backend. From the
dogfood scripts, open for the std host's contract
(`tools/host/README.md`): an `Entry` field telling a directory from a
file, a directory's size as its contents', and a modification time or
a pinned clock so an `age` test can be golden; a `String` reads as a
`Sink` by its path, so a shadowed `out` writes a file named by the
data, which the contract states and the tour warns about.