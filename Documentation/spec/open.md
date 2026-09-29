# Open specification decisions

These are proposals for review, not approved behavior. Examples distinguish
choices; an implementation must not fill a gap without updating the owning
rule. T/E/H concern checking and execution. Lexical choices are defined by
[lexical structure](lexical.md). The user settled parser choices
S1–S3 on 2026-09-27 and T1–T2 on 2026-09-29; [syntax](syntax.md) and
[types](types.md) own their rules.

## T1 — Outcome identity (ruled)

Three nominal prelude types with per-type operator eligibility;
[types](types.md) §Outcome types. Whether a backend shares their layout
is E2/H1's.

## T2 — Generalization, effects and recursion (ruled)

[Types](types.md) §Core judgments, "Generalization". Effects are not
typed in this draft; a host call is typed by its declared signature.

## T3 — Traits and declarations

`Self<A>`, trait-as-type dispatch, inferred Bool arguments and the
associated type `Error` need exact checking rules. **Proposed:** a `pub`
binding must have a complete type, written or inferred monomorphic; keep
written type parameters in the prelude only, except that ordinary source
implements the parameterized prelude traits `From<A>`, `Into<B>`,
`TryFrom<A>` and `TryInto<B>`. Coherence keys on (trait, arguments, type)
under the "`Self` is local" orphan rule, with at most one conversion impl
per direction across the four traits. Fully define trait kinds, signatures,
derived instances, default-expression evaluation and termination of
recursive type detection before the checker milestone; hoisting, the case
rule and name lookup are ruled and implemented by resolution.

## E1 — Evaluation and outcome rebuilding

**Proposed:** strict left-to-right evaluation, each supplied expression once,
defaults in parameter order. When a marked pipe's return could be either a
payload or the same outcome family, prefer the latter; use an explicit
constructor for nesting. Keep Bool to `||`/`&&` until `Bool |:`/`|!` has a
defined payload. Define failing non-success patterns before admitting them.
Test cases: `Just 1 |: (_ => Just 2)` yields `Just 2`; an inactive branch does
not call the host; `[x] = []` must have a defined rejection or outcome.

## E2 — Arithmetic

**Proposed:** integers divide toward zero; remainder has the dividend's sign;
float operations use their declared IEEE binary width and ties-to-even;
division by zero and overflow Fault. Specify NaN/infinity introduction,
comparison, float equality, signed zero, casts and stable formatting before
backend equivalence. The current cast example requires `1.5 as i64` to Fault.

## H1 — Host and backend observables

**Proposed:** identify modules by host-provided stable identity, reject import
cycles, and fix a semantic charging schedule shared by backends. Specify
closure environment capture, cancellation, cleanup failure, external byte
charging, declaration validation and codec/formatter failure in the embedding
contract. No Reovim paths, wire hashes or placeholder budget numbers become
language constants. Tests replay one host response/effect trace on each backend.
