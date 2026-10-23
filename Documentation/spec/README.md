# Fipe language specification

Fipe means Functional Pipeline Language: embedded, extensible,
functional, typed and compiled. Its path is source → tokens → syntax
tree → checked, typed intermediate form → backend, and no effect occurs
before the whole unit checks. The prelude is part of the language; host
APIs are supplied declarations.

This specification describes the experimental `0.2.0-dev.2` language. The
implementation includes parsing, type inference, static checks, a VM and
native execution. The [conformance chapters](10-conformance.md) describe the
required observations; [open decisions](11-open.md) identify rules that remain
unresolved. Publishing the packages does not make those proposals final.
Published `0.1.0` is the earlier dynamic language and does not conform.

Start with the [tour](../tour/README.md) for runnable scripts, the
[examples index](../examples/README.md) for embedding examples, or the
[embedding API](../api/09-embed.md) for Rust integration. Cargo workspace and
release configuration are described in [development](../development.md) and
[publishing](../publishing.md); they are tooling concerns, not language syntax.

## Rule owners

The chapters are numbered in the order of the path, and each rule lives
in one chapter; the others cite it. The Rust boundary of each stage is
under [api](../api/), numbered by the chapter it serves; its names are
not language rules. Parsing never depends on checking.

| Chapter | Owns |
| --- | --- |
| [01 Lexical](01-lexical.md), [EBNF](01-lexical.ebnf) | characters, tokens, literals, docs, spans, recovery |
| [02 Syntax](02-syntax.md), [EBNF](02-syntax.ebnf), [corpus](02-syntax.fipe) | grammatical forms, precedence, disambiguation |
| [03 Types](03-types.md) | meanings, type forms and signs, identity, inference, fields, constructors, defaults, outcome types |
| [04 Members](04-members.md) | members and member limits, the dot, derived members, packed types, conversions, `Form` and `Literal`, the number types |
| [05 Containers](05-containers.md) | tuples, arrays, `Vec` and `Deque`, brackets, type parameters and their sums |
| [06 Limits](06-limits.md) | the four kinds of limit, declaration, `Self`, gates, coverage by a gate and its `!`, `Bits` and `Size` |
| [07 Checks](07-checks.md) | phases, entry and outcome, modes, timing, the plan, what is known, order and charges |
| [08 Evaluation](08-evaluation.md) | application, pipelines, outcomes, pattern binding |
| [09 Host](09-host.md) | compilation units, name lookup, host services, budget, backends |
| [10 Conformance](10-conformance.md), [checks](10-conformance-checks.md), [corners](10-conformance-corners.md) | observables and required discriminating cases |
| [11 Open](11-open.md) | proposals that are not yet language rulings |

| API | Serves |
| --- | --- |
| [01 Lexer](../api/01-lexer.md) | Rust token and diagnostic representation, source lookup |
| [02 Parser](../api/02-parser.md) | owned syntax nodes, parser diagnostics and limits |
| [09 Embed](../api/09-embed.md) | the request-and-reply boundary, sessions and their reports |

An open decision overrides the apparent completeness of a rule that cites
it; a candidate spelling chosen in this draft is not approval.

## Terms and notation

A **source** is UTF-8 text. A **token** has a kind, optional payload and a
half-open byte span. A **line** is a language expression, possibly spanning
physical lines. A **stage** is one part of a pipeline. A **spot** is one
parameter awaiting a value. A **module** is one source file and its
exports. A **meaning**, **limit**, **type** and **place** are defined in
[types](03-types.md). A **host** supplies declarations and effects. A **run**
evaluates a checked unit under one environment and, when the host gives
one, a budget.

`Γ` is a static environment, `ρ` a value environment; `τ` ranges over
types, `σ` over type schemes, `e` over expressions, `v` over values.
`Γ ⊢ e : τ` means the expression checks with that type; `e ⇓ v` is
evaluation, `e ⇑ F` an uncatchable Fault. **Must** is required, **may**
permitted. Examples are illustrative unless listed as conformance cases;
implementation strategy, Rust API names and diagnostic wording are not
language rules unless specified.

## Failures

1. A lexical or syntax error rejects source structure.
2. A check error rejects a statically invalid program before effects,
   including a refuted limit ([checks](07-checks.md)).
3. A Fault ends an executing run: budget, cast, arithmetic, a failed test,
   host or explicit `fault`. A script cannot catch it.
4. `Err e` and `Nothing` are ordinary values, not Faults.

Diagnostics identify a source and span; independent errors may be
collected, but a unit with any error never executes.
