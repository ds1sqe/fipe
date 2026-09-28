# Types and checking

Checking precedes all program effects. The following judgments constrain the
checker; T1–T3 and E2 in [open decisions](open.md) identify remaining choices.
They are not a proof of soundness or a fully specified inference algorithm.

## Type forms

```text
τ ::= () | Bool | i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64
    | f32 | f64 | [τ; n] | Vec<τ> | Deque<τ>
    | C<τ…> | τ -> τ | α | dyn Trait
σ ::= ∀α₁…αₙ. Q ⇒ τ
```

`C` names a declared nominal type, `α` an inference variable, and `Q` a set of
trait obligations. `dyn Trait` is semantic notation; source writes the trait
name itself. An API reference includes the host-resolved version. Every
function is curried: an arm with two head patterns has type `A -> B -> C`,
which is `A -> (B -> C)`. A zero-parameter function is `() -> T` and takes
the unit value.

Declared types are nominal: equal shape is not equal type. `type Name {
members }` declares either a record, whose lowercase fields are its
constructor's spots in order, or a sum, whose uppercase members are its
constructors; a record with one field is the newtype. Value types are
nonrecursive; recursive functions remain possible. Whether a function type
may make a recursive value type indirect is part of T3. `String` is the
prelude's `type String { bytes: Vec<u8> }` with a private field and valid
UTF-8; `Bool` is `type Bool { True, False }`. Decoded/raw string literals
have type `String`; `b"…"` has type `Vec<u8>`.

Integers have the written signedness and width. No ordinary subtyping widens
a number. Integer literals default to `i64`; floating literals to `f64` when
no expected type fixes their width. An expected numeric type checks the
literal against that type directly. Out-of-range literals are check errors.

The language names no resource type. A host that has resources declares an
opaque nominal type for them through the declaration interface, with the
functions that establish or refine one; the conformance host calls the type
`Rd` and `Rd<api>` and the function `Rd`. The language owns no mandatory
`sys` API, handle numbering or wire layout.

## Core judgments

Fresh variables in each instantiation are distinct from all surrounding ones.
`ftv` means free type variables; substitution applies to `Γ`, `τ` and `Q`.

```text
Γ(x) = ∀α. Q ⇒ τ
────────────────────────────────── VAR
Γ ⊢ x : τ[α := βfresh] ▷ Q[α := βfresh]

Γ, x : α ⊢ e : β ▷ Q
────────────────────────────────── ARM
Γ ⊢ (x => e) : α -> β ▷ Q

Γ ⊢ f : α -> β ▷ Q₁    Γ ⊢ x : α ▷ Q₂
────────────────────────────────── APP
Γ ⊢ f x : β ▷ Q₁ ∪ Q₂

Γ ⊢ e : τ ▷ Q    σ = ∀(ftv(τ,Q) − ftv(Γ)). Q ⇒ τ
Γ, x : σ ⊢ body : υ ▷ R
────────────────────────────────── BIND
Γ ⊢ x = e; body : υ ▷ R
```

The semicolon in BIND is metalanguage sequence, not Fipe syntax. A head with
several patterns is nested ARMs; an arm list with several arms shares one
input and result type per position. Actual command application first binds
parameter spots under [evaluation](evaluation.md). Then APP applies, possibly
after inserting a checked cast at a known spot. T2 governs generalization for
recursive groups, unresolved constraints and host effects. The equations do
not authorize independently casting unknown variables to avoid a unification
failure. `name<τ…>` instantiates the declared type parameters of `name` in
order, the target first for the conversion words; a bare line whose target
is still unknown is an ambiguity error.

## Fields, functions and branches

- If `Γ ⊢ e : S` and record `S` declares `field : T`, then `Γ ⊢ e.field :
  T`. An absent field, or any field of a sum value, is a check error; a sum's
  payload is reached by pattern. There is no runtime string-based field
  lookup. `.field` is a function using that same rule. `e.f a` where `f` is
  not a field is `f` with `e` in its `self` spot, as [embedding](host.md)
  orders the lookup.
- Arithmetic requires one numeric type and its `Num` obligation. Comparison
  requires `Eq` or `Ord`; function equality is not defined.
- An `if` condition is `Bool`; both arms have one result type. With no `else`,
  the omitted arm is `()` and the written arm must have that type.
- Arms have common input types and one result type. Patterns are checked
  against those inputs; guards have type `Bool`. The arm set must be
  exhaustive. A guard alone cannot establish coverage; the first matching
  true guard wins. `(name = v)` gives the parameter `v`'s type and a default.
- Captures are values. Shadowing introduces a new binding; it never mutates
  an existing capture. A discarded non-unit value in a body is a check error;
  `_ = e` explicitly discards it.
- The case rule at declarations: a declared type, constructor or associated
  type is uppercase; a binding, field, parameter or method is not. A
  file-level binding whose value is an arm list is collected with the items
  and may recurse; every other binding is visible after its line.

## Constructors, privacy and `Self`

A `type` declaration puts one function per constructor in the value
namespace under the constructor's name, or under the type's name for a
record; nothing else does, so `i64`, `Vec` and a host's opaque type have no
value unless a function is declared for them. Types and values are separate
namespaces. `Point { x: 1, y: 2 }` is the raw construction; the constructor
`Point 1 2` is the same function applied, unless the type's module writes one.

Bare arms in `impl T { }` are `T`'s written constructor, joined in order into
one arm list; `T` then names that function, its signature is inferred from
the record it builds, and its visibility is the type's. A sum has no written
constructor. Inside `type`, `trait` and `impl` bodies `Self` names the
enclosing type and, in value position, its constructor. An inherent or trait
impl is written only in its target's module; one definition per (type,
name), and a name that is both inherent and trait-supplied is a check error.

`pub type` exports the type. A private field is readable, matchable and
suppliable only inside its module; outside it is not a spot and takes its
default, so the derived constructor exists outside exactly when every private
field defaults. `String`'s field has no default, so `utf8` is the only way in.
A `pub` binding or member needs a complete type (T3); a `pub` on an
incomplete one is a check error.

## Traits

There is one implementation per (trait, arguments, concrete type), written
in the type's module. A trait name in a parameter position accepts a value
implementing it and uses runtime dispatch. Trait methods are functions in
scope; `T.m` names `T`'s member `m` and `x.m` fills its `self` spot. An API
method and trait method with the same applicable name are ambiguous, which
is a check error. Trait obligations from operators can be inferred. A trait
member `name: type = value` is a default method; an uppercase member with a
bound, `Error: Err`, is an associated type, reached as `Self.Error`.

The checker derives `Eq`, `Ord`, `Show`, and `Num` where structurally valid;
users cannot supply their implementations. `Num` applies only to numeric
types. `Display`, `Read`, `From<A>`, `Into<B>`, `TryFrom<A>`, `TryInto<B>`,
`Err`, `Functor`, `Monad`, and `Alternative` are declared traits. T3 must
specify their complete signatures, higher-kinded variables, derivation rules
and dispatch restrictions before the checker is conforming.

## Defaults and declarations

| Type | Type default |
| --- | --- |
| `()` | `()` |
| `Bool` | `False` |
| integers, floats, `String`, function, `Either`, sum, host types | none |
| `Vec<T>`, `Deque<T>` | empty |
| `[T; n]` | `n` copies of `T`'s default, if it has one |
| record | its constructor applied to nothing, if every spot defaults |
| `Maybe<T>` | `Nothing` |

A declared `= value` overrides the type default for that spot, and may name
the spots declared before it. Duplicate parameter names or ambiguous Bool
shorthand letters are check errors. No parameter is `Maybe` or `Result`;
defaults model optional arguments. A host boundary type has no unresolved
type variable and no function value. T3 specifies default-expression limits.

## Conversion

Define `Cast(A, B)` as an admitted conversion pair. Expected parameter types
and annotations use this closed relation; nothing else fires at a spot. A
missing pair is a check error; a permitted conversion that fails on a value
is a Fault. A statically known invalid literal is rejected during checking.

| Spot | Admitted source | On failure |
| --- | --- | --- |
| number | number | exact, or Fault; E2 owns rounding and ranges |
| `T: Read` | word or `String` | Fault |
| `String`, `Vec<u8>` | any `T` via `Display`, else derived `Show` | never |
| host-declared type | `Vec<u8>` via the host codec | Fault |
| `Vec<u8>` | `String` | never |
| trait | an implementing type | never |

`Vec<u8>` → `Vec<u8>` is identity, not formatting. Host formatting or codec
failure is a Fault and never triggers a different fallback formatter. Pairs
do not chain, and a type name is never a conversion: `pl | Point` is
`Point pl`, the constructor. The codec and formatting contracts are H1.

An outcome type (`Maybe`, `Result`, `Either`) never meets a `String` or
`Vec<u8>` spot by implicit cast, even where `Show` would derive; `show m` is
the explicit form. This keeps `find x | put out` a check error until `||`
or `|:` has removed the `Maybe`.

Every other conversion is a written word. To and from text and numbers the
prelude has `show : a -> String`, `read : Read a => String -> Result<a,
ParseError>`, `bytes`, `utf8 : Vec<u8> -> Result<String, Utf8Error>` and
`round`, `trunc`, `floor`, `ceil : Float a => a -> a`. Type to type is one
of four traits, and **the word names the trait, the trait names the side
that does the work**: `From<A>` and `TryFrom<A>` on the target, which builds
itself; `Into<B>` and `TryInto<B>` on the source, which converts itself.

```text
trait From<A>    { from: A -> Self }
trait Into<B>    { into: Self -> B }
trait TryFrom<A> { Error: Err, try_from: A -> Result<Self, Self.Error> }
trait TryInto<B> { Error: Err, try_into: Self -> Result<B, Self.Error> }

pl | Point.from     pl.into<Point>     "3,4" | Point.try_from     pl.try_into<Point>
pl | to<Point>      "3,4" | try<Point>
```

One impl per direction across the four traits; a second, on either side or
of either kind, is a check error naming the first. The side-named words are
exact. The neutral `to<T>` calls a total impl, or a failable one and Faults
on `Err`; `try<T>` calls a failable impl, or a total one wrapped in `Ok`. The
target comes from `<T>` or from the spot. Text stays with `Display` and
`Read`: `Read` is what a word at a spot uses, and `show`, not `Into<String>`,
is how a value prints.

## Outcome types

`Either<L,R>` has `Left L` / `Right R`; `Maybe<T>` has `Nothing` / `Just T`;
`Result<T,E>` has `Err E` / `Ok T`, with `E : Err`. Their payload behavior is
specified by [evaluation](evaluation.md). T1 must decide their type identity:
ordinary transparent aliases cannot have different operator eligibility.

An outcome bracket names a side, not a family. Against `Maybe<T>`, `[:e]`
binds `e : T` and `[!]` is `Nothing`; against `Result<T,E>`, `[:e]` binds
`e : T` and `[!e]` binds `e : E`. `[!e]` against `Maybe` is a check error,
since `Nothing` has no payload. In construction the expected type supplies
the family; with none (`m = [:1]`) it is a check error, "outcome family
unknown", with no default. `Just`, `Nothing`, `Ok` and `Err` remain ordinary
prelude constructors and are the spelling that fixes a family. `Either` has
no bracket form, as it has no `|:`.
