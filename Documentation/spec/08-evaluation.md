# Evaluation

`e ⇓ v` denotes a value, `e ⇑ F` a Fault; a demanded subexpression that
Faults ends the run. E1, E2 and T5 in [open decisions](11-open.md) hold the
proposals cited here.

## Order

Values are immutable; a function returns a new value. E1: strict,
left-to-right evaluation, callee first, supplied arguments in source order,
defaults in parameter order, then body; each supplied expression exactly
once, even when a named argument binds a later parameter; a tuple's parts
left to right; tests where [checks](07-checks.md) §Order and charges places
them. A block evaluates its lines in order and is its last line, or `()`.
An `if` evaluates only its selected arm. Arms test patterns in source
order, a guard only after its pattern matches, and only the chosen body.

## Parameter binding

Let a function declare ordered spots `(p₁:τ₁, …, pₙ:τₙ)`. Written
arguments are kept in order until the signature is known, then bound:

1. Named arguments fill their named spots; one or two dashes are the same.
   For a Bool, a bare flag is `True` and `= value` an explicit Bool. A
   non-Bool flag consumes its `= operand` or the next operand. A chained
   flag expands only to unambiguous Bool first letters, case-sensitive.
2. Positional operands fill unfilled non-Bool spots in declaration order.
   A trailing `Vec<T>` collects the remaining operands, default empty; a
   named Vec argument supplies one list. A word over `Items<T>` does not
   collect trailing operands; commands retain their `Vec` spots.
3. A pipe supplies the last spot still open, whatever its type: `x | f 1 2`
   is `f 1 2 x`; to place the input elsewhere, name it, `x | v => f v 2 3`.
   A previously filled spot is a check error. Collections and tuples are
   never spread; `f (a, b)` passes one tuple ([containers](05-containers.md)).
4. Each remaining spot takes its parameter default, then its type default,
   in declaration order; a default may name the spots before it, so `dir
   (width = 80) (cols = width / 8) => …` is well defined. A spot with
   neither stays open. The result is a function of the open spots in order,
   or the body result. Construction is application: `Point 1` fills `y`
   from its default, and `Just` stays a function; construction tests the
   meaning's limits ([limits](06-limits.md) §Gates).

Each bound argument enters its spot ([checks](07-checks.md)). Duplicate
supply, unknown flags, non-Bool chaining and extra arguments are check
errors where statically knowable, and a refuted entry always is. `--` is a
word; quoting prevents flag recognition; there is no end-of-flags marker.
T3 resolves inferred Bool parameters.

```fipe
add: Integer -> Integer -> Integer = a b => a + b
add2 = add 2                   # one open spot
add2 4                         # 6
```

## Plain pipe

`a | f x ≡ f x a`: evaluate the left once, then bind its value to the final
spot of the right stage. No payload is extracted: `Maybe<Entry> | cat` is a
check error when `cat` needs `Entry`.

## Marked pipes

For a two-sided value let `L(l)` be the bad side and `R(r)` the good side.
`Maybe` has no miss payload: `|!` on a `Maybe` supplies nothing, and its
stage is a value evaluated on the miss. `|:` selects right and `|!` left.

```text
L(l) |: f   ⇓ L(l)                # f is not called
R(r) |: f   ⇓ rebuild_right(f r)
R(r) |! f   ⇓ R(r)                # f is not called
L(l) |! f   ⇓ rebuild_left(f l)   # Result and Either
Nothing |! v ⇓ rebuild_left(v)    # Maybe: v is a value
```

A plain return from `f` is wrapped in the selected constructor; a return of
the same family is used directly, and E1 prefers that reading when both
unify. An inactive branch keeps its payload. For `Maybe`, a plain `|!`
result is discarded and yields `Nothing`; `Result` and `Either` constrain
neither payload. Both branches have one static result type. Marked pipes
on a user meaning, through its `then`, `pure`, `or` and `empty` members,
are T5's. Bool is unresolved in E1. An outer `|!` sees a miss from any
stage below it, and with `Maybe` cannot tell which. Prefer, in order:
`Result` with one error type per chain; closing the inner pipeline,
`find x |: (| cat || fault "unreadable") |! log`; reordering, `find x
|! log || root | cat`.

## Fallback and sequencing

On Bool, `||` and `&&` short-circuit with Bool operands. For `Maybe` or
`Result` the checker fixes the mode of `||`:

| Right type | Good left | Bad left |
| --- | --- | --- |
| same outcome family | original left value | evaluate right outcome |
| good payload type | unwrap good payload | evaluate right payload |

An unneeded right side has no effects and no budget cost. Other right
types are check errors after entry; both branches have one result type,
so a wrapper cannot survive where the failure branch returns a payload.
`m && rhs` is `m |: (_ => rhs)`. `Either` refuses `||` and `&&`.
A user meaning's fallback is T5's.

## Pattern binding

`name = e` binds in the remaining scope; a later `name = …` shadows, and a
closure keeps what it captured. A constructor pattern exposes fields or
payloads; a gate pattern tests and binds the whole value
([limits](06-limits.md) §Gates); `_` discards.

```fipe
Just x = e
body                ≡   e |: (x => body)
```

`Ok` is analogous and `[:x] = e` is the same binding spelled by side: a
failure ends the current body with that failure, lifted by the marked-pipe
rule; nested pattern bindings nest. Where implicit rebuilding would leave
two meanings, an explicit constructor is required. E1 defines other
failing patterns; they are never runtime panics.

## Faults

`fault message` ends the run without a value; budget exhaustion,
arithmetic failure, a failed test, dynamic cast failure and host failure
do the same. The host receives a source-located Fault; prior host effects
stand. An `Err` value is never a Fault merely by being returned. Writing
is a host command with a value-last spot, in [embedding](09-host.md).

## Arithmetic

`Integer`'s members never wrap and nothing overflows. `div` truncates
toward zero and `rem` takes the dividend's sign; division by zero is a
Fault. A width is a limit of the place ([members](04-members.md) §Number
types). E2 specifies float division and overflow, NaNs and ordering;
constant evaluation obeys the same rules.
