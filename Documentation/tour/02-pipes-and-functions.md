# Pipes and functions

`x | f` is `f x`: the value on the left enters the function's last
spot. A pipeline reads in order:

```fipe
[1, 2, 3, 4] | filter (n => n > 1) | map (n => n * 10) | sum
```

`filter` keeps the elements its function says `True` of, `map`
applies a function to each, `sum` adds them: `90`. A lambda `n => n >
1` is a function value; a binding gives one a name.

## Fold

`fold seed step` carries a state through the elements: `step` takes
the state and an element and gives the next state. The state can be a
tuple, taken apart by a pattern in the lambda's head. `dedupe` keeps
the first of each repeated line:

```fipe
# Print the lines of stdin without repeats, keeping first-seen order.
keep = seen l => if (seen | filter (s => s == l) | len) > 0 { seen } else {
    seen | concat (to<Vec<String>> [l])
}
text => text | lines | fold (to<Vec<String>> []) keep | join "\n"
```

`concat` joins two vectors or two strings, the piped value first:
`xs | concat ys` is `xs` then `ys`. A bare `[]` is an array of a fixed
length, so a growing vector starts from `to<Vec<String>> []`; a
`to<T>` is the language's cast.

## The script's arguments

`cli.args` holds the arguments after `--` on the command line, as a
`Vec<String>`; `use cli` brings the module in. `grep-log` numbers the
lines holding a pattern:

```fipe
# Print the lines of a log that contain a pattern, with line numbers.
use cli
pat = first cli.args || ""
hit = l => (len (split pat l)) > 1
numbered = (n, acc) l => (n + 1, if hit l {
    acc | concat (join ": " [show n, l]) | concat "\n"
} else { acc })
log => {
    (_, found) = log | lines | fold (1, "") numbered
    found | bytes | put out
}
```

```sh
fipe run grep-log.fipe -- ERROR < app.log
```

`first cli.args` is a `Maybe<String>`; `|| ""` takes the string or the
default. `split pat l` cuts the line at each `pat`, so more than one
piece means a hit. The body in braces holds its own bindings, `(_,
found) = …` taking the fold's result apart, and ends with its value.
`put out` writes bytes to the standard output, in order with the
lines, so the last line's value is `()` and writes nothing more.

## `if` and blocks

`if c { a } else { b }` is an expression; both branches have one
type. A block `{ … }` holds bindings and ends with its value. A
function of several spots is `a b => body`; a call with fewer
arguments than spots gives a function of the rest: `concat "\n"` is
the function that appends a line feed.
