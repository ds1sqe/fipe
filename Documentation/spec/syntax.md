# Syntax

[syntax.ebnf](syntax.ebnf) is the candidate grammar over the token inventory
in [lexical structure](lexical.md). It is accompanied by the constraints here;
EBNF alone does not resolve stage, comma or redirect ambiguity. S1–S3 in
[open decisions](open.md) identify the missing user rulings.

## Grammar notation

`a = b ;` defines a production; `,` concatenates; `|` chooses; `[ ]` is optional;
`{ }` repeats zero or more times; `( )` groups. Quoted text is an exact token
spelling, not a source-character rule. Uppercase identifiers are payload or
structural terminals. `(* … *)` is a grammar comment. Empty files are valid.

Every bare `NAME` can be qualified lexically. Definition sites and parameter
names require one segment; imports and type/value references may be qualified.
`WORD("..")` alone is allowed as a struct pattern's rest field. Every other
`WORD` in `pattern_field` is rejected. `_` in a pattern binds nothing.

`Default` is the only name admitted by the `default` production. An import
version and an array extent are nonnegative integer values. Alias type
parameters and bounds are reserved to prelude declarations in this draft;
ordinary source never declares explicit type parameters.

## Lines and grouping

File entries and block lines are separated by retained newlines. A closing
brace or EOF can end the final line without a newline. Blank/comment lines
are ignored by the parser. Semicolon is accepted only in a fixed-array type.
There is no return statement, loop statement, reassignment or `match` keyword.

S1 proposes ignoring newlines inside parentheses, brackets, type arguments,
record fields and enum bodies. Braces used as a block preserve line boundaries.
The lexer still returns those newlines; this is a parser rule.

When a block begins with an arm, every top-level line in that block is an
arm (with optional commas between arms); it forms one function value.
Otherwise it is a sequence of lines whose last value is the block value.
An empty block has the unit value. Pattern arms are total and ordered.

## Stage selection

Use the following candidate resolution (S1), without consulting value types:

1. `let`, `with`, and `if` introduce their own forms.
2. A top-level `=>` following a valid pattern/guard starts an arm list.
3. A first operand followed by an arithmetic/comparison operator is a value
   expression, using the precedence table below.
4. A name followed by operands or flags starts an application.
5. A lone operand is a value; whether a name evaluates to a function is typed.

An operator after an application argument is rejected: `add 2 - 1` is invalid;
`add (2 - 1)` is valid. A negative token is an operand, so `add 2 -1` is valid.
A bound function at a stage head can have arguments even though it is a value.
`as T` can start a pipeline stage as the cast operation with one open input;
`value as T` is the explicit postfix cast. This special form needs no binding
named `as` and must be represented explicitly in the parser.

In an `if` head or guard the first top-level `{` begins the body, so a record
or block used in that head must be parenthesized. `else` attaches to the nearest
unmatched `if`; S1 allows separating it from `}` by newlines.

## Precedence proposal (S1)

Higher rows bind tighter. There is no unary arithmetic minus operator; minus
is part of an adjacent numeric literal or a binary operator.

| Forms | Association |
| --- | --- |
| grouping, literals, field/selector, function and record forms | atomic |
| `!` | right |
| `as T` | left |
| `* / %` | left |
| `+ -` | left |
| `< <= > >=` | non-associative |
| `== !=` | non-associative |
| `| |: |! || &&` | left, one level |
| final `> target`, `>> target` | suffix on the entire pipeline |

An arm body extends to its enclosing delimiter, including a pipeline. Inside
a list, record or call, an arm list must be parenthesized. This prevents its
commas from consuming neighboring elements. S1 also limits those element
productions to non-arm lines despite the shared EBNF `line` production.

S2 proposes treating `>` at outer line level as redirection. A value comparison
using `>` must be grouped there: `(x > y)`. In an `if` head, guard or group it
is comparison. Thus `cat x > y` redirects, and `filter (e => e.size > n)`
compares. `<`, `<=` and `>=` have no redirect meaning. Redirects appear once,
after the final stage; `2>`, input redirection and stderr pipes do not exist.

Inside a type-argument list, a `>>` token is consumed as two `>` closers with
separate byte spans. No shift operator exists. This is a parser operation,
not a context-sensitive lexer rule.

## Arguments and calls

The parser retains a sequence of positional operands and flag arguments.
It does not decide whether `-l` takes a value from the next operand; declared
types drive [binding](evaluation.md). Under the lexical contract, `=` in an attached flag must
touch the flag and its operand's first token; whitespace form is `-c 80`.
Quoted flag text is an ordinary string, and `--` does not end flag parsing.

The EBNF uses space application. Existing design examples additionally use
`fs.Entry(a, b)` and `x.m()`; their distinction from `f (a, b)` and curried
application is S3. They are not silently admitted by the candidate grammar.
There is no general tuple value grammar pending that decision; `()` is unit.

## Definitions

Items are module-level. A function has a name, parameters, an optional result
annotation and a body. An anonymous `fn` has the same parameter/body grammar.
Written signatures export functions; inferred signatures remain private.
The exact completeness required for an exported signature is T3.

Struct and enum definitions are nominal. A newtype has one unnamed field.
A trait contains signatures; an implementation contains function definitions.
Whether user aliases and trait families need richer grammar is governed by
T1 and T3, not by acceptance of a token sequence alone.
