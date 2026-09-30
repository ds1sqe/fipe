# Fipe language specification

Fipe means Functional Pipeline Language: embedded, extensible, functional,
typed and compiled. Its path is source → tokens → syntax tree → checked,
typed intermediate form → backend, and no effect occurs before the whole
unit checks. The prelude is part of the language; host APIs are supplied
declarations. Lexical structure and syntax are defined and implemented.
The checker reports each form of the type model in [types](types.md),
[containers](containers.md), [limits](limits.md), [checks](checks.md) and
[members](members.md) that it does not check yet as one check error;
members ([members](members.md)) are checked, and there is no `trait`
item. Checking,
evaluation and embedding are the draft for the later stages, with their
gaps in [open decisions](open.md). Published 0.1.0 is the earlier
dynamic language and does not conform.

## Rule owners

Each rule lives in one chapter; the others cite it.

| Chapter | Owns |
| --- | --- |
| [Lexical structure](lexical.md), [EBNF](lexical.ebnf) | characters, tokens, literals, spans, recovery |
| [Lexer API](lexer-api.md) | Rust token/diagnostic representation and source lookup |
| [Syntax](syntax.md), [EBNF](syntax.ebnf) | grammatical forms, precedence, disambiguation |
| [Parser API](parser-api.md) | owned syntax nodes, parser diagnostics and limits |
| [Types](types.md) | meanings, type forms and signs, identity, inference, fields, constructors, defaults, outcome types |
| [Containers](containers.md) | tuples, arrays, `Vec` and `Deque`, brackets, type parameters and their sums |
| [Limits](limits.md) | the four kinds of limit, declaration, `Self`, gates, coverage by a gate and its `!`, `Bits` and `Size` |
| [Checks](checks.md) | phases, entry and outcome, modes, timing, the plan, what is known, order and charges |
| [Members](members.md) | members and member limits, the dot, derived members, packed types, conversions, `Form` and `Literal`, the number types |
| [Evaluation](evaluation.md) | application, pipelines, outcomes, pattern binding |
| [Embedding](host.md) | compilation units, name lookup, host services, budget, backends |
| [Embedding API](embed-api.md) | Rust request-and-reply boundary, sessions and their reports |
| [Conformance](conformance.md) | observables and required discriminating cases |
| [Open decisions](open.md) | proposals that are not yet language rulings |

An open decision overrides the apparent completeness of a rule that cites
it; a candidate spelling chosen in this draft is not approval.

## Terms and notation

A **source** is UTF-8 text. A **token** has a kind, optional payload and a
half-open byte span. A **line** is a language expression, possibly spanning
physical lines. A **stage** is one part of a pipeline. A **spot** is one
parameter awaiting a value. A **module** is one source file and its
exports. A **meaning**, **limit**, **type** and **place** are defined in
[types](types.md). A **host** supplies declarations and effects. A **run**
evaluates a checked unit under one environment and budget.

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
   including a refuted limit ([checks](checks.md)).
3. A Fault ends an executing run: budget, cast, arithmetic, a failed test,
   host or explicit `fault`. A script cannot catch it.
4. `Err e` and `Nothing` are ordinary values, not Faults.

Diagnostics identify a source and span; independent errors may be
collected, but a unit with any error never executes.
