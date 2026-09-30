# Open specification decisions

These are proposals for review, not approved behavior. Examples distinguish
choices; an implementation must not fill a gap without updating the owning
rule. T/E/H concern checking and execution. Lexical choices are defined by
[lexical structure](lexical.md). The user settled parser choices
S1–S3 on 2026-09-27 and T1–T3 on 2026-09-29; [syntax](syntax.md),
[types](types.md), [containers](containers.md), [limits](limits.md),
[checks](checks.md) and [members](members.md) own their rules.

## T1 — Outcome identity (ruled)

Three nominal prelude types with per-type operator eligibility;
[types](types.md) §Outcome types. Whether a backend shares their layout
is E2/H1's.

## T2 — Generalization, effects and recursion (ruled)

[Types](types.md) §Core judgments, "Generalization". Effects are not
typed in this draft; a host call is typed by its declared signature.
Purity for the compile phase is [checks](checks.md) §Phases.

## T3 — Declarations

A trait is a type (ruled 2026-09-29), and every operation lives in a
type: [types](types.md), [limits](limits.md), [checks](checks.md) and
[members](members.md). Still open from T3: `Self<A>`, inferred Bool
arguments, default-expression evaluation and termination of recursive
type detection. **Proposed:** a `pub` binding must have a complete type,
written or inferred monomorphic.

Ruled 2026-09-30: a host type that takes an API reference declares its
parameter uppercase, `type Rd<Api>`; the use site `Rd<fs>` stays
([types](types.md) §Type forms).

## T4 — Limits and checks

- A range notation for what the REPL prints, such as `Integer 0..127`.
- Bit order inside a record held by `Bits` and `Size`.
- A limited type that declares the operations it is closed under.
- May literal patterns under a limit complete an arm set, as `0` and `1`
  under `Bit`? Not yet.
- A default for a tuple: the tuple of its parts' defaults, when each has
  one.
- Arithmetic on number parameters beyond a sum.
- What is known of a float: a small set of values, as 2.0 or 2.5.
- A local strict scope for a program's own code, if one is wanted.
- May a limit read a value known at init when it says `Init`?
- Whether the strict session reaches the prelude.
- The compile budget's default.
- Is a value evaluated on an earlier session line exact on a later line?
- Whether a tuple takes a trailing comma, as a list does; its grammar
  has none.
- Whether a line continues after a trailing `&` in a type.
- A hatch for hardware, in design: a trusted entry, and the holdings
  `Volatile`, `Pinned`, `At` and `Order`, byte order, offsets and
  addresses.

## T5 — Members

- Whether a word and a path become forms. Today a word is read by its spot
  through `Read`.
- The member of `Items<T>` ([containers](containers.md) §Brackets).
  **Proposed:** `type Items<T> where self.fold: (b -> T -> b) -> b -> b`.
- A bracket pattern for a program's own type; arrays and `Vec` only now.
- An identity for empty reductions, `Zero` with `Self.zero`, so that
  `sum []` has an answer. Until then `sum` needs `Number`.
- The defaulting list beyond `Literal`: which other member limits a
  defaulted variable may carry. Ruled 2026-09-30: a variable with a
  `Literal` limit defaults, `Integer` first and `F64` next
  ([types](types.md) §Core judgments).
- `Float` holds `round`; `trunc`, `floor` and `ceil` are not in it yet.
- Marked pipes on a user meaning through `then`, `pure`, `or`, `empty`.
- A checked claim, `impl Circle: Shape`, if accidental matches bite.

## E1 — Evaluation and outcome rebuilding

**Proposed:** strict left-to-right evaluation, each supplied expression once,
defaults in parameter order. When a marked pipe's return could be either a
payload or the same outcome family, prefer the latter; use an explicit
constructor for nesting. Keep Bool to `||`/`&&` until `Bool |:`/`|!` has a
defined payload. Define failing non-success patterns before admitting them.
Test cases: `Just 1 |: (_ => Just 2)` yields `Just 2`; an inactive branch does
not call the host; `[x] = []` must have a defined rejection or outcome.

## E2 — Arithmetic

`Integer` arithmetic is ruled in [evaluation](evaluation.md) §Arithmetic
and the number row in [members](members.md) §Conversions. **Proposed:**
float operations use their declared IEEE binary width and ties-to-even;
float division by zero and overflow Fault. Specify NaN/infinity
introduction, comparison, float equality, signed zero and stable
formatting before backend equivalence. Also open: words under a width for
machine arithmetic, generated for a finite range, the one code a limited
type's name holds: `U8.wrap 256` is 0, `U8.sat 300` is 255; bit
operations on `Integer`; an exact `Rational` or `Decimal` meaning.

## H1 — Host and backend observables

Ruled 2026-09-29: `use` runs a module once where it stands and a cycle is
a check error, and the compile budget and the strict session are host
inputs ([host](host.md)). **Proposed:** identify modules by host-provided
stable identity and fix a semantic charging schedule shared by backends,
including `Integer` by operand size. Ruled 2026-09-30: the lexer has no
bound on a number ([lexical](lexical.md) §Numbers). Specify closure environment capture,
cancellation, cleanup failure, external byte charging, declaration
validation and codec/formatter failure in the embedding contract. No
Reovim paths, wire hashes or placeholder budget numbers become language
constants. Tests replay one host response/effect trace on each backend.
