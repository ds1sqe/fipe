# Parser API

[Syntax](syntax.md) owns the language rules; this chapter owns the Rust
representation and the implementation limits.

## Entry point

~~~text
parse::parse(input: lex::Lexed) -> Result<ast::Module, Vec<parse::Error>>
Error { span: token::Span, reason: parse::Reason }
~~~

The parser consumes owned tokens and does no rescanning, lookup, checking
or I/O. Lexical errors come back as `Reason::Lexical` without parsing.
Any error rejects the whole result; recovery only collects more errors.
`Lexed` is publicly constructible: a missing, repeated or nonempty-span
EOF, or overlapping or reversed spans, is `InvalidTokenStream`, never a
panic. An empty file is a module; the module span runs from byte zero
through EOF, including a skipped BOM.

## Representation

A module owns expressions, patterns and type syntax in three flat arenas,
reached by `ExprId`, `PatternId` and `TypeId` through `expr`, `pattern`
and `ty`. Cloning, comparing and dropping a module never recurse through
source nesting. Entries keep source order and separate items from lines.
Every node, name, field, argument and operator keeps its byte span.

| Form | Node |
| --- | --- |
| `( )` not touching a callee; `{ }` | group and block, each a line sequence; `()` is the unit literal |
| `f a -l -c=80`, `f(a, b)` | call with callee, argument sequence and written style; `-c 80` is a flag then an operand |
| `e.name`, `.name`, `(e).name` | qualified name kept in segments; selector; postfix field |
| operators, pipes | distinct binary and pipe nodes with operator spans |
| `read<I64>`, `Bits<8>` | instance or named type: name and type arguments, a type or an integer |
| `[:e]`, `[!]` | outcome node with side and optional payload, in values and patterns |
| `Point { x, y: 2 }` | record: uppercase head and fields, a punned field holding a name |
| `pub p: T = v` | binding: visibility, pattern, optional annotation, value |
| `a (b = 0) if g => body` | arm: head patterns, guard, body; the default is a pattern atom; an arm list is the function |
| `(a, b)` | tuple, in values and patterns |
| `A -> B`, `A * B`, `A & B`, `A | B`, `!A` | curried function type; tuple type; both, either and not |
| `Show a => T`, `b.age: a => T`, `(x: T = v) -> U` | qualified type with constraints; labeled parameter with a default |
| `use`, `type`, `impl`, `name: T` | items with visibility; declarations keep parameters, constraints, a base and limits; a `type` without a body has no member list |

No parser rule decides which spot an argument fills, whether a dot is a
field or a module, or whether a declaration form is allowed; checking
owns those.

## Limits and recovery

At most 64 nested frames; deeper input is `NestingLimit`. Flat lists,
pipelines, operator and postfix chains cost no frame per element. Rules
inspect at most three tokens ahead, except the binding rule, which scans
the current line for a depth-zero `=`.

Recovery resumes at a retained newline that ends a line, at the enclosing
closer or at EOF; a carried-over newline is not a recovery point. A missing
comma before a line break inside a list is `CommaBeforeLineBreak`, spanning
the newline. Reasons and spans are stable; wording is the host's.
