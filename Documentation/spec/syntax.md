# Syntax

[syntax.ebnf](syntax.ebnf) is the grammar over the token inventory
in [lexical structure](lexical.md). It is accompanied by the constraints here;
EBNF alone does not resolve line, stage, binding or comma ambiguity. The
rules below complete it. The [parser API](parser-api.md) defines
representation and resource limits; checking and execution remain later
stages.

## Grammar notation

`a = b ;` defines a production; `,` concatenates; `|` chooses; `[ ]` is optional;
`{ }` repeats zero or more times; `( )` groups. Quoted text is an exact token
spelling, not a source-character rule. Uppercase identifiers are payload or
structural terminals. `(* … *)` is a grammar comment. Empty files are valid.

Every bare `NAME` can be qualified lexically. Definition sites, parameter
names and members require one segment; `use` and value or type references
may be qualified. `WORD("..")` alone is allowed as a record pattern's rest
field. Every other `WORD` in `pattern_field` is rejected. `_` in a pattern
binds nothing. `Self` is a name, not a keyword.

**The case rule.** A name whose last segment starts with an ASCII uppercase
letter is a constructor or a type wherever the parser must tell the two
apart: in a pattern it takes one payload, and before `{` it opens a record.
Elsewhere case has no syntactic meaning; checking owns "a declared type or
constructor is uppercase" and "a binding is not".

## Lines and continuation

A newline ends a line everywhere: at top level, inside `{ }` and inside
`( )`. There is no statement terminator, no layout rule and no `match`,
return, loop or reassignment form. Semicolon is accepted only in a fixed-array
type. A closing bracket or EOF can end the final line without a newline.
Blank and comment lines are ignored, except as stated below.

A line continues across one newline in exactly these cases:

- **After a trailing operator**: a pipe, `+ - * %`, a comparison, `->` or
  `=>`. A trailing `/` is the root path, not a continuation, so `ls /` ends
  there.
- **Before a leading operator**, when no blank line intervenes: a pipe,
  `+ - * %`, `/` followed by an operand, a comparison or `=>`. `!`, a
  negative literal, a flag such as `-b` and a lone `/` begin a new line.
- **After a trailing comma** inside a list, and after a list's opener or
  before its closer.
- **Across a backslash join**, which the lexer performs before tokenization.

A blank line ends a continuation. Because the lexer drops comments, a
comment-only line counts as blank. A leading operator after a blank line, on
a file's or block's first line, or after an item is an error: an operator
needs a stage on both sides, and there are no operator sections.

```text
find x |: cat |! log || root      find x                    total = price
                                      |: cat                    * count
                                      |! log                    + shipping
                                      || root
pub inc = n                       # the head ends where `=>` begins
    => n + 1
```

## Lists and blocks

Two containers exist, chosen by position, never by glyph.

A **list** is comma-separated: `[ ]`, type arguments and parameters `< >`,
a call's `( )` that touches its callee, a constructor's fields, a record's
`{ }`, a pattern's fields, a `type`, `trait` or `impl` body and an arm list.
Newlines are allowed after the opener, after a comma and before the closer.
Elsewhere a newline inside a list is an error at the newline, "expected `,`
before the line break"; a comma therefore never begins a line. A trailing
comma is allowed.

A **block** is line-separated and its value is its last line: a file, a body,
`{ }` anywhere else, and `( )` that does not touch a callee. An empty block
has the unit value. `()` is the unit literal.

```text
f(x,                # a list: the comma carries the line over
  y)
f(x                 # error at the end of this line: expected `,`
  , y)
(x = 1              # a two-line group: a binding, then `x + 1`
 x + 1)
f (                 # `f` applied to one argument, a group whose value is `g`
  g
)
cat x \             # the backslash join: `cat x y`
    y
```

**Records by case.** After a name, `(` that touches it is a call. `{` after
an uppercase name opens a record, the raw construction that mirrors the
record pattern: `Point { x: 1, y: 2 }`, and `{ x, y }` puns each field with
the value of that name. After any other name `{` is a block argument:
`map { [:e] => e.name, [!] => "-" }` is `map` applied to an arm block.

**Heads.** `if c { }`, `else { }`, `with k v { }` and item bodies open with
`{`, and in an `if` or `with` head the first top-level `{` is the body. A
record or block used in that head must be parenthesized. `else` attaches to
the nearest unmatched `if`; newlines may separate it from `}`.

## Bindings

There is no `let`. A line is a binding when a top-level `=` comes before its
first `=>`, pipe, comma or newline, not counting an `=` attached to a flag:
`x = 1`, `[x, y] = e`, `Just x = e`, `Ok _ = fs.remove dir /tmp/x`,
`x: i64 = e`. `=` always makes a new binding; a second `x = …` shadows.
`cmd -c=80` is a call, and `x => y = 1` is an arm whose body binds. The
value is a whole line, so `add = a b => a + b` binds an arm list and
`f: A -> B = x => …` puts the signature on the binding. `pub` before a
file-level binding exports it; `pub` anywhere else is an error.

## Arms

A block whose first line contains a top-level `=>` is an arm list, in either
bracket; so is a stage that begins with a head and `=>`. A **head** is one or
more patterns, each a pattern atom or `(name = operand)`, a parameter with a
default; by the case rule `Just e rest` is two patterns and `Just Just e`
one. An arm with several patterns is a function of that many parameters,
curried. Arms are a comma list: a comma follows every arm body, including a
block body. An arm body is a non-arm line and may start on the line after
`=>`; a nested arm list needs its own brackets. An arm list is one function
value: piped into, called, bound or passed. Arm order is preserved;
checking owns totality.

```text
find x | {
    [:e] if e.size > 0 => e.name,
    [:_] => "empty",
    [!] => "-",
}
find x | ([:e] => e.name, [!] => "-")
max = a b if a > b => a, _ b => b
ls: Rd -> Bool -> Vec<Entry> = dir (long = False) => …
```

## Stage selection

Use the following resolution, without consulting value types:

1. `with`, `if` and a binding introduce their own forms.
2. A top-level `=>` following a valid head starts an arm list.
3. A first operand followed by an arithmetic/comparison operator is a value
   expression, using the precedence table below.
4. An operand followed by spaced operands or flags starts an application.
5. A lone operand is a value; whether a name evaluates to a function is typed.

An operator after an application argument is rejected: `add 2 - 1` and
`cat x > y` are invalid; `add (2 - 1)` and `(cat x) > y` are valid. A
negative token is an operand, so `add 2 -1` is valid. A bound function at a
stage head can have arguments even though it is a value.

**Explicit instantiation.** A name followed by a touching `<` carries type
arguments in value position: `read<i64>`, `Rd<fs>`, `pl.into<Point>`. A
spaced `<` is comparison, so `a<b` is an error, "expected `>`", and `a < b`
compares. There is no cast keyword: a type name in value position is an
ordinary name, its constructor, and conversion is a named function.

**Outcome brackets.** `[` whose next token is `:` or `!` is an outcome
bracket, in patterns and in values: `[:e]` and `[:]` are the success side,
`[!e]` and `[!]` the miss side, with an optional payload. It is never a
list, so a one-element list of a negation is written `[(!x)]`. Which family
the bracket denotes is the checker's, from the scrutinee or expected type.

## Precedence

Higher rows bind tighter. There is no unary arithmetic minus operator; minus
is part of an adjacent numeric literal or a binary operator. `>` is
comparison at every position; no redirection form exists.

| Forms | Association |
| --- | --- |
| grouping, literals, field/selector, record and outcome forms | atomic |
| adjacent call, type arguments and field postfixes | left |
| `!` | right |
| `* / %` | left |
| `+ -` | left |
| `< <= > >=` | non-associative |
| `== !=` | non-associative |
| `| |: |! || &&` | left, one level |

An arm body extends to its enclosing delimiter or comma, including a
pipeline. Inside a list, an arm list must be bracketed so that its commas do
not consume neighboring elements; a member value in a `trait` or `impl`
body is therefore one arm, or a bracketed arm list.

`/` is a root-path operand at an operand position and division between
values. After a potential call head, `/` is division if another operand
follows it on the same line; otherwise it supplies the root path, as in
`ls /`. Group an earlier root-path argument when needed: `cat (/) other`.
Other standalone arithmetic symbols used as text arguments must be quoted.

## Arguments and calls

The parser retains a sequence of positional operands and flag arguments.
It does not decide whether `-l` takes a value from the next operand; declared
types drive [binding](evaluation.md). Under the lexical contract, `=` in an
attached flag must touch the flag and its operand's first token; whitespace
form is `-c 80`. Quoted flag text is an ordinary string, and `--` does not
end flag parsing.

Space application requires a source gap before each argument. Adjacent `(`
instead starts an explicit argument-list call on any operand: `f(a,b)` and
`f a b` supply the same argument sequence. `f()` records an explicit call with
no arguments; checking owns defaults and arity. `f (x)` passes a group;
`f (x,y)` is invalid because tuples do not exist. `f` on one line and `(g)`
on the next are two lines: a call paren must touch its callee. Call, list
and record elements may contain non-arm lines; bracket an arm value used
there. A constructor is called like any function: `Point 3 4`, `Point -y 4
-x 3`, `Point(3, 4)`, `Width`, `Point.line 2`.

Adjacent selector tokens extend an operand: `(e).name`, `f().name`, and
`(e).a.b`. A separated `.name` is a selector argument. Names such as `e.name`
retain their segments; one dot serves fields, module members and a type's
namespace, and the checker resolves the first segment. `::` has no meaning
in this draft and is kept free for a later namespace form. Numeric field
selectors such as `.0` are not admitted; no tuple syntax is added.

Record-pattern `..` may occur once, as the last field. Items cannot appear
inside expression blocks. A record or block argument immediately before a
required body brace, including a `with` value, must be grouped.

## Definitions

Items are module-level: `use`, `type`, `trait` and `impl`. `pub` may precede
`type`, `trait` and a binding; it exports. `use x` imports a module, `use
x.inc x.Point` names, `use text 2` an API version. A file-level binding
whose value is syntactically an arm list is collected with the items, so it
may recurse and be used before its line; every other binding is a line in
order.

`type Name<params> { members }` has one member grammar: a lowercase field
`[pub] name: type [= operand]`, or an uppercase constructor `Name` or
`Name(fields)`. Whether a body mixes the two, and whether names have the
right case, is checked, not parsed. Type parameters are written only by the
prelude. There is no alias, `struct` or `enum` form.

`trait Name[: Parent] { members }` has members `name: type [= value]`; an
uppercase member with a bound, `Error: Err`, is an associated type. `impl
Target[: Trait] { members }` has members `[pub] name [: type] = value`, and
in an inherent impl bare arms, which together are the type's written
constructor. Member values follow the list rule above: one arm, or a
bracketed arm list.

The function type is `A -> B`, right-associative: `A -> B -> C` is
`A -> (B -> C)`, `() -> T` takes unit, and `(A -> B) -> C` groups. A
trailing `->` continues the line. There is no `fn` in types, items or
values; the arm is the one function form.
