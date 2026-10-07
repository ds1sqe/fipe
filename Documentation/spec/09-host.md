# Embedding, modules and backends

A host embeds Fipe by providing declarations, environment, budget and
input/output. No editor, OS or transport is prescribed. H1 in
[open decisions](11-open.md) holds what conformance still needs.

## Compilation units

A compilation unit is one complete line, or a module file with its resolved
imports. A file is a module named by its stem, and a nested namespace is
another file; the host resolves the tree,
supplies module identity for caching, and loads sources before evaluation.
Items are collected before the executable top-level lines, the implicit
main. The whole unit checks before any effect; a source, API or version
that fails to resolve is a check error. A Rust driver supplies lines and
file sources through the [embedding API](../api/09-embed.md).

`use module` runs a module once per unit, where its first `use` stands:
its bindings in order, effects included; a later `use` of it only brings
names into scope. `use module n` requests a host-declared API version;
`use module.name …` brings names into scope. A used module holds items and
bindings only; another top-level line in it is the check error "a used
module has no main", and a `use` that reaches its own module is the check
error "import cycle" at that `use`. What `pub` exports is
[syntax](02-syntax.md)'s §Definitions; a module exports its final bindings, a
name when its final binding is `pub`, and an exported host-callable
signature is monomorphic. This draft has no `mod`. H1 defines import identity.

A host declares in Fipe, with the declaration forms of
[syntax](02-syntax.md): one file per host module, named by the module, and
one for the command namespace, holding bare commands, bindings and the
types they share. A declaration is a signature with labeled spots and
defaults, an opaque type, a type, which may carry limits, or an impl; a
name or impl member with no value is implemented by the host. A host
declaration with a written value, such as `pub max_port = 65535`, is
an exact compile-time constant and belongs to the checked unit. No Rust,
kernel or transport type is part of Fipe type identity unless a
declaration admits it. A host default is typed in its declaring module
when a stage first needs it; a failure names its spot at that stage. A
program default is checked at its declaration.

A declaration file keeps Markdown docs on its tree. After optional
indentation, `- ` then a one-segment lowercase name and `:`, then a space or
the line's end, documents a spot, as `- depth: …`; other heads, including
`*`, are opaque. Every declaration file (prelude, commands and host modules)
checks these names on top-level signatures and supplied impl members against
their labeled spots, including `self`. An unknown spot is a declaration
error naming it. Program docs are never checked; nothing else reads docs for
a rule. The prelude and every host declaration carry a doc. The REPL prints
a name's doc above its type with `:t name`.

## Name lookup

Bindings and `use` are sequential in every scope: a name is visible from
its line onward, so a use above its binding or `use` does not see it;
`type` and `impl` are order-free, and the members of one `impl` see each
other. For a name in expression position: the nearest lexical binding,
then this module's items and imports, then the prelude, then a host
module's name, then the host's command namespace. An unbound name in an
argument position is text; anywhere else it is the check error "unknown
name". Shadowing is lexical: `cat x` uses a bound `x`, `cat "x"` forces
text. One dot serves every namespace. For `x.a`, resolve `x` as above,
then [members](04-members.md) §The dot; `m.T.a` reads the namespace of the
module `m`'s type `T`, as `T.a` does. A lowercase type argument naming a
`use`d module, `fs` in `Rd<fs>`, is an API reference ([types](03-types.md)
§Type forms). A value named `fs` hides the module `fs` in its scope, so
`fs.list` on a value without that field is a check error, never a module
call. `::` has no meaning in this draft and stays free for a later `mod`
and namespace form such as `std::fs`.

The prelude is implicit: the language's primitive types, members,
constructors and functions. `fs`, `sys`, `cli`, `root`, `env`, `Rd`, `put`
and `append` are host declarations, not reserved words; the conformance
host declares exactly those the corpus uses. Host PATH is a policy for
the command namespace, not a requirement.

## Environment and resources

The host environment is separate from bindings; `cwd = x` never changes
it. `with key value { body }` evaluates its body under a child environment
for that evaluation only; lookup, mutation, fallback and permissions are
declared host operations.

A host with resources declares an opaque nominal type and the functions
that make one from a number or word or refine it to an API, as the
conformance host's `Rd` and `Rd<fs>`; the language names none. A resource
value carries identity, not a handle. The host opens, caches and closes
capabilities, validates replies against their declared types, and cleans
up on success and Fault. Cleanup failures and capture by escaping closures
are H1.

## Input, output and observation

A host provides optional byte input and success and error output. An open
one-argument top-level function may take host input, which enters its
spot ([checks](07-checks.md)); a different open arity is an admission error.
A host may reject implicit top-level output, as a callback-only module
does.

Writing is a declared host command with a value-last spot, never an
operator; values stay typed data until the host's sink policy applies.
Output formatting, outcome unwrapping, exit codes, streaming and
diagnostic presentation are the host's documented contract, so backends
are compared under one host contract.

## Budget

The host supplies a finite budget before a run; a script cannot raise it.
Exhaustion is a Fault before the chargeable action that cannot be paid,
and no backend, runtime check or dictionary call bypasses charging. H1
defines the semantic charge points and byte count; raw VM instructions are
not an accepted measure, since parity would then depend on optimization.
The host also sets the compile budget and the strict session
([checks](07-checks.md) §Modes); both are inputs of the observation, and the
compile budget is part of the checked unit, as host constants are.

## Backend equivalence

Given the same checked unit, host responses, input, environment and budget,
every conforming backend produces the same observation:

```text
observation = (returned value OR source-located Fault,
               ordered host-call/effect trace,
               success/error output bytes,
               budget charges)
```

Addresses, allocation and instruction layout are not observations; a
backend may optimize only while preserving the observation. The checker
emits one typed intermediate form. The VM is the first backend and the
oracle for a later native one; an LLVM backend would be a separate crate.
Float observations and host nondeterminism await E2 and H1.
