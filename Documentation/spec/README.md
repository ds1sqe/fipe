# Fipe language specification

Fipe means Functional Pipeline Language. Plan 01.0 defines lexical structure;
the remaining chapters are the formal draft for subsequent language stages.

The development crate implements the lexical contract. Published 0.1.0 is
the earlier dynamic language and does not conform to this specification.
The syntax and semantic draft still includes [open decisions](open.md) for
the parser/checker/runtime stages. They do not qualify lexical conformance.

## Scope

Fipe is embedded, extensible, functional, typed and compiled. Its compilation
path is source → tokens → syntax tree → checked, typed intermediate form →
backend. No program effects occur before its entire compilation unit checks.
The prelude is part of the language; host APIs are supplied declarations.

The language is independent of an editor, operating system, resource tree,
transport encoding and user interface. The implementation crate has no
dependencies; its portable core uses `core` and `alloc`. Those implementation
constraints do not give Rust types authority over language semantics.

## Rule owners

| Chapter | Owns |
| --- | --- |
| [Lexical structure](lexical.md), [EBNF](lexical.ebnf) | characters, tokens, literals, spans, recovery |
| [Lexer API](lexer-api.md) | Rust token/diagnostic representation and source lookup |
| [Syntax](syntax.md), [EBNF](syntax.ebnf) | grammatical forms, precedence, disambiguation |
| [Types](types.md) | type identity, inference, traits, defaults, cast checking |
| [Evaluation](evaluation.md) | application, pipelines, outcomes, pattern binding |
| [Embedding](host.md) | compilation units, modules, host services, budget, backends |
| [Conformance](conformance.md) | observables and required discriminating cases |
| [Open decisions](open.md) | proposals that are not yet language rulings |

An open decision overrides the apparent completeness of a production or rule
that cites it. The draft chooses a candidate spelling to make review concrete;
choosing it in this draft is not user approval.

## Terms and notation

A **source** is UTF-8 text. A **token** has a kind, optional payload and a
half-open byte span. A **line** is a language expression, possibly spanning
physical source lines. A **stage** is one part of a pipeline. A **spot** is one
parameter awaiting a value. A **module** is one source file and its exports.
A **host** supplies external declarations and effects. A **run** evaluates a
checked unit under one environment and budget.

`Γ` is a static environment; `ρ` is a value environment. `τ` ranges over
types, `σ` over polymorphic type schemes, `e` over expressions and `v` over
values. `Γ ⊢ e : τ` means the expression checks with that type. `e ⇓ v`
means successful evaluation; `e ⇑ F` means an uncatchable Fault.

**Must** states a required property; **may** states a permitted choice.
Examples are illustrative unless listed as conformance cases. Implementation
strategy, allocation layout, Rust API names and diagnostic wording are not
language rules unless explicitly specified.

## Failures

1. A lexical or syntax error rejects source structure.
2. A check error rejects a statically invalid program before effects.
3. A Fault terminates an executing run: budget, cast, arithmetic, host or
   explicit `fault`. A script cannot catch it.
4. `Err e` and `Nothing` are ordinary values, not Faults.

Diagnostics identify a source and span. Collecting independent errors is
permitted; executing a unit with any lexical, syntax or check error is not.

## Status

The [source corpus](examples.fipe) is exercised by the public lexer tests.
The complete opening example has an independent token oracle. Those tests
and document checks establish lexical behavior, not parsing, type soundness
or backend equivalence. Those are the following implementation milestones.
