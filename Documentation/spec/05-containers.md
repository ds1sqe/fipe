# Containers

This chapter owns the tuple, the array, `Vec` and `Deque`, the bracket and
type parameters. [Types](03-types.md) owns meanings and their forms,
[members](04-members.md) packed elements, [limits](06-limits.md) holdings.

## Kinds

| Kind | Written | Length | Element types | Held |
| --- | --- | --- | --- | --- |
| tuple | `A * B` | fixed, in the type | one per position | the sum of its parts |
| array | `[T; n]` | fixed, in the type | one | `n` times the element |
| `Vec<T>` | | grows at one end | one | on the heap |
| `Deque<T>` | | grows at both ends | one | on the heap |

A tuple type is `A * B * C`, flat, two parts or more; its value and
pattern are `(a, b)` ([syntax](02-syntax.md)). A tuple is known by its shape,
as an array and a function are, with no declaration. Its parts are reached
by pattern only; there is no numeric selector, and a record is the answer
when parts need names. Each part is a place of its own: `U8 * U8` tests
both. A mismatch between `f(a, b)` and `f (a, b)` ([syntax](02-syntax.md)
§Arguments and calls) is a check error that says which was written. Where
both readings check, as with a default, the checker warns; `f ((a, b))` is
the form no spacing can change.

## Brackets

A bracket is a written form: `[a, b, c]` means the array `[T; n]`, and
with no place it is one. A place that expects another type hands the array
to that type's `sequence` member. The place gives the element type; each
element enters it as a place of its own ([checks](07-checks.md) §Entry and
outcome), then the array goes to `sequence`. A limited type uses its
base's member, then tests its limits. A bracket of exact elements is
handled at compile time, and a `sequence` that Faults on it is a check
error. An array already bound enters a `Vec` place by a gate only.

`Items<T>` is the member type the prelude's list functions are written
over, held by the array, `Vec` and `Deque`; its one member is `fold`, a
walk from the front carrying a value. How a walk stops early or runs from
either end is T5's. `map` on an array keeps its length; `filter` returns
a `Vec`.

## Parameters

A type takes a type or a number as an argument: `Bits<8>`, `[T; n]`. A
program may write type parameters on its own meanings and limited types,
as the prelude does: `type Pair<A, B> { … }`. A parameter may carry member
limits, `type Sorted<T: Ord> …`; a test goes on the place, `v: T & U8`. A
lowercase parameter stands for a number known at compile time, `type
Ring<T, n> { slots: [T; n] }`, the same rule that gives `Bits<n>` its
argument. There is no higher kind: a parameter never stands for a
container without its argument. A parameter may take a default, trailing
as a spot's: `type Add<R = Self, O = Self> where self.add: R -> O`. An
argument left out is its default, and `Self` there is the meaning that
meets the limit ([limits](06-limits.md) §Self), so `Add a` is `Add<a, a> a`
and the printer leaves out an argument equal to its default.

A number argument may be a sum of numbers and number parameters, and no
other arithmetic. Two are equal when their sums are, in any order. An
array's `concat` is `[T; m] -> (self: [T; n]) -> [T; n + m]`.

```fipe
pair: Integer * String = (1, "one")
(n, name) = pair
swap = (a, b) => (b, a)                       # a * b -> b * a
divmod: Integer -> Integer -> Integer * Integer = a b => (a / b, a % b)
ends: U8 * U8 = (0, 255)
rgb: [U8; 3] = [255, 128, 0]                  # Size<3>
type Pair<A, B> { first: A, second: B }
type Ring<T, n> { slots: [T; n], at: Natural }
type Sorted<T: Ord>: Vec<T> where is_sorted self

type Sequence<T> where Self.sequence: [T; n] -> Self
type Items<T> where self.fold: b -> (b -> T -> b) -> b
impl Vec<T>     { sequence: [T; n] -> Self }               # supplied
impl Ring<T, n> { sequence = slots => Self slots 0 }
xs = [1, 2, 3]                                   # [Integer; 3]
zs = xs ++ [4, 5]                                # [Integer; 5]
shapes: Vec<Shape> = [Circle 1.0, Square 2.0]    # packed, then sequence
s: Sorted<Integer> = [1, 2, 3]                   # Vec's, then the limit
total xs                     # check error when total takes a Vec
total (xs | to<Vec<Integer>>)
```
