# A first script

A Fipe script is a file of lines. Each line is a binding, `name =
value`, or an expression; every expression line has a value, and the
std host writes it to the standard output when the line ends, a
`String` as its text and anything else as the language spells it:

```fipe
"hello"
1 + 2
[1, 2, 3]
```

```sh
fipe run hello.fipe
```

```text
hello
3
[1, 2, 3]
```

Bindings write nothing. A name is lower case; a type, upper. `()` and
`Nothing` write nothing either, so a line can stand for its effect
alone.

## Input

When the last line is a function of one `String` spot, the standard
input is that string. `wc` counts it three ways:

```fipe
# Count lines, words and characters of stdin.
text => join " " [
    show (len (lines text)),
    show (len (words text)),
    show (len text),
]
```

```sh
fipe run wc.fipe < poem.txt
```

`lines` and `words` split the text; `len` counts a vector's elements
and a string's characters; `show` spells a value; `join " "` puts the
parts in one string, which the last line writes. The prelude's names
are listed by `:help NAME` in `fipe repl`.

A function is written `spot => body`. A call is the function followed
by its arguments: `show 3`, `join " " parts`. Arguments bind left to
right, so an argument that is itself a call, or an operator
expression, is grouped: `show (len text)`, `(len parts) > 1`.

## Exit codes

| Code | When |
| --- | --- |
| 0 | the run ended with a value that is not an `Err` |
| 1 | the run ended with `Err e`, or Faulted |
| 2 | the script does not check, or its input is refused |

A script that does not check prints each diagnostic as
`file:line:column: reason` and runs nothing. A Fault, as a division by
zero, is written as `fault in main at line:column: reason`. Both go to
the standard error. [Outcomes](03-outcomes.md) shows how a script
reports an `Err` of its own.
