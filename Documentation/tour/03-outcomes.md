# Outcomes

A value that may be missing is a `Maybe`: `Just v` or `Nothing`. A
value that may have failed is a `Result`: `Ok v` or `Err e`. Reading a
number from text gives a `Result`:

```fipe
"42" | read<Integer>
"x" | read<Integer>
```

```text
42
ParseError "not an integer: \"x\""
```

The std host writes `Ok v` and `Just v` as `v`, `Err e` to the
standard error, and the run exits 1 when its last value is an `Err`.

## Patterns

A `Maybe` or `Result` is taken apart by brackets: `[:v]` is `Just v`
or `Ok v`, `[!e]` is `Err e`, `[]` is `Nothing`. The arms of a function
written `{ pattern => value, … }` choose by pattern:

```fipe
describe: Result<Integer, String> -> String = r => r | {
    [:v] => join " " ["got", show v],
    [!e] => join " " ["error:", e],
}
describe (Ok 1)
describe (Err "no")
```

The annotation `name: type = value` fixes the outcome's family, which
the patterns alone leave open.

## Pipes through outcomes

`v |: f` applies `f` to the payload of a good outcome and passes a bad
one on; `v |! f` lets `f` replace a bad one; `v || d` takes `v`'s
payload or `d`. `calc` reads each line `a op b` with them:

```fipe
# Calculate each line `a op b` (op is + - * /); one result or `error: ...` per line.
apply = op a b => if op == "+" { Ok (a + b) } else if op == "-" { Ok (a - b) }
    else if op == "*" { Ok (a * b) }
    else if op == "/" { if b == 0.0 { Err "division by zero" } else { Ok (a / b) } }
    else { Err (join " " ["unknown operator", op]) }
number = s => s | read<F64> |! (_ => Err (join " " ["not a number:", s]))
calc = line => line | words | {
    [a, op, b] => number a |: (x => number b |: (y => apply op x y)),
    _ => Err "expected a op b",
}
show_line = line => calc line | {
    [:v] => show v,
    [!e] => join " " ["error:", e],
}
text => text | lines | map show_line | join "\n"
```

`[a, op, b]` matches a vector of exactly three; `_` matches anything.
`F64` arithmetic shows a fraction, so `1 + 2` prints `3.0`.

## Reporting from a script

A binding `[:n] = r` takes `r`'s payload and, when `r` is an `Err`,
ends the script with it: the host writes the error and exits 1. The
last line can also be an `Err` on purpose. `config-lint` writes its
report and ends with the count of problems as an `Err`, so its exit
code says whether the config passed:

```fipe
problems = ["3: duplicate key: port"]
if (len problems) == 0 { Ok "config ok" } else {
    problems | join "\n" | concat "\n" | bytes | put out
    Err (join " " [show (len problems), "problems"])
}
```
