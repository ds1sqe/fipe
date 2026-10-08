# Members

Every operation lives in a type. This chapter owns members and member
limits, the dot, derived members, packed types, conversions, literals and
the number types. [Limits](06-limits.md) owns the member limit as a kind of
limit; [checks](07-checks.md) owns entry.

## Members

`impl T { members }` holds the code of `T`, a meaning declared in this
module, one member per name. In the prelude and a host file a member
written `name: type`, with no value, is supplied by the implementation;
it takes its value last, so its `self` is its last spot.

A type whose limits are member limits names who has an operation: `Add`,
`Concat`, `Eq` and `Show` are declared with `type … where` (§Prelude).
**Membership is by claim** ([limits](06-limits.md) §Declaration): a meaning
is an `Add` when its type line says so and `impl T: Add { … }` writes `add`
of that type, or the member is derived. A member of that name on a meaning
that does not claim is its own, not the limit's. "Of that type" is the proof
of [checks](07-checks.md) §What is known for a function, so a member whose
parameter is narrower does not meet it. A limit claimed twice, `Mul<F64> &
Mul<Vec2>`, has a block each; the argument's meaning chooses.

An operation is a member of the meaning of its subject. An operator is a
spelling of a claimed member: `a + b` is the `add` of the left operand's
`Add`, and a meaning that does not claim `Add` has no `+`:

| Written | Type | Its limit |
| --- | --- | --- |
| `+` `-` `*` `/` `%` | `Add<R = Self, O = Self>`, `Sub` … `Rem` | `self.add: R -> O`, and alike |
| `==` `!=` | `Eq` | `self.eq: Self -> Bool` |
| `<` `<=` `>` `>=` | `Ord`, which is an `Eq` | `self.cmp: Self -> Ordering` |
| `!` | `Not` | `self.not: Self` |
| a literal | `Literal` | `Self.literal: Form -> Self` |
| words | `Len`, `Show`, `Read` | `self.len: Natural`, and alike |

`++` spells `concat` by name, not its type: an array is not a `Concat`, whose
result is `Self`; its `concat` adds the lengths
([containers](05-containers.md) §Parameters). The right operand and the
result of an arithmetic operator are what the left's member takes and gives,
`Self` by the parameters' defaults ([containers](05-containers.md)
§Parameters): `v * k` is a `Vec2`'s `Mul<F64>`, `p - q` a `Sub<Point,
Vector>`, and the left operand owns the operation. On a head whose meaning is
unknown a parameter the operands do not fix takes its default: the right
operand fixes `R`, nothing fixes `O`, so `diff = p q => p - q` is `Sub<b> a
=> a -> b -> a` and a mixed result needs a known head. A literal operand is
read by its other operand's meaning (§Literals). The other operators take one
meaning; a comparison across meanings is a conversion, not an operator. A
prelude word is an ordinary function over such a type: `show = x => x.show`,
`len`, `map`, `concat`. `Functor`, `Monad` and `Alternative` are the members
`map`, `then`, `pure`, `or` and `empty`, each meaning with its own types, as
there is no higher kind ([containers](05-containers.md) §Parameters).

## The dot

`x.m` is a field of `x`'s meaning, else a member of it, else a member of
module `x`; [embedding](09-host.md) resolves `x`. A type's member written on
the type, not on a value, is reached by the type's name: an uppercase
head reads its type's members first, so `Point.from pl`, `Maybe.pure 1`,
`Integer.add`. The dot fills `self`; the rest bind positionally. A
function with a `self` parameter in scope is called as a
function, not by the dot. A private field is seen only in its own module.
A member has no bare name, even in its own module: it is `x.m`, `T.m` or
the selector `.m`, so `| .area`, not `| area`.

A dot dispatches on the head's tag. A member limit with no value to read,
`Self.read` or `Self.literal`, receives the meaning as a hidden argument.
A program cannot read a tag.

**Limits are inferred.** On a head whose meaning is not yet known an
operator is its type's limit in the scheme, `Add<b> a`, met by a claim,
and a dot is a member limit, `b.age: a`, met by any member of that name;
each is solved once the head is known, and the error stands at the
operator or the dot if it never is. The prefix the REPL prints is written
in programs too. The printer never turns a member limit into a name, nor
a variable with a limit into a packed type.

## Derived members

`eq`, `cmp` and `show` are derived for a meaning that claims `Eq`, `Ord`
or `Show` and writes no block for it, when its parts claim them, and for
a tuple and an array from their parts; a function has none, an opaque
meaning only what is supplied. A block replaces the derived member.
Derived `cmp` orders constructors by declaration, then fields left to
right. The REPL and a Fault print a value by its structure, whatever
`show` is written, and text as text. `String` supplies its `show`.

## Packed types

A type whose limits are all member limits, or an `A | B` of two meanings,
used as the type of a place, is a meaning of its own: it holds a value of
any meaning that claims the limits, with its tag. Entering it packs, at a
place of known type. It claims those of its limits whose members take
`Self` as the receiver alone, and `Eq` when it is among them: tags
first, then values. So `Add a => a -> a` is generic and keeps `add`, while
`Add -> Add` packs and cannot add. `Vec` and `Deque` hold packed
elements, `Vec<Shape>`, and so may an array; a tuple needs none, since
each position has its type.

## Conversions

`Cast(A, B)` is the closed set of conversions a place admits; nothing
else fires at a place. A missing pair is a check error, a failing admitted
one a Fault, a statically invalid literal a check error. A word or path
literal is text with no type of its own: at a spot whose type is
`String`, or any meaning claiming `Read`, it takes that type;
anywhere else it is `String`.

| Place | Admitted source | On failure |
| --- | --- | --- |
| numeric meaning | a number of another numeric meaning | exact, or Fault; a literal is checked at compile time |
| a `Read` | word or `String` | Fault |
| `String`, `Vec<U8>` | any `T` claiming `Show` | never |
| host-declared type | `Vec<U8>` via the host codec | Fault |
| `Vec<U8>` | `String` | never |
| a packed type | a meaning that claims its limits | never |

The number row is the one way between numeric meanings, with no `from` for
it; between widths of `Integer` entry replaces it. An integer literal a
float cannot hold exactly is a check error. A byte row needs a `Vec<U8>`
place, written or reached by a limited base chain; `Vec<Integer>` is none.
An exact pair precedes `Show`, and every host-declared type admits the
codec. Pairs do not chain, a codec or formatting failure never falls back,
and a type name is never a conversion: `pl | Point` is `Point pl`. An
outcome type, by its own meaning, never meets a `String` or `Vec<U8>` place
until `||` or `|:` removes it.

Every other conversion is a word. Text and numbers: `show`, `read : Read
a => String -> Result<a, ParseError>`, `bytes`, `utf8 : Vec<U8> ->
Result<String, Utf8Error>`, and `round : Float a => a -> a`. A change of
meaning is a `From<A>` or a `TryFrom<A, E>` claim on the target, one
block per source meaning, and never both for one pair. The argument's
meaning chooses among them and must be known at the call, else a check
error that names the candidates. `to<T>` calls
`from`, or `try_from` and Faults on `Err`; `try<T>` calls `try_from`, or
wraps `from` in `Ok` after testing its parameter's limits, so it never
Faults on them. The target comes from `<T>` or the spot. A new type that
must reach a foreign target writes a named member. Nothing chains and
nothing is searched; text stays with `show` and `read`.

```fipe
pl | Point.from     "3,4" | Point.try_from     pl | to<Point>     "3,4" | try<Point>
```

## Literals

What is written is a `Form`: a number in any radix, a fraction, text or
bytes, with its exact payload, and for a number its radix and digit
count. The compiler builds it. `Literal` is a type as `Add` is: who can
be written. A literal at a meaning `T` is `T.literal` of its form, run at
compile time, and a Fault there is a check error; a `literal` must be
pure. The meaning is that of the literal's place or of its other operand,
which must be a `Literal`.
In generic code it is known only where the function is used, so the
literal is prepared there. With nothing fixed a number in any radix is an
`Integer` known exactly, a fraction an `F64` known after rounding, text a
`String`, bytes a `Vec<U8>`; literals side by side choose together
([types](03-types.md) §Core judgments). A program's own meaning takes
literals by claiming `Literal`.

A written form may carry a prefix and a suffix ([lexical](01-lexical.md)):
each constructor of `Form` also holds `prefix: String` and `suffix:
String`, empty when not written. A place decides as for any literal, and
its type's `literal` reads both; with no place the reader in scope, a
type's statement that it reads a prefix or suffix, decides the type.
Two readers of one prefix or suffix in scope with no place to decide are
a check error; one with neither is a check error that names the missing
reader, never an application. `b"…"` is the prelude's reader. The
reader's spelling is open ([open decisions](11-open.md) L1).

## Number types

`Integer` is the one integer meaning, exact and unbounded; its arithmetic
is [evaluation](08-evaluation.md)'s and is charged by operand size (H1). `I8
I16 I32 I64 U8 U16 U32 U64` are widths, `U8` as in [limits](06-limits.md)
§Examples, and a width is a limit: values of different widths mix, since
all are `Integer`. Only the way down can fail, at the place, by
[checks](07-checks.md): a concrete integer operation is the exact one
followed by the way down. Each width names `Bits<n>` beside its range,
signed when the range holds a negative.

`F32` and `F64` are two meanings with IEEE 754 members ([evaluation](08-evaluation.md)
§Arithmetic): no value is infinite or NaN, so both are ordered.

## Prelude

```fipe
type Add<R = Self, O = Self> where self.add: R -> O     # prelude; Sub … Rem alike
type Concat where self.concat: Self -> Self
type Eq     where self.eq: Self -> Bool
type Ord: Eq where self.cmp: Self -> Ordering
type Show   where self.show: String
type Zero    where Self.zero: Self           # the identity of `add`
type Default where Self.default: Self        # what an empty place takes
type Read   where Self.read: String -> Result<Self, ParseError>
type From<A> where Self.from: A -> Self
type TryFrom<A, E> where Self.try_from: A -> Result<Self, E>
type Form {                                  # what was written
    Whole(value: Integer, radix: U8, digits: Natural),   # 7 0b0111 0x00FF
    Fraction(digits: Integer, exponent: Integer),    # 2.5 is 25 and -1
    Text(value: String),                             # "abc" 'raw'
    Bytes(value: Vec<U8>),                           # b"abc"
}
type Literal where Self.literal: Form -> Self        # who can be written
type Number: Add & Sub & Mul & Div & Rem & Ord & Literal
type Float: Number where self.round: Self
type I8: Integer & Bits<8> where -128 <= self, self <= 127   # I16 to I64

impl Integer: Add { add: Self -> (self: Self) -> Self }  # supplied
impl Maybe   { map: (a -> b) -> (self: Maybe<a>) -> Maybe<b> }
```

## Examples

```fipe
type Vec2: Add & Mul<F64> & Show { x: F64, y: F64 }
impl Vec2: Add      { add = o self => Self (self.x + o.x) (self.y + o.y) }
impl Vec2: Mul<F64> { mul = k self => Self (self.x * k) (self.y * k) }
impl Vec2: Show     { show = self => join "," [self.x.show, self.y.show] }
(Vec2 1.0 2.0) + (Vec2 3.0 4.0)
scale = v k => v * k           # Mul<b> a => a -> b -> a
(Vec2 1.0 2.0) * 2.0           # check error: Vec2 is no Literal

type Mask: Literal { bits: U8 }
impl Mask: Literal {
    literal = f => f | {
        Whole { value, radix: 2, digits: 8 } => Self value,
        _ => fault "a Mask is written as eight binary digits",
    },
}
m: Mask = 0b0000_1111          # its own literal, at compile time
k: Mask = 15                   # check error: the literal Faults
who: Name = "fipe"             # String's literal, then Name's limits
double = n => n + n            # Add a => a -> a
inc = n => n + 1               # Add a, Literal a => a -> a
same = p q => p == q           # Eq a => a -> a -> Bool
older = d e => e.age > d       # b.age: a, Ord a => a -> b -> Bool
total: Vec<Shape> -> F64 = xs => xs | map .area | sum   # packed

x = 2                  # Integer, known 2
b: U16 = 1000          # a is checks.md's U8, 200
a + b                  # Integer, known 1200: widths mix
w: U16 = a             # proved: the way up
n: U8 = b              # refuted: 1000 is outside 0..255
f: F64 = 2             # 2.0, by F64's `literal`
k: I64 = round 2.5     # 2, by the number row, then the limit
g: I46 -> I46 = x => x # an unknown type, not a variable
```
