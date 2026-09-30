# Syntax

[syntax.ebnf](02-syntax.ebnf) is the grammar over the tokens of
[lexical structure](01-lexical.md); the rules here resolve what EBNF cannot:
lines, stages, bindings and commas. The [parser API](../api/02-parser.md) owns
representation and limits.

In the grammar `,` concatenates, `|` chooses, `[ ]` is optional, `{ }`
repeats, `( )` groups, quoted text is a token spelling and uppercase names
are terminals. Definition sites, parameters and members take a one-segment
`NAME`; a member limit and a constraint take two, `self.area: F64`,
`b.age: a`; `use` and references may be qualified. The only `WORD` in
`pattern_field` is `..`, the rest field. `_` in a pattern binds nothing.

**The case rule.** A name whose last segment starts with an ASCII uppercase
letter is a constructor, a gate ([limits](06-limits.md)) or a type wherever
the parser must tell them apart: in a pattern it takes one payload, and
before `{` it opens a record.
Elsewhere case has no syntactic meaning; checking owns "a declared type or
constructor is uppercase" and "a binding is not".

## Lines and continuation

A newline ends a line everywhere, including inside `{ }` and `( )`; a
closer or EOF ends the last line. There is no terminator, layout rule,
`match`, return, loop or reassignment. Semicolon occurs only in a
fixed-array type. A line continues across one newline exactly when:

- **After a trailing operator**: a pipe, `+ - * % ++`, a comparison, `->`
  or `=>`, and `& | *` in a type. A trailing `/` is the root path, not a
  continuation, so `ls /` ends there.
- **Before a leading operator**, when no blank line intervenes: a pipe,
  `+ - * % ++`, `/` followed by an operand, a comparison, `=>` or `=`, so
  a signature may stand above its `= value`. `!`, a negative literal, a
  flag such as `-b` and a lone `/` begin a new line.
- **After a trailing comma** inside a list, and after a list's opener or
  before its closer.
- **In limits**: after `where` and after a limit's comma, and before a
  line that begins with `where`, which continues the `type` item.
- **Across a backslash join**, which the lexer performs.

A blank line ends a continuation, and a comment-only line is blank. A
leading operator after a blank line, at the start of a file or block, or
after an item is an error: there are no operator sections.

## Lists and blocks

Position, never the glyph, chooses between two containers. A **list** is
comma-separated: `[ ]`, `< >`, a call's `( )` touching its callee, a
constructor's or record's or pattern's fields, an item body and an arm
list. Newlines are allowed only after the opener, after a comma and before
the closer; elsewhere a newline in a list is the error "expected `,` before
the line break". A trailing comma is allowed. A **block** is line-separated
and its value is its last line: a file, a body, any other `{ }`, and `( )`
not touching a callee. An empty block is `()`, the unit literal.

A **tuple** `(a, b)`, value or pattern, is a `( )` not touching a callee
whose top level holds a comma before any `=>`. It takes the newlines and
the trailing comma of a list; an arm inside it is grouped. `(a)` stays a group and `()` unit:
there is no tuple of one.

```fipe
f(x,                # a list: the comma carries the line over
  y)
f(x                 # error at the end of this line: expected `,`
  , y)
(x = 1              # a two-line group: a binding, then `x + 1`
 x + 1)
f (                 # `f` applied to one argument, a group whose value is `g`
  g
)
```

**Records by case.** `{` after an uppercase name opens a record, the raw
construction mirroring the record pattern: `Point { x: 1, y: 2 }`, and
`{ x, y }` puns each field with the value of that name. After any other
name `{` is a block argument: `map { [:e] => e.name, [!] => "-" }`.

**Heads.** In an `if` or `with` head the first top-level `{` is the body;
a record or block in the head is parenthesized. `else` attaches to the
nearest unmatched `if`, across newlines.

## Bindings

A line is a binding when a top-level `=` comes before its first `=>`,
pipe, comma or newline, not counting a flag's `=`, or a `=>`, pipe or
comma inside an annotation: `x = 1`, `[x, y] = e`, `Just x = e`,
`x: I64 = e`, `id: Integer | String = 3`. `=` always makes a new binding,
visible from its line to the end of its scope, so a second `x = …`
shadows; an arm binding's own name is in scope in its arms. `cmd -c=80` is
a call and `x => y = 1` an arm whose body binds. The value is a whole line:
`add = a b => a + b` binds an arm list, `f: A -> B = x => …` puts the
signature on the binding. `pub` before a file-level binding exports it;
elsewhere it is an error.

## Arms

A block whose first line has a top-level `=>` is an arm list, in either
bracket; so is a stage beginning with a head and `=>`. A **head** is one or
more patterns, each a pattern atom or `(name = operand)`, a parameter
with a default; `Just e rest` is two patterns, `Just Just e` one. Several
patterns make a curried function of that many parameters. Arms are a comma
list; a comma follows every body, block bodies included. A body is a
non-arm line and may start after the newline following `=>`; a nested arm
list needs its own brackets. An arm list is one function value, in order;
checking owns totality.

```fipe
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

In order, without types: `with`, `if` and a binding introduce their own
forms; a top-level `=>` after a head starts an arm list; an operand
followed by an arithmetic or comparison operator is a value expression; an
operand followed by spaced operands or flags is an application; a lone
operand is a value. An operator after an application argument is an error:
`add 2 - 1` and `cat x > y` are invalid, `add (2 - 1)` and `(cat x) > y`
valid; a negative literal is an operand, so `add 2 -1` is valid.

**Explicit instantiation.** A name with a touching `<` carries type
arguments: `read<I64>`, `Rd<fs>`, `to<Point>`. A spaced `<` compares,
so `a<b` is the error "expected `>`". There is no cast keyword.

**Outcome brackets.** `[` followed by `:` or `!` is an outcome bracket in
patterns and values: `[:e]` and `[:]` the success side, `[!e]` and `[!]`
the miss side. It is never a list, so a one-element list of a negation is
`[(!x)]`. The family is the checker's. In a type `[!T; 3]` is an array,
and `|!` is an error with the hint to write `| !`.

## Precedence

Higher rows bind tighter. There is no unary minus: minus is part of a
literal or a binary operator. `>` is comparison everywhere.

| Forms | Association |
| --- | --- |
| grouping, literals, field/selector, record and outcome forms | atomic |
| adjacent call, type arguments and field postfixes | left |
| `!` | right |
| `* / %` | left |
| `+ - ++` | left |
| `< <= > >=` | non-associative |
| `== !=` | non-associative |
| `| |: |! || &&` | left, one level |

In a type `!` binds tightest, then `&`, `|` and `*`; `->` is loosest and
right-associative, so `A * B -> C` takes a tuple.

An arm body extends to its enclosing delimiter or comma, pipeline
included; inside a list an arm list is bracketed, and a member value is one
arm or a bracketed arm list.

`/` is the root path at an operand position and division between values.
After a call head it is division if another operand follows on the line,
else the root path: `ls /`; an earlier root-path argument is grouped,
`cat (/) other`. Other bare arithmetic symbols as text are quoted.

## Arguments and calls

The parser keeps positional operands and flags in order and never decides
which spot one fills; [evaluation](08-evaluation.md) binds them. `-c=80` is
one flag argument, `-c 80` a flag and an operand. Space application needs
a gap before each argument; a touching `(` is an explicit argument list, so
`f(a,b)` and `f a b` supply the same sequence, `f()` calls with none,
`f (x)` passes a group and `f (a, b)` one tuple
([containers](05-containers.md)). A call paren touches its callee, so `f` and
`(g)` on the next line are two lines. A constructor is called like any
function: `Point 3 4`, `Point -y 4 -x 3`, `Point(3, 4)`, `Width`,
`Point.line 2`.

Touching selectors extend an operand: `(e).name`, `f().name`. A separated
`.name` is a selector argument. `e.name` keeps its segments for the checker,
whose one dot serves fields, modules and type namespaces. `::` has no
meaning in this draft. Numeric selectors such as `.0` do not exist.

Record-pattern `..` occurs once, last. Items cannot appear in expression
blocks. A record or block argument just before a required body brace,
including a `with` value, is grouped.

## Definitions

`##` documents the next item (except `use`), binding, field, constructor
or impl member; a binding inside a body may have a doc too. Spacing and
blank lines before the owner are allowed. Consecutive doc lines of one
form join with LF. A doc with no owner or followed by the other form is
a syntax error. `##!` documents the module, only before its first entry.
Text is Markdown, opaque except the declaration spot check in
[host](09-host.md); no doc changes meaning. There is no block doc or `@param`.
Constructor fields and `use` lines take no docs ([open](11-open.md) L2).

Items are module-level: `use`, `type`, `impl` and a declaration; `pub`
exports a `type`, binding or declaration. `use x` imports a module, `use
x.inc x.Point` names, `use text 2` an API version. A
file-level binding whose value is syntactically an arm list is collected
with the items, so it may recurse; every other binding is a line in order.

`type Name<params> { members }` has one member grammar: a lowercase field
`[pub] name: type [= operand]` or an uppercase constructor `Name` or
`Name(fields)`; mixing and case are checked, not parsed. There is no
alias, `struct` or `enum`. A `type` may also take a base after `:` and
limits after `where`, [limits](06-limits.md) §Declaration; `:` joins a name
to its type everywhere: an annotation, a field, a parameter to its limit,
a type to its base. `where` is a word of the `type` item only, as `Self`
is a name: `grep where` is a command.

`impl Target { members }` has members `[pub] name [: type] = value`, a
member `name: type` with no value, and bare arms, the type's written
constructor; `impl Target: Limit { … }` holds one claim's members
([limits](06-limits.md)). A member value is one arm or a bracketed list.

The function type `A -> B -> C` is `A -> (B -> C)`; `() -> T` takes unit
and `(A -> B) -> C` groups. There is no `fn`: the arm is the one function
form.

Constraints stand at the top of an annotation, `show: Show a => a ->
String`, never inside a type.

**Declaration forms**, for the prelude and host declarations, rejected
elsewhere by the checker: a signature-only declaration, `pub len: Vec<a>
-> Natural`, a file-level `name: type` with no `=`; an impl member with no
value; a labeled parameter in a function type, `(long: Bool = False) ->
…`, whose default is an operand; and `type Rd<Api>` with no body, an
opaque type. [Types](03-types.md) §Type forms reads a lowercase name in a
type.

```fipe
##! Files: the shell reading of Fipe.
use fs

## The ten largest files under a directory, as names.
## - depth: how far down to look
pub biggest = (depth = 1) dir =>
    ls -l -depth depth dir | sort -by .size -desc | take 10 | map .name

type Point {
    ## Horizontal, in cells.
    pub x: I64,
    ## Vertical, in cells.
    pub y: I64,
}
```
