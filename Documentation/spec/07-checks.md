# Checks

Each limit of a place ([limits](06-limits.md)) is decided in a phase, with
one of three outcomes, under a mode. The rules of §What is known are part
of the language: a checker proves exactly what they give, so acceptance
is the same on every backend.

## Phases

| Phase | When | May run |
| --- | --- | --- |
| compile | before any effect | pure code on exact values, within the compile budget |
| init | once, when a module runs | its bindings in order, host calls included |
| eval | when an expression runs | anything |

Init is ordinary evaluation of a module's bindings, once, under the same
evaluator; the phase is a label. Pure code reaches no host
command, no `log` and nothing of a hatch; a float member is pure for the
compile phase only when IEEE 754 rounds it correctly, so `sin` is not. A
host constant is a host declaration with a written value, exact and part
of the checked unit, as the compile budget is. The compile budget is an
allowance per evaluation, counted in the checker's steps
([host](09-host.md) §Budget), set by the host and part of the checked
unit.

The pure float members are the operators, comparisons, `sqrt`, the four
roundings and the number conversions; `sin`, `cos`, `atan2` and `hypot`
are not. Purity follows names, called or passed, through each binding
group. A member at an unknown head is pure only when every member of
that name in the checked unit is pure.

## Entry and outcome

`x: Limit = value`. The place is the lower bound, what `x` must meet;
the value is the upper bound, what it is; entry decides the difference.
A value **enters** a place when it is bound to it. A signature on an arm
list declares that function's places: its parameters take the arguments,
and each body enters the declared result.

At a place of known type a value of another meaning is cast, not unified
([types](03-types.md)), by the closed cast table of [members](04-members.md)
§Conversions: a number to another numeric meaning, a word read, text, the
host codec, packing. First check against the place; on failure, read again
without an expected type, default literals and consult the table, keeping
the first error if no pair is admitted. Casts that may Fault are recorded
for evaluation, not open limit entries, and strict mode does not refuse
them. Each element of a literal enters the element place
before the elements are unified, so `[Circle 1.0, Square 2.0]` fills a
`Vec<Shape>`. Then each limit of the place is:

- **proved**: §What is known evaluates it to `True`; nothing runs;
- **refuted**: it evaluates it to `False`; a check error in every mode;
- **open**: neither.

A value enters by each part that the place's type limits: the elements of
a `Vec<U8>`, the payload of a `Maybe<U8>`.

## Modes

| Knob | Effect | Set by |
| --- | --- | --- |
| a plain limit | an open entry is tested when it runs | the limit's author |
| `Compile (test)` | the test is settled at compile time, else a check error | the limit's author |
| `Init (test)` | the test is settled when the module runs; an entry inside a function must be proved | the limit's author |
| strict session | an open entry is a check error, in every module the unit checks; a gate still tests | the host |
| compile budget | how far each compile-time evaluation may run; exhausted means open, and a check error for a `Compile (test)` or a demanded value | the host |

No mode skips a test; a trusted entry belongs to a hatch (T4).

| Check | Compile | Init or eval | Configurable |
| --- | --- | --- | --- |
| syntax, names, meanings, member limits, arm coverage | always | never | no |
| a function entering a place | always, but at a callee's variable | at such a variable, per call (rule 8) | no |
| a test: range, length, named test, record limit | proved or refuted | an open entry | yes |
| `Integer` division by zero | refuted when the divisor is exactly zero | when its range holds zero | no |
| a host reply against its declaration | never | always | no |

## Timing

Timing is a limit of a place: `Compile & T` holds a value known at
compile time, `Init & T` one known once its module has run; a place with
neither takes any value, and the way up is free. Every expression has an
earliest phase, the latest of its inputs; a host call is no earlier than
where it runs. A binding's place, or a timing form, may demand an earlier
phase: the value is then prepared in that phase, pure and within the
budget, and is exact; a prepared value stays exact for what follows,
later session lines included; a value that cannot be ready is a check
error. The
demand is read before anything is evaluated, and an effect is never moved
out of the function that holds it. A parameter's phase is eval: a timing
limits only a binding annotation, never another place, since a body is
checked once. A demand moves pure code only: a host call is init only
when it stands in a module-level binding outside every arm. A bare
expression line is eval; a session binding is init. Names keep their
binding value's phase across lines and modules. Pure calls take the
latest phase of their callee and arguments; parameters are eval.
`Compile (test)` in a limit asks that the truth of the test be known at
compile time, by §What is known, not that `self` be. There is no `prove`
and no `compile` word. `Compile` and `Init` are documented prelude names
and may be shadowed. Their expression forms take exactly one operand;
piping into them or passing the names as values is a check error.

## The plan

The checker lists every entry left open: its place, its limit, its phase,
the reason (an unknown input, no rule, the budget) and how often it runs:
once, per call, per element. An open test is init at a module-level binding,
eval inside an arm or on a bare expression line, and compile on an exact
value or a value demanded at compile. Such a test runs in the compile
phase: proved, it leaves the plan; refuted, or Faulting, it is a check
error. It stays open as "compile budget exhausted" when the budget runs
out, and as "compile-time evaluation" when its value reads a local or a
parameter, which a single expression cannot be evaluated without. An
open `Compile (test)` is a check error. The REPL shows it with `:checks`, and
the [embedding API](../api/09-embed.md) returns it with the checked unit.

```text
:checks sum2
sum2: U8 -> U8 -> U8
eval, per call: the result, range 0..510, against U8 (unknown input)
```

## What is known

Known of a value: a range `[lo, hi]` when it is an `Integer` or a length,
its ends finite or not; the names of the limited types it has passed; its
parts. Known of a function: its parameter and result limits. Rules 4
to 8 are ruled (2026-10-21).

1. **Exact.** A literal is known exactly, a float after rounding; a list
   literal has its length, a text literal the length `String.len` gives,
   and an exact value's length is exact. So is an `Integer` operation on
   exact operands, charged to the compile budget, and a construction of
   exact parts. A test on an exact value is evaluated at compile time. A
   test that Faults has failed, in every phase.
2. **Places.** A value read from a place meets its limits: each narrows
   `self` as a condition would, and the names of the place's type are
   known. A name keeps what is known of its value.
3. **Arithmetic**, on `Integer` only. `a + b` is `[la + lb, ha + hb]`;
   `a - b` is `[la - hb, ha - lb]`; `a * b` spans the four corner
   products; `a / b` spans the four corner quotients of each nonzero part
   of the divisor; `a % b`, with `m` the largest magnitude of `b` less
   one, is `[0, min(ha, m)]` for `la >= 0`, `[max(la, -m), 0]` for
   `ha <= 0`, else `[max(la, -m), min(ha, m)]`. Zero times an infinite end
   is zero; a finite end over an infinite one is zero; an infinite end
   over an infinite one is infinite, with the sign of the product. An
   empty range is dead code and proves anything. A comparison of two
   ranges is `True` when every pair satisfies it and `False` when none
   does; `&&`, `||` and `!` follow over known Bools.
4. **Conditions.** `x OP e` and `e OP x` narrow `x` by the range of `e`,
   for `x` an `Integer` name or `x.len`, and `OP` one of `< <= > >= ==`.
   `!=` narrows only at an end of the range. `&&` narrows by both sides,
   its right side already by the left; `||` narrows the else branch by
   both negations; `!` swaps. An else branch and the later arms take the
   negation where it is one range of one name; a later arm takes a guard's
   negation at the same parameter position when the guarded arm's patterns
   are names or `_`. A gate pattern narrows its arm. A literal pattern
   narrows a piped bare `Integer` name to itself in its arm and, past an
   unguarded literal arm, by `!=` in the later arms. A condition known
   `True` or `False` prunes no branch. `fault` never returns, so its
   branch is empty. Branches join: the smallest range that holds both, the
   names both have, and of a function, in the value or a part, its
   parameters met and its results joined. Nothing else leaves a branch.
5. **Calls.** A call is known by the declared result's limits and by the
   body, taken once at the declared parameter limits. The body is read for
   a function of the checked unit, its file modules or an earlier session
   line; a prelude or host function is known by its declared limits and
   scheme only, which say what goes in at its variables (a sequence's
   elements, `Items<a> c`; ruled 2026-10-22). A callee not yet read, as in
   a cycle, is known by its declared result.
6. **Variables.** At a call of a function whose body is read, what comes
   out at a type variable is the join of what went in at it and of the
   declared result limits of each function argument that returns it. It is
   unknown when something can produce that type unseen: a limit of the
   scheme whose member's result holds it, a function argument that returns
   it with no declared limit, or an argument holding it in a part. An arm
   list written as the argument takes what is known at its parameter. With
   nothing going in, it is unknown, not dead.
7. **Parts.** A construction is known by its parts; a marked pipe joins
   its branches.
8. **Functions.** A function value enters a place of function type only
   when proved: parameters wider, results narrower, flipping at each
   arrow; at a variable, against what is known there, so `apply f 300`
   with `f: U8 -> U8` is refuted; where what goes in there is not known to
   fit, the parameter limits are an open entry tested on each call of the
   value (ruled 2026-10-22). A function value going in whole at a variable
   is known where it comes out as the call's result (rule 6); elsewhere,
   as at a constructor or `log`, its parameter limits are such an open
   entry. Any other function entry unproved is a check error in every mode
   (§Modes). A member meets a member limit by the same proof, so a member
   with a narrower parameter does not. A call tests its argument against
   the known parameter limits, an argument not known to fit being an open
   entry as at any place. A partial call `f a` is known by the remaining
   parameter limits and the result limits. An arm list with no signature
   has no parameter limits and is known by its body; only a function value
   from a place without limits is unknown, and calling it gives an unknown
   result.
9. A limit is required only where written. It never moves backward to a
   parameter.
10. Anything else is unknown, as a cast's value or a call in a test.

## Narrow and wide

Limits have an order, inside one meaning. Narrow holds fewer values:
exact `2`, then `U8`, then `Natural`, then `Integer`. Narrow to wide is
always permitted and costs no test; wide to narrow is an entry. Branches
join (rule 4); a join never fails, a later entry may. No fixed point is
computed, so no widening step is needed: a recursive result is known as
declared (rule 5). Meanings have no order ([types](03-types.md) §Core
judgments).

```fipe
y = if c { 2 } else { 200 }   # Integer, known 2..200
a: U8 = y                     # proved
b: I8 = y                     # open
```

## Order and charges

A test runs when its value is bound, before the next argument is
evaluated: the base's limits first, then the type's in written order, `&`
and `|` parts left to right, elements and tuple parts in order. The first
failure stops and is a Fault that names the place, the limit and the
value. A proved entry costs nothing at run time; compile-time work
counts against the compile budget. A value that moves to a place with another
holding is converted, charged per element.

```fipe
a: U8 = 200
b: U8 = 100
c: U8 = (a + b) / 2       # proved: 150
d: U8 = a + b             # refuted: 300 is outside 0..255
p: Prime = 7              # proved: the test ran at compile time
port: Port = cli.port     # init: tested once, when the module runs
half: U8 -> Integer = n => n / 2
h: U8 = half 200          # proved: the body gives 0..127
clamp: Integer -> U8 = n =>                       # proved
    if n < 0 { 0 } else { if n > 255 { 255 } else { n } }
sum2: U8 -> U8 -> U8 = x y => x + y               # open: tested per call
max = x y if x > y => x, _ y => y
m: U8 = max a b           # proved: both went in at one variable
primes = Compile (sieve 1000)                     # prepared at compile time
q: Seed = cli.seed        # a check error: not known at compile time
type Odd: Integer where self % 2 == 1
from: Integer -> Odd = n => n | {                 # the gate pattern tests
    Odd q => q,
    _ => from (n + 1),
}
next: Odd -> Odd = p => from (p + 1)              # proved: declared result
```
