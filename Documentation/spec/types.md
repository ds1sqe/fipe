# Types and checking

Checking precedes all program effects. This chapter owns meanings and
the forms that join them; [containers](containers.md), [limits](limits.md),
[checks](checks.md) and [members](members.md) own the rest. T3 to T5 and E2 in
[open decisions](open.md) name the remaining choices.

## Meanings and places

A **meaning** is what a value is: a record, a sum, an opaque type,
`Integer`, `F32`, `F64`, `()`, a function, or a packed type
([members](members.md)). Meanings are nominal: a declared meaning is its
declaration, in checking, in the value's tag and in messages. A value has
exactly one meaning. A **limit** narrows a meaning and never changes what
an operation means; where operations differ it is a new meaning, as
`String` is beside `Vec<U8>`. A **type** is a meaning with zero or more
limits.

A **place** holds a value and declares a type: a parameter, a field, a
constructor payload, an element, an annotated binding, a declared result,
the host boundary. Every value in a place meets its limits; outside a
hatch (T4) values and places are immutable, so this never goes stale. The
meaning belongs to the value, the limits to the place: `2` is an
`Integer` and may enter every place whose limits it meets.

## Type forms

```text
τ ::= () | Bool | Integer | F32 | F64 | [τ; n] | Vec<τ> | Deque<τ>
    | τ * τ | C<τ…> | τ -> τ | α
σ ::= ∀α₁…αₙ. Q ⇒ τ
```

`C` names a declared meaning, `α` an inference variable, `Q` a set of
member limits. Meanings are unified; limits take no part. An API
reference includes the host-resolved version. An arm with n head patterns
has an n-ary curried type; a zero-parameter function is `() -> T`.

A type name starts uppercase, the numeric ones included. A lowercase name
in a type is a variable, a number parameter ([containers](containers.md)
§Parameters), or a used module as an API reference, `Rd<fs>`
([embedding](host.md)); the host type that takes one declares the
parameter uppercase, `type Rd<Api>`, since a lowercase parameter is a
number. One that spells an old lowercase numeric type, `i64`, is a check
error with the hint `I64`.

Three **signs** join types, the ones of a Bool, since a limit is a Bool
over `self`; [syntax](syntax.md) gives their precedence:

- `A & B` holds both. Its parts share one meaning, or are member limits,
  timings or holdings. Two holdings that disagree, `Bits<8> & Bits<16>`,
  are a check error.
- `A | B` holds either. With one meaning it passes either test, tried left
  to right, stopping at the first that passes. With two it is a packed
  type ([members](members.md)), and a gate pattern tells which: `Integer
  n`, `String s`. It is never inferred (§Core judgments) and has no
  holding. A literal at it takes its default meaning.
- `!T` holds the values that fail at least one of `T`'s tests. It reads
  the tests of the whole chain and drops the holdings, so `!U8` is the
  integers outside 0 to 255. On a type with a member limit, or with a
  timing in a test, it is a check error.

## Declared meanings

`type Name { members }` declares a meaning: a record, whose lowercase
fields are its constructor's spots in order, or a sum, whose uppercase
members are its constructors; a one-field record is the newtype.
[Limits](limits.md) owns the other forms of `type`. Value types are
nonrecursive; whether a function type may make one indirect is T3's.
`String` is the prelude's `type String { bytes: Vec<U8> }` with a private
field and valid UTF-8; `Bool` is `type Bool { True, False }`. The language
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
applies; each argument enters its spot ([checks](checks.md) §Entry and
outcome), where a cast is admitted only at a place whose declared type is
known, and inference only unifies. `name<τ…>` instantiates `name`'s
declared type parameters in order, the target first for the conversion
words; a line whose target is still unknown is an ambiguity error.

**Generalization.** Every binding generalizes over the variables not free
in the environment and keeps its unsolved member limits; there is no
mutation, so no value restriction, and a host call in a value does not
block it. An arm binding may call itself, and the members of one `impl`
may call each other: such a group is typed monomorphically and generalized
at its boundary, so there is no polymorphic recursion. An annotation's
lowercase names are rigid variables and its value is checked against it; a
`pub` binding ends closed, with no unsolved variable and no quantified
variable it did not write. A variable with a `Literal` limit defaults,
`Integer` first and `F64` next: at its binding when only literals
constrain it, so `x = 1` is `Integer`, else where it leaves the type.
Which other member limits it may carry is T5's; a member limit no default
covers, whose variable left the type, is the error "ambiguous". Where an
expected type is known, from an annotation, a spot or a sibling arm,
inference checks against it; [members](members.md) §The dot infers member
limits.

**Meanings have no order**: `Integer` and `F64` are neither wider.
Inference never moves a value between meanings and never invents a packed
type: `A | B` is written or it is a mismatch. **Literal arms choose.** A
literal has no meaning until one reads its form. Operands, arms and
elements that are literals stay open until the expression ends and take
the one meaning that reads every form, in the defaulting order. It is a
check error when no meaning reads them all, or when one of them already
has a meaning that cannot read the others' forms.

```text
z = if c { 2 } else { 2.5 }   # F64, in either order
v = [1, 2.5]                  # [F64; 2]
n: Integer = 2
w = if c { n } else { 2.5 }   # check error: n has a meaning
u = if c { 2 } else { "a" }   # check error: no meaning reads both
```

## Fields, functions and branches

- If `Γ ⊢ e : S` and record `S` declares `field : T`, then `e.field : T`;
  `.field` is the same function. An absent field, or any field of a sum,
  is a check error; a sum's payload is reached by pattern. Any other dot
  is [members](members.md)' §The dot.
- An `if` condition is `Bool`; both arms have one result type, and a
  missing `else` is `()`.
- Arms have common input types and one result type; guards are `Bool`; the
  set is exhaustive, and a guard alone cannot establish coverage;
  [limits](limits.md) §Gates adds a gate with its `!`. The first matching
  arm with a true guard wins. `(name = v)` gives the parameter `v`'s type
  and a default.
- Captures are values; shadowing never mutates a capture. A discarded
  non-unit value in a body is a check error; `_ = e` discards. A file's
  top-level line is the implicit main's output, not a discard.
- The case rule at declarations: a declared type or constructor is
  uppercase; a binding, field, parameter or member is not. A file-level
  binding whose value is an arm list is collected with the items and may
  recurse; every other binding is visible after its line.

## Constructors and privacy

A `type` puts one function per constructor in the value namespace, under
the constructor's name or, for a record, the type's; nothing else does but
a gate and the two timing forms ([limits](limits.md)), so `Integer`, `Vec`
and an opaque host type have no value unless a function is declared. Types
and values are separate namespaces; a type's own members are read first
after an uppercase head ([members](members.md) §The dot). `Point { x: 1,
y: 2 }` is the raw construction; `Point 1 2` is the same function applied,
unless the type's module writes one: bare arms in `impl T { }` are `T`'s
written constructor, joined in order, with its signature inferred from the
record it builds and the type's visibility. A sum has no written
constructor. Construction, braces included, tests a meaning's limits
([limits](limits.md) §Gates); [limits](limits.md) §Self owns `Self`.

`pub type` exports the type. A private record field is readable, matchable
and suppliable only inside its module; outside it is not a spot and takes
its default, so the derived constructor exists outside exactly when every
private field defaults. A sum constructor's fields are its payload and are
always visible. A member of a type is reached from another module only
when it is `pub`; reaching a hidden one is a check error that says the
member is private. `String`'s field has no default, so `utf8` is the only
way in. A `pub` binding or member needs a complete type (T3).

## Defaults and declarations

| Type | Type default |
| --- | --- |
| `()` | `()` |
| `Bool` | `False` |
| `Integer`, floats, `String`, function, `Either`, sum, host types | none |
| `Vec<T>`, `Deque<T>` | empty |
| `[T; n]` | `n` copies of `T`'s default, if it has one |
| record | its constructor applied to nothing, if every spot defaults |
| `Maybe<T>` | `Nothing` |
| limited type | its base's default, when that meets the limits |

A default enters its field as any value does ([checks](checks.md)). A
declaration's labeled parameters are its spot names and defaults, not
part of its type: `(long: Bool = False) -> Vec<Entry>` has type `Bool ->
Vec<Entry>`. An opaque type has no fields, constructors or derived
members; a declaration of its own name, `Rd: U64 -> Rd<api>`, is its
constructor. A declared `= value` overrides the type default, may name
the spots before it, and is checked against its spot's type. Duplicate
parameter names and ambiguous Bool shorthand letters are check errors. No
parameter is `Maybe` or `Result`; defaults model optional arguments. A
host boundary type has no unresolved variable and no function value. T3
limits default expressions.

## Outcome types

`Either<L,R>` has `Left L` / `Right R`; `Maybe<T>` has `Nothing` / `Just T`;
`Result<T,E>` has `Err E` / `Ok T`. They are three
nominal prelude types with no shared type; a backend may share their
layout. Eligibility is per type: `|:`, `|!`, `||` and `&&` on `Maybe`
and `Result`; `|:` and `|!` on `Either`, `Left` bad and `Right` good;
`||` or `&&` on `Either` is the error "needs `Maybe` or `Result`", so
`Either<(),Integer> || 0` is rejected whatever `Right 1` looks like at run
time.

An outcome bracket names a side, not a family. Against `Maybe<T>`, `[:e]`
binds `e : T` and `[!]` is `Nothing`; against `Result<T,E>`, `[:e]` binds
`e : T` and `[!e]` binds `e : E`; `[!e]` against `Maybe` is a check error.
In construction the expected type supplies the family; with none, `m =
[:1]` is the check error "outcome family unknown". `Just`, `Nothing`, `Ok`
and `Err` are ordinary constructors and fix a family. `Either` has no
bracket form, as it has no `|:`.
