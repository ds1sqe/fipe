# Open specification decisions

These are proposals for review, not approved behavior. Examples distinguish
choices; an implementation must not fill a gap without updating the owning
rule. S1–S3 concern the parser; T/E/H concern checking and execution.
Plan 01.0's lexical choices are defined by [lexical structure](lexical.md).

## S1 — Precedence, newlines and stages

The design only says pipes are loosest. **Proposed:** the complete table and
stage selection in [syntax](syntax.md), with all five pipes left-associative
at one level. `a || b && c` means `(a || b) && c`; parenthesize to change it.
Ignore newlines in grouping and type/field lists; keep them in block bodies;
allow a newline before `else`. An arm's body includes its remaining pipeline.
Alternative: conventional `&&` tighter than `||`, with another pipe level.

## S2 — Slash and greater-than context

`/` is both a root path and division; `>` both comparison and redirect.
**Proposed:** `/` remains one symbol token, usable as a root-path operand or
infix division by position. `>` at outer line level redirects; write `(x > y)`
for an outer value comparison. It compares inside a group, guard or if head.
`cat x > y` redirects; `if x > y { x }` compares. In types, parser-local
splitting makes `Vec<Vec<u8>>` close twice; elsewhere `>>` redirects.
Alternative: consult bound names/types to decide comparison versus redirect.

## S3 — Parenthesized calls and selectors

Space application coexists in examples with `fs.Entry(a,b)` and `x.m()`.
**Proposed:** adjacent parentheses are an explicit argument-list call for any
callable, not only constructors/methods. `f(a,b)` and `f a b` supply the same
spots; `f()` supplies no arguments and invokes defaults/arity-zero behavior.
`f (x)` is space application to a group. Add no tuple syntax in this step.
The candidate EBNF currently omits this extension until approved. Tuple fields
such as `.0` and selecting a field on `(e).name` need the same postfix ruling.

## T1 — Outcome identity

Transparent `Maybe<T> = Either<(),T>` / `Result<T,E> = Either<E,T>` aliases
erase which names the checker saw, yet `Either` refuses `||` and `&&` while
the other two allow them. **Proposed:** nominal prelude types with one shared
two-sided representation; explicit conversions preserve constructor meaning.
Alternative: introduce a specified alias/trait mechanism that retains identity.
Discriminating case: a value typed `Either<(),i64>` must refuse `|| 0`, even
when its runtime representation equals `Just 1`.

## T2 — Generalization, effects and recursion

“Haskell's type system” does not define Fipe's inference under host effects,
casts and defaults. **Proposed:** generalize solved nonrecursive immutable
bindings; infer recursive function groups monomorphically then generalize at
the group boundary. Casts require a known target and do not manufacture a
type to resolve ambiguity. Specify effectful/resource generalization and
ambiguous trait defaulting before checking polymorphic host calls.

## T3 — Traits and declarations

`Self<A>`, trait-as-type dispatch, inferred Bool arguments and user aliases
need exact checking rules. **Proposed:** export only signatures with every
parameter and result annotated, except a receiver fixed by its enclosing impl;
keep written type parameters in the prelude only. Fully define trait kinds,
signatures, coherence, derived instances, default-expression evaluation and
termination of recursive type detection before the checker milestone.

## E1 — Evaluation and outcome rebuilding

**Proposed:** strict left-to-right evaluation, each supplied expression once,
defaults in parameter order. When a marked pipe's return could be either a
payload or the same outcome family, prefer the latter; use an explicit
constructor for nesting. Keep Bool to `||`/`&&` until `Bool |:`/`|!` has a
defined payload. Define failing non-success patterns before admitting them.
Test cases: `Just 1 |: (_ => Just 2)` yields `Just 2`; an inactive branch does
not call the host; `let [x] = []` must have a defined rejection or outcome.

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
