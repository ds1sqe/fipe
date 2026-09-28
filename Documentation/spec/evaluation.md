# Evaluation

This chapter defines observable language behavior, not VM opcodes. `e ⇓ v`
denotes a value; `e ⇑ F` denotes a Fault. Any demanded subexpression that
Faults terminates the run. Order and rebuilding proposals cite E1/E2 in
[open decisions](open.md).

## Values and order

Values are primitives, immutable collections/records/variants, resources,
functions with captured values, or a value packed with its trait dictionary.
A function returns a new value; it does not mutate an input value. Host calls
may have declared effects. There is no language assignment.

E1 proposes strict, left-to-right evaluation: callee first, supplied arguments
in source order, defaults in parameter order, then body. Evaluate each supplied
expression exactly once, even when a named argument binds a later parameter.
A block evaluates lines in order and returns its last line, or `()` if empty.
An `if` evaluates only its selected arm. Arms test patterns in source order;
evaluate a guard only after its pattern matches, and only the chosen body.

## Parameter binding

Let a function declare ordered spots `(p₁:τ₁, …, pₙ:τₙ)`. Retain written
arguments in order until its signature is known. Bind in these steps:

1. Named arguments fill their named spots. One or two dashes are equivalent.
   For a Bool, a bare flag means `True` and `= value` supplies an explicit Bool.
   A non-Bool flag consumes its explicit `= operand` or the next operand.
   A chained flag expands only to unambiguous Bool first letters, case-sensitive.
2. Positional operands fill unfilled non-Bool spots in declaration order.
   A trailing `Vec<T>` collects remaining positional operands; its default is
   empty. A named Vec argument supplies one list, not separate elements.
3. A pipe supplies the last spot still open after named and positional
   arguments, whatever its type: `x | f 1 2` is `f 1 2 x`. To place the
   input elsewhere, name it: `x | v => f v 2 3`. A previously filled spot is
   a check error. Collections are never implicitly spread.
4. Each remaining spot takes its parameter default, then its type default,
   in declaration order; a default may name the spots before it, so
   `dir (width = 80) (cols = width / 8) => …` and a field `len: u64 = norm
   x y` are well defined. A spot with neither remains open. The result is a
   function of the open spots in declaration order, or the body result if
   all spots are filled. Construction is application: `Point 1` fills `y`
   from its default, and `Just` stays a function.

Duplicate supply, unknown flag, non-Bool shorthand chaining, extra arguments
and an invalid cast are rejected at check time where statically knowable.
`--` is an ordinary word. Quoting prevents flag recognition. There is no
end-of-flags marker. T3 must resolve interactions with inferred Bool parameters.

```text
add: i64 -> i64 -> i64 = a b => a + b
add2 = add 2                   # one open spot
add2 4                         # 6
```

## Plain pipe

```text
a | f x   ≡   f x a
```

The equivalence is spot binding, not textual re-evaluation: evaluate the left
once, then bind its value to the final spot of the right stage. No outcome
payload is extracted. `Maybe<Entry> | cat` is a check error if `cat` needs
`Entry`; use `|:` for payload-aware application.

## Marked pipes

For a two-sided value let `L(l)` be the bad/left side and `R(r)` the good/right
side. `Maybe` uses unit for its left payload. `|:` selects right and `|!` left.

```text
L(l) |: f   ⇓ L(l)                # f is not evaluated/called
R(r) |: f   ⇓ rebuild_right(f r)
R(r) |! f   ⇓ R(r)                # f is not evaluated/called
L(l) |! f   ⇓ rebuild_left(f l)
```

A plain return from `f` is wrapped in the selected constructor. A return of
the same outcome family is used directly, with types checked across both
possible paths. E1 proposes preferring direct use over an extra wrapper when
both interpretations could unify. An inactive branch retains its payload.

For `Maybe`, a plain result of `|!` is discarded and the result is `Nothing`;
a returned `Maybe` is retained. For `Result`, a plain `|!` result must implement
`Err`. For `Either`, both payload types are unconstrained by error/display.
Both branches still determine one static result type.

A user type implementing `Monad` supports `|:` by `then`, with `pure` for a
plain return. It has no `|!` unless a future specification defines a bad-side
interface. T3 owns trait signatures. Bool participation is unresolved in E1.

## Fallback and sequencing

On Bool, `False || rhs` evaluates `rhs`, `True || rhs` returns `True`;
`False && rhs` returns `False`, `True && rhs` evaluates `rhs`. Both operands
are Bool. Evaluation is short-circuited.

For `Maybe` or `Result`, the checker fixes the result mode of `||`:

| Right type | Good left | Bad left |
| --- | --- | --- |
| same outcome family | original left value | evaluate right outcome |
| good payload type | unwrap good payload | evaluate right payload |

An unneeded right expression has no effects and consumes no evaluation budget.
Other right types are check errors after admitted casts. The checker must
choose one result type for both branches; successful branches cannot preserve
a wrapper when the failure branch returns a plain payload.

`m && rhs` has the behavior of `m |: (_ => rhs)`; a bad left skips `rhs`.
`Either` refuses `||` and `&&`, because neither side means success. A user
`Alternative` supplies same-family fallback; payload-unwrapping fallback is
not inferred without a declared payload interface (T3).

## Pattern binding

Ordinary `name = e` binds the value in the remaining scope; a later `name =
…` shadows it and a closure keeps the value it captured. A matching
constructor pattern exposes fields or payloads; `_` discards a value.

The agreed success-pattern case has the following meaning:

```text
Just x = e
body

≡ e |: (x => body)
```

`Ok` behaves analogously, and `[:x] = e` is the same binding spelled by
side. Failure ends the current body with the same failure;
the continuation is lifted by the marked-pipe rule. Nested pattern bindings
nest these binds. If implicit rebuilding would leave two possible meanings,
require an explicit constructor. E1 must define arbitrary failing patterns,
`Left`/`Err` patterns and nested partial patterns; they are not runtime
panics by default.

## Writing and faults

The language has no redirection. A host that writes declares a command whose
final spot takes the value, so the pipe fills it: `cat x | put out`,
`ls | append names` in the conformance host. The value meets that spot by
the ordinary [cast policy](types.md), which never admits an outcome
implicitly. The language owns no filesystem.

`fault message` ends the run without a value. Budget exhaustion, arithmetic
failure, dynamic cast failure and host failure behave the same way. The host
receives a source-located Fault. Prior host effects are not implicitly rolled
back, and an `Err` value does not become a Fault merely because it is returned.

## Arithmetic

Integer overflow and division by zero Fault, including minimum signed integer
divided by -1. No wraparound is implicit. E2 must specify integer division and
remainder signs, float division/overflow, NaNs and ordering before numeric
backends can claim equivalence. Constant evaluation must obey the same rules.
