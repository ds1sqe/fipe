# Parser API

[Syntax](../spec/02-syntax.md) owns the language rules; this chapter owns the Rust
representation and the implementation limits.

## Entry point

~~~text
parse::parse(input: lex::Lexed) -> Result<ast::Module, Vec<parse::Error>>
parse::parse_partial(input: lex::Lexed) -> ast::Module
Error { span: token::Span, reason: parse::Reason }
ast::Module::at(&self, offset: usize) -> Option<ast::Node>
ast::Node = Expr(ExprId) | Pattern(PatternId) | Type(TypeId)
~~~

The parser consumes owned tokens and does no rescanning, lookup, checking
or I/O. Lexical errors come back as `Reason::Lexical` without parsing.
Any error rejects the whole result; recovery only collects more errors.
`parse_partial` keeps what it read: each entry it reads whole is in
`entries`, and each other one, an entry with an error or an error
recovered inside it, is an error node, its span in `malformed` in source
order; `errors` holds the diagnostics, lexical ones first, and an entry
holding a lexical error is an error node too. An error node binds
nothing, and a module with errors is not a compilation unit: `check`
refuses it. `parse` is `parse_partial` with no error, and returns only
the lexical errors when there are any, as before. An invalid token
stream is an empty module with that one error. `at` is the innermost
node whose span holds a byte offset, `start <= offset < end`, an
expression before a pattern before a type of one span; an error node's
nodes are not reached.
`Lexed` is publicly constructible: a missing, repeated or nonempty-span
EOF, or overlapping or reversed spans, is `InvalidTokenStream`, never a
panic. Comment spans must be nonempty, ordered, within EOF and disjoint
from tokens. An empty file is a module; the module span runs from byte zero
through EOF, including a skipped BOM.

## Representation

A module owns expressions, patterns and type syntax in three flat arenas,
reached by `ExprId`, `PatternId` and `TypeId` through `expr`, `pattern`
and `ty`. Cloning, comparing and dropping a module never recurse through
source nesting. Entries keep source order and separate items from lines.
Every node, name, field, argument and operator keeps its byte span.
The module keeps `comments: Vec<Span>` from the lexer for a later formatter.
The module, items, fields, constructors, impl members and `Bind` expressions
keep `doc: Option<String>`. A `use` item and constructor fields have none.
Docs do not participate in lookup or checking except declaration spot lists.
`UnattachedDoc` diagnoses an outer doc without an owner or mixed forms;
`InnerDocPosition` diagnoses a module doc after an entry or inside a body.

| Form | Node |
| --- | --- |
| `( )` not touching a callee; `{ }` | group and block, each a line sequence; `()` is the unit literal |
| `f a -l -c=80`, `f(a, b)` | call with callee, argument sequence and written style; `-c 80` is a flag then an operand |
| `e.name`, `.name`, `(e).name` | qualified name kept in segments; selector; postfix field |
| operators, pipes | distinct binary and pipe nodes with operator spans |
| `read<I64>`, `Bits<8>`, `[T; n]` | instance or named type: name and type arguments, each a type or a number node; an array's length is a number, a name or a flat `TypeKind::Sum` of them |
| `10ms`, `re"a"i`, `b"…"` | literal holding the token's `Affixed` |
| `[:e]`, `[!]` | outcome node with side and optional payload, in values and patterns |
| `Point { x, y: 2 }` | record: uppercase head and fields, a punned field holding a name |
| `pub p: T = v` | binding: visibility, pattern, optional annotation, value |
| `a (b = 0) if g => body` | arm: head patterns, guard, body; the default is a pattern atom; an arm list is the function |
| `(a, b)` | tuple, in values and patterns |
| `A -> B`, `A * B`, `A & B`, `A | B`, `!A` | curried function type; tuple type; both, either and not, each sign one flat node |
| `Show a => T`, `b.age: a => T`, `(x: T = v) -> U` | qualified type with constraints; labeled parameter with a default |
| `use`, `type`, `impl`, `name: T` | items with visibility; declarations keep parameters, constraints, a base and limits, each limit a member limit or a test; a `type` without a body has no member list; an impl keeps `claim: Option<TypeId>`; an impl member may have no value |

No parser rule decides which spot an argument fills, whether a dot is a
field or a module, or whether a declaration form is allowed; checking
owns those.

## Limits and recovery

At most 64 nested frames; deeper input is `NestingLimit`. Flat lists,
pipelines, operator and postfix chains cost the parser no frame per
element, but each element counts toward its expression's depth: `a + b +
c` is three deep. An expression deeper than `MAX_DEPTH` (256) is
`DepthLimit` at the operator or node that passed it, since every later
stage walks the tree; on a 2 MiB stack each stage handles the deepest
expression admitted, in debug and release builds. Rules
inspect at most three tokens ahead, except those that scan for a
depth-zero token: the binding, arm and constraint rules to the line's
end, and the tuple rule to its bracket's closer.

Recovery resumes at a retained newline that ends a line, at the enclosing
closer or at EOF; a carried-over newline is not a recovery point. A missing
comma before a line break inside a list is `CommaBeforeLineBreak`, spanning
the newline; `|!` in a type is `EitherNot`. Reasons and spans are stable;
a reason's `Display`, `parse::Expected`'s included, is its words.

Integer forms retain radix and written digit count, excluding underscores
and the sign or radix prefix; leading zeros count. The syntax literal
keeps the same metadata for the checked literal Form.
