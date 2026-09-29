# Types and checking

Checking precedes all program effects. T3 and E2 in
[open decisions](open.md) name the remaining choices.

## Type forms

```text
τ ::= () | Bool | i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64
    | f32 | f64 | [τ; n] | Vec<τ> | Deque<τ>
    | C<τ…> | τ -> τ | α | dyn Trait
σ ::= ∀α₁…αₙ. Q ⇒ τ
```

`C` names a declared nominal type, `α` an inference variable, `Q` a set of
trait obligations; `dyn Trait` is notation, source writes the trait name.
An API reference includes the host-resolved version. An arm with n head
patterns has an n-ary curried type; a zero-parameter function is `() -> T`.

Declared types are nominal. `type Name { members }` is a record, whose
lowercase fields are its constructor's spots in order, or a sum, whose
uppercase members are its constructors; a one-field record is the newtype.
Value types are nonrecursive; whether a function type may make one indirect
is T3's. `String` is the prelude's `type String { bytes: Vec<u8> }` with a
private field and valid UTF-8; `Bool` is `type Bool { True, False }`.
String literals have type `String`, `b"…"` type `Vec<u8>`.

Integers have the written signedness and width and never widen. Integer
literals default to `i64` and float literals to `f64` unless an expected
type fixes them; an out-of-range literal is a check error. The language
names no resource type; [embedding](host.md) says how a host declares one.

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

The semicolon in BIND is metalanguage. A head with several patterns is
nested ARMs; an arm list shares one input and result type per position.
Application first binds spots under [evaluation](evaluation.md), then APP
applies, possibly after a checked cast at a known spot; inference only
unifies, and a cast is admitted only at a spot whose declared type is
known. `name<τ…>` instantiates `name`'s declared type parameters in
order, the target first for the conversion words; a line whose target is
still unknown is an ambiguity error.

**Generalization.** Every binding generalizes over the variables not free
in the environment and keeps its unsolved obligations; there is no
mutation, so no value restriction, and a host call in a value does not
block it. An arm binding may call itself, and the members of one `impl`
may call each other: such a group is typed monomorphically and
generalized at its boundary, so there is no polymorphic recursion. An
annotation's lowercase names are rigid variables and its value is checked
against it; a
`pub` binding ends closed, with no unsolved variable and no quantified
variable it did not write. A variable constrained only by literals defaults
to `i64` or `f64` at its binding, so `x = 1` is `i64`; a variable that
leaves the type with a `Num`, `Float` or literal obligation and otherwise
at most `Eq` and `Ord` defaults the same way where it leaves, so `even 4`
is `Bool`; any other obligation whose variable left the type is the error
"ambiguous". Where an expected type is known, from an annotation, a spot or
a sibling arm, inference checks against it; a dot on a head whose type is
not yet known is an obligation, `a.name`, solved field first and then
`self`-spot function once the head's type is known, carried by the scheme
until then, and the error at the dot if it never is.

## Fields, functions and branches

- If `Γ ⊢ e : S` and record `S` declares `field : T`, then `e.field : T`;
  `.field` is the same function. An absent field, or any field of a sum,
  is a check error; a sum's payload is reached by pattern. `e.f a` where
  `f` is not a field is `f` with `e` in its `self` spot, in
  [embedding](host.md)'s lookup order.
- Arithmetic requires one numeric type and its `Num` obligation; comparison
  requires `Eq` or `Ord`; functions have no equality.
- An `if` condition is `Bool`; both arms have one result type, and a
  missing `else` is `()`.
- Arms have common input types and one result type; guards are `Bool`; the
  set is exhaustive, and a guard alone cannot establish coverage. The first
  matching arm with a true guard wins. `(name = v)` gives the parameter
  `v`'s type and a default.
- Captures are values; shadowing never mutates a capture. A discarded
  non-unit value in a body is a check error; `_ = e` discards. A file's
  top-level line is the implicit main's output, not a discard.
- The case rule at declarations: a declared type, constructor or associated
  type is uppercase; a binding, field, parameter or method is not. A
  file-level binding whose value is an arm list is collected with the items
  and may recurse; every other binding is visible after its line.

## Constructors, privacy and `Self`

A `type` puts one function per constructor in the value namespace, under
the constructor's name or, for a record, the type's; nothing else does, so
`i64`, `Vec` and an opaque host type have no value unless a function is
declared. Types and values are separate namespaces. `Point { x: 1, y: 2 }`
is the raw construction; `Point 1 2` is the same function applied, unless
the type's module writes one: bare arms in `impl T { }` are `T`'s written
constructor, joined in order, with its signature inferred from the record
it builds and the type's visibility. A sum has no written constructor.
Inside `type`, `trait` and `impl` bodies `Self` is the enclosing type and,
in value position, its constructor. In an impl member the parameter `self`
has type `Self`. An impl is written only in its target's module; one
definition per (type, name), and a name both inherent and trait-supplied
is a check error.

`pub type` exports the type. A private record field is readable,
matchable and suppliable only inside its module; outside it is not a spot
and takes its default, so the derived constructor exists outside exactly
when every private field defaults. A sum constructor's fields are its
payload and are always visible. `String`'s field has no default, so `utf8` is the
only way in. A `pub` binding or member needs a complete type (T3).

## Traits

One implementation per (trait, arguments, concrete type), written in the
type's module. A trait name in a parameter position accepts an implementing
value with runtime dispatch. Trait methods, and a type's inherent
members in its module, are functions in scope; `T.m` names `T`'s member
`m` and `x.m` fills its `self` spot. An API method and
trait method with the same applicable name are ambiguous, a check error.
Operators infer their obligations. A trait member `name: type = value` is a
default method; an uppercase member with a bound, `Error: Err`, is an
associated type, reached as `Self.Error`.

The checker derives `Eq`, `Ord`, `Show` and `Num` where structurally valid
and users cannot supply them; `Num` is numeric only. `Display`, `Read`,
`From<A>`, `Into<B>`, `TryFrom<A>`, `TryInto<B>`, `Err`, `Functor`, `Monad`
and `Alternative` are declared traits. T3 owns kinds, derivation and
dispatch restrictions.

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

A declaration's labeled parameters are its spot names and defaults, not
part of its type: `(long: Bool = False) -> Vec<Entry>` has type `Bool ->
Vec<Entry>`. An opaque type has no fields, constructors or derived
classes; a declaration of its own name, `Rd: u64 -> Rd<api>`, is its
constructor. In the prelude and host declarations an impl member with no
value is supplied by the implementation, as a name with no value is.
A declared `= value` overrides the type default, may name the spots
before it, and is checked against its spot's type. Duplicate parameter names and ambiguous Bool shorthand letters
are check errors. No parameter is `Maybe` or `Result`; defaults model
optional arguments. A host boundary type has no unresolved variable and no
function value. T3 limits default expressions.

## Conversion

`Cast(A, B)` is the closed set of conversions a spot or annotation admits;
nothing else fires at a spot. A missing pair is a check error, a failing
admitted one a Fault, a statically invalid literal a check error.
A word or path literal is text with no type of its own: at a spot whose
type is `String`, or a host-declared type that implements `Read`, it
takes that type; anywhere else it is `String`.

| Spot | Admitted source | On failure |
| --- | --- | --- |
| number | number | exact, or Fault; E2 owns rounding and ranges |
| `T: Read` | word or `String` | Fault |
| `String`, `Vec<u8>` | any `T` via `Display`, else derived `Show` | never |
| host-declared type | `Vec<u8>` via the host codec | Fault |
| `Vec<u8>` | `String` | never |
| trait | an implementing type | never |

`Vec<u8>` to `Vec<u8>` is identity. Pairs do not chain, a codec or
formatting failure never falls back, and a type name is never a conversion:
`pl | Point` is `Point pl`, the constructor. An outcome type never meets a
`String` or `Vec<u8>` spot implicitly, so `find x | put out` is a check
error until `||` or `|:` removes the `Maybe`.

Every other conversion is a word. Text and numbers: `show : a -> String`,
`read : Read a => String -> Result<a, ParseError>`, `bytes`, `utf8 :
Vec<u8> -> Result<String, Utf8Error>`, and `round`, `trunc`, `floor`,
`ceil : Float a => a -> a`. Type to type is one of four traits, and **the
word names the trait, the trait names the side that does the work**:
`From<A>` and `TryFrom<A>` on the target, `Into<B>` and `TryInto<B>` on
the source.

```text
trait From<A>    { from: A -> Self }
trait Into<B>    { into: Self -> B }
trait TryFrom<A> { Error: Err, try_from: A -> Result<Self, Self.Error> }
trait TryInto<B> { Error: Err, try_into: Self -> Result<B, Self.Error> }

pl | Point.from     pl.into<Point>     "3,4" | Point.try_from     pl.try_into<Point>
pl | to<Point>      "3,4" | try<Point>
```

One impl per direction across the four; a second is a check error naming
the first. The side-named words are exact. `to<T>` calls a total impl, or a
failable one and Faults on `Err`; `try<T>` calls a failable impl, or a total
one wrapped in `Ok`; the target comes from `<T>` or the spot. Text stays
with `Display` and `Read`: `show`, not `Into<String>`, prints a value.

## Outcome types

`Either<L,R>` has `Left L` / `Right R`; `Maybe<T>` has `Nothing` / `Just T`;
`Result<T,E>` has `Err E` / `Ok T`, with `E : Err`. They are three
nominal prelude types with no shared type; a backend may share their
layout. Eligibility is per type: `|:`, `|!`, `||` and `&&` on `Maybe`
and `Result`; `|:` and `|!` on `Either`, `Left` bad and `Right` good;
`||` or `&&` on `Either` is the error "needs `Maybe` or `Result`", so
`Either<(),i64> || 0` is rejected whatever `Right 1` looks like at run
time.

An outcome bracket names a side, not a family. Against `Maybe<T>`, `[:e]`
binds `e : T` and `[!]` is `Nothing`; against `Result<T,E>`, `[:e]` binds
`e : T` and `[!e]` binds `e : E`; `[!e]` against `Maybe` is a check error.
In construction the expected type supplies the family; with none, `m =
[:1]` is the check error "outcome family unknown". `Just`, `Nothing`, `Ok`
and `Err` are ordinary constructors and fix a family. `Either` has no
bracket form, as it has no `|:`.
