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

## T3 — Declarations

A trait is a type (ruled 2026-09-29), and every operation lives in a
type: [types](03-types.md), [limits](06-limits.md), [checks](07-checks.md) and
[members](04-members.md). Still open from T3: `Self<A>`, inferred Bool
arguments, default-expression evaluation and termination of recursive
type detection.

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
- The compile budget's default.
- Is a value evaluated on an earlier session line exact on a later line?
- A hatch for hardware, in design: a trusted entry, and the holdings
  `Volatile`, `Pinned`, `At` and `Order`, byte order, offsets and
  addresses.

## T5 — Members

- `Items<T>` is `fold` (ruled 2026-09-30, [containers](05-containers.md)
  §Brackets). Open, for a later stage: a walk that stops early, and one
  held by a front and a back position, from either end.
- A bracket pattern for a program's own type; arrays and `Vec` only now.
- Marked pipes on a user meaning through `then`, `pure`, `or`, `empty`.

## E1 — Evaluation and outcome rebuilding

**Proposed:** strict left-to-right evaluation, each supplied expression once,
defaults in parameter order. When a marked pipe's return could be either a
payload or the same outcome family, prefer the latter; use an explicit
constructor for nesting. Keep Bool to `||`/`&&` until `Bool |:`/`|!` has a
defined payload. Define failing non-success patterns before admitting them.
Test cases: `Just 1 |: (_ => Just 2)` yields `Just 2`; an inactive branch does
not call the host; `[x] = []` must have a defined rejection or outcome.

## E2 — Arithmetic

`Integer` arithmetic is ruled in [evaluation](08-evaluation.md) §Arithmetic
and the number row in [members](04-members.md) §Conversions. **Proposed:**
float operations use their declared IEEE binary width and ties-to-even;
float division by zero and overflow Fault. Specify NaN/infinity
introduction, comparison, float equality, signed zero and stable
formatting before backend equivalence. Also open: words under a width for
machine arithmetic, generated for a finite range, the one code a limited
type's name holds: `U8.wrap 256` is 0, `U8.sat 300` is 255; bit
operations on `Integer`.

## H1 — Host and backend observables

Ruled 2026-09-29: `use` runs a module once where it stands and a cycle is a
check error, and the compile budget and the strict session are host inputs
([host](09-host.md)). **Proposed:** identify modules by host-provided stable
identity and fix a semantic charging schedule shared by backends, including
`Integer` by operand size. Ruled 2026-09-30: the lexer has no bound on a
number ([lexical](01-lexical.md) §Numbers). Specify closure environment
capture, cancellation, cleanup failure, external byte charging, declaration
validation and codec/formatter failure in the embedding contract. No Reovim
paths, wire hashes or placeholder budget numbers become language constants.
Tests replay one host response/effect trace on each backend.