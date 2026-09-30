# Limits

A limit narrows a meaning ([types](03-types.md)). This chapter owns the kinds
of limit, their declaration, `Self`, gates, coverage by a gate, and
holdings; [checks](07-checks.md) owns when a limit is decided and what it
costs, and [members](04-members.md) what a member limit asks.

## Four kinds

| A type says | By | Example | Decided |
| --- | --- | --- | --- |
| what a value is | the meaning | `Integer` | by unification |
| which values | a test | `where self <= 255` | proved, refuted or tested |
| what can be done | a member | `self.add: Self -> Self` | checked |
| when it is known | a timing | `Compile`, `Init` | checked |
| where and how it is held | a holding | `Bits<8>`, `Size<4>` | checked |

- A **test** is a Bool over `self`, a `pipeline` in the grammar.
- A **member limit**, `self.area: F64`, has a meaning or a packed type as
  its type; it holds no test.
- A **timing**, `Compile` or `Init`, says when a value is known
  ([checks](07-checks.md) §Timing); it is checked, never tested. `Compile
  (e)` and `Init (e)` are forms, a timing name with one operand: alone,
  passed as a value or in a pattern the name is a check error.
- A **holding** says how a value is held, §Holdings.

A limit is pure. It reads `self`, prelude names, host constants and the
items of its module, never a line binding. A test that enters its own
type is a check error.

## Declaration

The forms are `type_item` in [syntax.ebnf](02-syntax.ebnf):

- `type N: Base where …` narrows `Base`, a meaning or a limited type, so
  limits chain. `type N: A & B` names a combination and needs no limit.
  A single base with no limit would be an alias, and there is none.
- `type N where …` has limits and no base.
- `type N { … } where …` is a new meaning with limits; before its braces
  `:` takes holdings and claims, `type Vec2: Zero & Default { … }`. A
  claim names a member limit the meaning promises, by any name in scope,
  `a.Zero`. `impl Vec2: Zero { … }` holds exactly that claim's members; a
  limit the type line does not list is a check error there. A claim
  already met, `Default` by field defaults or a derived `Eq`, needs no
  block; one left unmet is a check error at the claim.
- `type N`, with nothing after the name, is opaque.

`where` is the one word for limits ([syntax](02-syntax.md) §Definitions). A
limited type has no `impl`; code lives in the meaning
([members](04-members.md)).

## Self

Inside `type` and `impl` bodies `Self` is the enclosing type and, in value
position, its constructor. In a limit `self` is the value and `Self` its
meaning: `Integer` inside `U8`. In an impl member the parameter `self`
has type `Self`.

## Gates

A type with a test puts its name in the value namespace, as a record puts
its constructor. A type of member limits alone has no gate: members are
checked, never tested. A gate and a constructor of one name are a check
error.

- `U8 x`, or `to<U8> x`, enters and is a Fault when the test fails.
- `try<U8> x` is `Result<U8, LimitError>`, `Err` on failure.
- The pattern `U8 b`, in an arm, tests, and binds `b` known as `U8`.
  After it fails the value is known as `!U8`, with no second test.
- `read<U8> s` reads the meaning, then tests; either failure is an `Err`
  of `ParseError`. A word at a `U8` spot does the same and Faults.

A gate is the written test, so a strict session allows it. A gate never
converts: `U8 2.0` is a check error. `to` and `try` on a limited type
first reach its meaning, by the number row or by `from` and `try_from`
([members](04-members.md) §Conversions), then test. `U8 300` is refuted;
`try<U8> 300` is `Err`; an arm that cannot match is only dead.

**Coverage** reads meanings, and one thing more: a gate pattern together
with a gate whose type is declared as its `!` completes an arm set.

A meaning declared with limits tests them once per construction: by the
derived constructor, a written one or braces. A record pattern never
tests.

## Holdings

A holding is written anywhere. `Bits<n>` holds a value in exactly `n`
bits, and `Size<n>` is `Bits<8n>`. A type must fit its holding, else a
check error where the holding is written: an `Integer` range within the
unsigned range of `n` bits, or the signed one when it holds a negative; a
sum in the bits its constructor count needs; a record or a tuple in the
sum of its parts, in order, with no padding. On a type variable it is
checked where the variable is known.

A tuple or an array has a `Size` when every part has one; an array of
packed elements has none, since a packed value has none. `Vec` and
`Deque` have none. A place with no holding is the implementation's, and
an `Integer` with none is unbounded. `Volatile`, `Pinned`, `At` and
`Order`, byte order, offsets and addresses are holdings of a hatch (T4).

## Examples

```fipe
type Natural: Integer where 0 <= self                 # a limited type
type U8: Natural & Bits<8> where self <= 255          # chained, in 8 bits
type Flag: Bool & Bits<1>                             # one bit
type Mode: Size<1> { read: Flag, write: Flag, exec: Flag }   # 3 of 8 bits
type Prime: Integer where is_prime self               # a named test
type Seed: Integer where Compile (is_prime self)      # settled at compile time
type Port: Integer where 0 < self, self <= config.max_port   # a host constant
type Name: String where 0 < self.len, self.len <= 64  # a length
type Range { lo: Integer, hi: Integer } where self.lo <= self.hi
type Shape where self.area: F64                       # members: a packed type
type Key: Eq & Show                                   # a named combination
n: U8 & Prime = 13                                    # both
type Handy: U8 | Prime                                # either
type NonPrime: Integer & !Prime                       # not
kind = n => n | { Prime p => "prime", NonPrime q => "other" }  # covered
id: Integer | String = 3
id | { Integer n => n.show, String s => s }           # which one it is

classify = n => n | { Prime p => "prime", U8 b => "small", _ => "big" }
```
