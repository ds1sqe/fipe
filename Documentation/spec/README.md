# Fipe language specification

Fipe means Functional Pipeline Language: embedded, extensible, functional,
typed and compiled. Its path is source → tokens → syntax tree → checked,
typed intermediate form → backend, and no effect occurs before the whole
unit checks. The prelude is part of the language; host APIs are supplied
declarations. Lexical structure and syntax are defined and implemented;
types, evaluation and embedding are the draft for the later stages, with
their gaps in [open decisions](open.md). Published 0.1.0 is the earlier
dynamic language and does not conform.

## Rule owners

Each rule lives in one chapter; the others cite it.

| Chapter | Owns |
| --- | --- |
| [Lexical structure](lexical.md), [EBNF](lexical.ebnf) | characters, tokens, literals, spans, recovery |
| [Lexer API](lexer-api.md) | Rust token/diagnostic representation and source lookup |
| [Syntax](syntax.md), [EBNF](syntax.ebnf) | grammatical forms, precedence, disambiguation |
| [Parser API](parser-api.md) | owned syntax nodes, parser diagnostics and limits |
| [Types](types.md) | type identity, inference, traits, defaults, cast checking |
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
exports. A **host** supplies declarations and effects. A **run** evaluates
a checked unit under one environment and budget.

`Γ` is a static environment, `ρ` a value environment; `τ` ranges over
types, `σ` over type schemes, `e` over expressions, `v` over values.
`Γ ⊢ e : τ` means the expression checks with that type; `e ⇓ v` is
evaluation, `e ⇑ F` an uncatchable Fault. **Must** is required, **may**
permitted. Examples are illustrative unless listed as conformance cases;
implementation strategy, Rust API names and diagnostic wording are not
language rules unless specified.

## Failures

1. A lexical or syntax error rejects source structure.
2. A check error rejects a statically invalid program before effects.
3. A Fault ends an executing run: budget, cast, arithmetic, host or
   explicit `fault`. A script cannot catch it.
4. `Err e` and `Nothing` are ordinary values, not Faults.

Diagnostics identify a source and span; independent errors may be
collected, but a unit with any error never executes.
