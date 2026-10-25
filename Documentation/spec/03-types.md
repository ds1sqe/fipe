# Types and checking

Checking precedes all program effects. This chapter owns meanings and the
forms that join them; [containers](05-containers.md),
[limits](06-limits.md), [checks](07-checks.md) and [members](04-members.md)
own the rest. T3 to T5 and E2 in [open decisions](11-open.md) name the
remaining choices.

## Meanings and places

A **meaning** is what a value is: a record, a sum, an opaque type,
`Integer`, `F32`, `F64`, `()`, a function, or a packed type
([members](04-members.md)). Meanings are nominal: a declared meaning is its
declaration, in checking, in the value's tag and in messages. A value has
exactly one meaning. A **limit** narrows a meaning and never changes what
an operation means; where operations differ it is a new meaning, as
`String` is beside `Vec<U8>`. A **type** is a meaning with zero or more
limits.

A **place** holds a value and declares a type: a parameter, a field, a
constructor payload, an element, an annotated binding, a declared result, the
host boundary, the payload fallback of `||`. Every value in a place meets its
limits; outside a hatch (T4) values and places are immutable, so its language
type remains fixed. This does not keep a Host entry live or authorize a later
operation on it. The meaning belongs to the value, the limits to the place:
`2` is an `Integer` and may enter every place whose limits it meets.

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
in a type is a variable, a number parameter ([containers](05-containers.md)
§Parameters), or a used module as an API reference, `Rd<fs>`
([embedding](09-host.md)); the host type that takes one declares the
parameter uppercase, `type Rd<Api>`, since a lowercase parameter is a
number. One that spells an old lowercase numeric type, `i64`, is a check
error with the hint `I64`.

Three **signs** join types, the ones of a Bool, since a limit is a Bool
over `self`; [syntax](02-syntax.md) gives their precedence:

- `A & B` holds both. Its parts share one meaning, or are member limits,
  timings or holdings. Two holdings that disagree, `Bits<8> & Bits<16>`,
  are a check error.
- `A | B` holds either. With one meaning it passes either test, tried left
  to right, stopping at the first that passes. With two it is a packed
  type ([members](04-members.md)), and a gate pattern tells which: `Integer
  n`, `String s`. It is never inferred (§Core judgments) and has no
  holding. A literal at it takes its default meaning.
- `!T` holds the values that fail at least one of `T`'s tests. It reads
  the tests of the whole chain and drops the holdings, so `!U8` is the
  integers outside 0 to 255. On a type with a member limit, or with a
  timing in a test, it is a check error.

## Declared meanings

`type Name { members }` declares a meaning: a record, whose lowercase fields
are its constructor's spots in order, or a sum, whose uppercase members are
its constructors; a one-field record is the newtype. [Limits](06-limits.md)
owns the other forms of `type`. The checker admits finite nominal templates
that refer back to their declared meaning, including changing arguments such
as `N<Vec<T>>` and `N<Rd<T>>`. This is not polymorphic recursive function
inference or permission for cyclic value graphs. The bounded finite-value
cases in
[conformance](10-conformance.md#finite-recursive-resource-conformance) admit
ordinary depths 0–2 and a genuine nested resource alias, rejecting incorrect
shapes. Broader recursive evaluation and detection termination remain T3's.
`String` is the prelude's `type String { bytes: Vec<U8> }` with a private
field and valid UTF-8; `Bool` is `type Bool { True, False }`. The language
names no resource type; [embedding](09-host.md) says how a host declares one.

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
Application first binds spots under [evaluation](08-evaluation.md), then APP
applies; each argument enters its spot ([checks](07-checks.md) §Entry and
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
lowercase names are rigid variables and its value is checked against it. A
`pub` binding exports its scheme: a variable nothing fixes stays
quantified with the member limits it carries, and each use instantiates
it, so `pub id = x => x` is `a -> a` and `pub add = a b => a + b` is
`Add<b> a => a -> b -> a`; only a host-callable signature is written out
([host](09-host.md)). A variable with a `Literal` limit defaults, to
`Integer` or else `F64`, the first that meets its other claims: at a
binding whose value is not a function, so `x = 1` and `x = [1, 2] | sum`
are `Integer`, else where it leaves the type. A member limit no default
covers, whose variable left the type, is the error "ambiguous". Where an
expected type is known, from an annotation, a spot or a sibling arm,
inference checks against it; [members](04-members.md) §The dot infers member
limits.

**Meanings have no order**: `Integer` and `F64` are neither wider.
Inference never moves a value between meanings and never invents a packed
type: `A | B` is written or it is a mismatch. **Literal arms choose.** A
literal has no meaning until one reads its form. Operands, arms and
elements that are literals stay open until the expression ends and take
the one meaning that reads every form, in the defaulting order. It is a
check error when no meaning reads them all, or when one of them already
has a meaning that cannot read the others' forms.

```fipe
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
  is [members](04-members.md)' §The dot.
- An `if` condition is `Bool`; both arms have one result type, and a
  missing `else` is `()`.
- Arms have common input types and one result type; guards are `Bool`; the
  set is exhaustive, and a guard alone cannot establish coverage;
  [limits](06-limits.md) §Gates adds a gate with its `!`. The first matching
  arm with a true guard wins. Coverage splits sums, Bool, outcome sides,
  record fields, tuples (a part is read by a tuple pattern, `(n, acc) = st`;
  there is no projection form) and packed meanings; literal patterns and Vec list
  patterns never complete a column. A literal pattern is matched by its
  meaning's `eq`, so its meaning claims `Literal` and `Eq`. An uncovered set
  names one missing pattern. `(name = v)` gives the parameter `v`'s type and
  a default.
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
a gate and the two timing forms ([limits](06-limits.md)), so `Integer`, `Vec`
and an opaque host type have no value unless a function is declared. Types
and values are separate namespaces; a type's own members are read first
after an uppercase head ([members](04-members.md) §The dot). `Point { x: 1,
y: 2 }` is the raw construction; `Point 1 2` is the same function applied,
unless the type's module writes one: bare arms in `impl T { }` are `T`'s
written constructor, joined in order, with its signature inferred from the
record it builds and the type's visibility. A sum has no written
constructor; inside the `impl`, `Self 1 2` calls it and `Self { x, y }`
bypasses it. Construction, braces included, tests a meaning's limits
([limits](06-limits.md) §Gates); a pattern never calls, it reads the raw
fields, and only a gate tests. [limits](06-limits.md) §Self owns `Self`. A
type with no members is a marker; a body is shape only, code lives in
`impl`; `pub` is written only on a record's fields.

`pub type` exports the type. A private record field is readable, matchable
and suppliable only inside its module; outside it is not a spot and takes
its default, so the derived constructor exists outside exactly when every
private field defaults. A sum constructor's fields are its payload and are
always visible. A member of a type is reached from another module only when
it is `pub`, a claim's member whenever its type is; reaching a hidden one is
a check error that says the member is private. `String`'s field has no
default, so `utf8` is the only way in; its `len` counts its UTF-8 bytes.

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
| limited type | its base's default, only when entry is proved by the static checks |

A meaning that claims `Default` has the table's value as `Self.default`,
or writes its own ([members](04-members.md) §Prelude). A zero is not a
default: `Integer` claims `Zero` and has no default, and a type that wants
both writes `default = Self.zero`. A default enters its field as any value does
([checks](07-checks.md)). A
declaration's labeled parameters are its spot names and defaults, not
part of its type: `(long: Bool = False) -> Vec<Entry>` has type `Bool ->
Vec<Entry>`. An opaque type has no fields, constructors or derived
members; a declaration of its own name, `Rd: U64 -> Rd<api>`, is its
constructor. A declared `= value` overrides the type default, may name
the spots before it, and is checked against its spot's type. Duplicate
parameter names and ambiguous Bool shorthand letters are check errors. A
default models an optional argument; `Maybe` and `Result` remain valid
parameter types. A
host boundary type has no unresolved variable and no function value. T3
limits default expressions.

## Outcome types

`Either<L,R>` has `Left L` / `Right R`; `Maybe<T>` has `Nothing` / `Just T`;
`Result<T,E>` has `Err E` / `Ok T`, with no bound on `E`. They are three
nominal prelude types with no shared type; a backend may share their
layout. Eligibility is per type: `|:`, `|!`, `||` and `&&` on `Maybe`
and `Result`; `|:` and `|!` on `Either`, `Left` bad and `Right` good;
`||` or `&&` on `Either` is the error "needs `Maybe` or `Result`", so
`Either<(),Integer> || 0` is rejected whatever `Right 1` looks like at run
time.

An outcome bracket names a side, not a family. Against `Maybe<T>`, `[:e]`
binds `e : T` and `[!]` is `Nothing`; against `Result<T,E>`, `[:e]` binds `e
: T` and `[!e]` binds `e : E`; `[!e]` against `Maybe` reports "`Nothing` has
no payload". An annotation, spot, arm input or sibling arm supplies the
family, including one learned later; with none, `m = [:1]` is the check
error "outcome family unknown". `Just`, `Nothing`, `Ok` and `Err` are
ordinary constructors and fix a family. `Either` has no bracket form, as it
has no `|:`.

## Resource identity

A trusted Host declares `type resource Rd<T>` for a genuine reference. An
unmarked opaque `Packet` is ordinary data; opacity alone does not make a
resource. A marked reference cannot be manufactured by a record constructor,
literal, `default` or automatic byte Codec conversion. Supplied Host
operations may explicitly return one, subject to the declared boundary type.

Its checked identity includes the marked declaration's context and all generic
arguments, including unit, full module references and ordinary nominal
arguments. Names, table indexes and an entry key alone do not establish that
identity. A Session fork/query context has fresh identity; withdrawing a
declaration retires its stamp, without retiring surviving earlier
declarations. The Host may rewrap a still-live entry using a validated witness
in a fresh context; that is not a new acquisition and cannot revive a closed
entry.

Typed tuples, arrays, sequences, records, packed values and function captures
may retain genuine aliases. An opaque or wildcard data position cannot conceal
one, including inside closure captures or filled arguments. A type witness
proves language identity, not payload provenance, permissions, grants or
liveness. [Host](09-host.md#environment-and-resources) owns those independent
checks.