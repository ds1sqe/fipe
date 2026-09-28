# Parser API

This chapter specifies the parser boundary. The syntax chapter
owns language rules; this API owns representation and implementation limits.
Public conformance tests exercise this boundary and the syntax corpus.

## Entry point

~~~text
parse::parse(input: lex::Lexed) -> Result<ast::Module, Vec<parse::Error>>
Error { span: token::Span, reason: parse::Reason }
~~~

The parser consumes owned tokens and performs no source rescanning, name
lookup, type checking or I/O. Existing lexical diagnostics are returned as
lexical reasons, without trying to parse an invalid token stream. An empty
file is a valid module. Any syntax diagnostic rejects the whole result;
recovery collects further errors but never returns a partial executable AST.

`Lexed` is publicly constructible. Missing/repeated EOF, overlapping or
reversed spans, and nonempty EOF spans produce `InvalidTokenStream` rather
than a panic. For normal use, supply the result of `lex::lex` unchanged.
The module span covers byte zero through EOF, including a skipped initial BOM.

## Syntax representation

A module owns expressions, patterns and type syntax in three flat arenas.
`ExprId`, `PatternId` and `TypeId` are distinct references scoped to that module;
`expr`, `pattern` and `ty` return optional references to their nodes. Nodes
refer to children by these IDs. Cloning, comparing and dropping a module do
not recurse through the source nesting or through a long pipeline/call chain.

Module entries preserve source order and distinguish items from expression
lines. Every expression, pattern, type, item, name, field and argument retains
its byte span. Operators have their own spans as well as enclosing expression
spans. A group node is a parenthesized line sequence and a block node a
braced one; both keep line order, and a unit literal is distinct.

Names retain their qualified segments. `e.name` may denote field access or a
module-qualified name; the checker resolves it. A selector `.name` is its own
node. Postfix `(e).name` explicitly selects fields from an expression.

Calls retain their callee, argument sequence and written call style. A flag
has its name and optional attached expression; `-c 80` is a flag followed by
a positional expression, while `-c=80` is one flag argument with a value.
No parser rule decides which declared parameter an argument fills.

Binary operators and pipes are distinct nodes. A name with touching type
arguments is an instance node holding the name and its type syntax; a bare
type name is a name. An outcome bracket is a node with its side and optional
payload, in expressions and in patterns. A record node holds its uppercase
head and field values, a punned field holding a name. A binding retains its
visibility, pattern, optional annotation and value; there is no function
node, an arm list is the function. An arm retains its head patterns, guard
and body; a head atom `(name = operand)` is a default pattern. Function
types are curried pairs. Imports (names and an optional version), types
(fields and constructors), traits (typed members with optional defaults) and
implementations (named members and constructor arms) are items, each with
its visibility. Type parameters are retained; checking owns prelude-only use.

## Progress and limits

The implementation accepts at most 64 nested parsing frames and reports
`NestingLimit` when exceeded. This is an implementation resource limit, not
a language type rule or the later execution budget. Flat argument lists,
pipelines, operators and postfix chains do not consume one frame per element.

Recovery advances to a retained newline that ends a line, a containing
closing bracket or EOF; the containing parser owns its closing bracket. A
newline that an operator carries over is not a recovery point. Malformed items/lines are not
placed in the returned module because a module is returned only on success.
Diagnostic wording is host-owned; typed reasons and byte spans are stable.

A missing comma before a line break inside a list is its own diagnostic,
spanning the newline. Continuation, the record rule and the default-pattern
atom inspect at most three tokens ahead; the binding rule scans the current
line for a depth-zero `=`. No rule performs a lookup.
