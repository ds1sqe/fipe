# Types and checking

Checking precedes all program effects. The following judgments constrain the
checker; T1–T3 and E2 in [open decisions](open.md) identify remaining choices.
They are not a proof of soundness or a fully specified inference algorithm.

## Type forms

```text
τ ::= () | Bool | i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64
    | f32 | f64 | [τ; n] | Vec<τ> | Deque<τ>
    | C<τ…> | Rd | Rd<api> | fn(τ) -> τ | α | dyn Trait
σ ::= ∀α₁…αₙ. Q ⇒ τ
```

`C` names a declared nominal type, `α` an inference variable, and `Q` a set of
trait obligations. `dyn Trait` is semantic notation; source writes the trait
name itself. An API reference includes the host-resolved version. A function
with several parameters curries: `fn(A, B) -> C = fn(A) -> fn(B) -> C`.
Zero-argument functions use a separate arity-zero call, subject to S3.

Structs, newtypes and enums are nominal: equal shape is not equal type. Value
types are nonrecursive; recursive functions remain possible. Whether a
function type may make a recursive value type indirect is part of T3.
`String` is the prelude newtype over `Vec<u8>` whose bytes are valid UTF-8.
Decoded/raw string literals have type `String`; `b"…"` has type `Vec<u8>`.
This selects the later Literal table over the older prose saying all quotes
are bytes. Lexical structure specifies validation of string escapes.

Integers have the written signedness and width. No ordinary subtyping widens
a number. Integer literals default to `i64`; floating literals to `f64` when
no expected type fixes their width. An expected numeric type checks the
literal against that type directly. Out-of-range literals are check errors.

`Rd` is a language resource identifier, never an owned operating-system
handle. The host supplies the meaning of an API and any dynamic check that
establishes `Rd<api>`. The language owns no mandatory `sys` API or wire layout.

## Core judgments

Fresh variables in each instantiation are distinct from all surrounding ones.
`ftv` means free type variables; substitution applies to `Γ`, `τ` and `Q`.

```text
Γ(x) = ∀α. Q ⇒ τ
────────────────────────────────── VAR
Γ ⊢ x : τ[α := βfresh] ▷ Q[α := βfresh]

Γ, x : α ⊢ e : β ▷ Q
────────────────────────────────── FN
Γ ⊢ fn(x) { e } : fn(α) -> β ▷ Q

Γ ⊢ f : fn(α) -> β ▷ Q₁    Γ ⊢ x : α ▷ Q₂
────────────────────────────────── APP
Γ ⊢ f x : β ▷ Q₁ ∪ Q₂

Γ ⊢ e : τ ▷ Q    σ = ∀(ftv(τ,Q) − ftv(Γ)). Q ⇒ τ
Γ, x : σ ⊢ body : υ ▷ R
────────────────────────────────── LET
Γ ⊢ let x = e; body : υ ▷ R
```

The semicolon in LET is metalanguage sequence, not Fipe syntax. Actual command
application first binds parameter spots under [evaluation](evaluation.md).
Then APP applies, possibly after inserting a checked cast at a known spot.
T2 governs generalization for recursive groups, unresolved constraints and
host effects. The equations do not authorize independently casting unknown
variables to avoid a unification failure.

## Fields, functions and branches

- If `Γ ⊢ e : S` and struct `S` declares `field : T`, then
  `Γ ⊢ e.field : T`. An absent field is a check error. There is no runtime
  string-based field lookup. `.field` is a function using that same rule.
- Arithmetic requires one numeric type and its `Num` obligation. Comparison
  requires `Eq` or `Ord`; function equality is not defined.
- An `if` condition is `Bool`; both arms have one result type. With no `else`,
  the omitted arm is `()` and the written arm must have that type.
- Arms have a common input type and result type. Patterns are checked against
  that input; guards have type `Bool`. The arm set must be exhaustive.
  A guard alone cannot establish coverage; the first matching true guard wins.
- Captures are values. Shadowing introduces a new binding; it never mutates
  an existing capture. A discarded non-unit value in a body is a check error;
  `let _ = e` explicitly discards it.

## Traits

There is one implementation per `(trait, concrete type)`. A trait name in a
parameter position accepts a value implementing it and uses runtime dispatch.
An API method and trait method with the same applicable name are ambiguous,
which is a check error. Trait obligations from operators can be inferred.

The checker derives `Eq`, `Ord`, `Show`, and `Num` where structurally valid;
users cannot supply their implementations. `Num` applies only to numeric
types. `Display`, `Err`, `Functor`, `Monad`, and `Alternative` are declared
traits. T3 must specify their complete signatures, higher-kinded variables,
derivation rules and dispatch restrictions before the checker is conforming.

## Defaults and declarations

| Type | Type default |
| --- | --- |
| `()` | `()` |
| `Bool` | `False` |
| integers, floats, `String`, function, `Rd`, `Either`, enum | none |
| `Vec<T>`, `Deque<T>` | empty |
| `[T; n]` | `n` copies of `T`'s default, if it has one |
| record struct | field defaults, if every field has one |
| `Maybe<T>` | `Nothing` |
| newtype | its written `Default`, if present |

A declared `= value` overrides the type default for that parameter. Duplicate
parameter names or ambiguous Bool shorthand letters are check errors. No
parameter is `Maybe` or `Result`; defaults model optional arguments. A host
boundary type has no unresolved type variable and no function value.
T3 specifies exported signature completeness and default-expression limits.

## Casts

Define `Cast(A, B)` as an admitted conversion pair. Expected parameter types,
annotations, explicit `as B` and host sinks use the same relation. A missing
pair is a check error; a permitted conversion that fails on a value is a
Fault. A statically known invalid literal is rejected during checking.

| Source → target | Required behavior |
| --- | --- |
| number → number | checked conversion; E2 owns exact rounding/ranges |
| word or `String` → readable `T` | interpret the complete text as `T` |
| `T` → `String` or `Vec<u8>` | declared `Display`, otherwise derived `Show` |
| `Vec<u8>` → host-declared type | host codec consumes exactly one value |
| `String` → `Vec<u8>` | preserve the UTF-8 bytes |
| `Rd` → `Rd<api>` | host verifies the target offers the resolved API |
| concrete type → implementing trait | pack value with its trait dictionary |

`Vec<u8>` → `Vec<u8>` is identity, not formatting. Host formatting or codec
failure is a Fault and never triggers a different fallback formatter. A
same-layout cast between nominal host types requires explicit host evidence;
shape alone is not a type equality. The codec and formatting contracts are H1.

## Outcome types

`Either<L,R>` has `Left L` / `Right R`; `Maybe<T>` has `Nothing` / `Just T`;
`Result<T,E>` has `Err E` / `Ok T`, with `E : Err`. Their payload behavior is
specified by [evaluation](evaluation.md). T1 must decide their type identity:
ordinary transparent aliases cannot have different operator eligibility.
