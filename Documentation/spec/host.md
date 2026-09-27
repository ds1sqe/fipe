# Embedding, modules and backends

A host embeds Fipe by providing external declarations, environment access,
execution budget and input/output. No editor, OS or transport is prescribed.
H1 in [open decisions](open.md) covers details still needed for conformance.

## Compilation units

A compilation unit is one complete line supplied by a host, or a complete
module file and its resolved imports. A file is a module named by its stem.
Items are collected before checking executable top-level lines as an implicit
main. All code in the unit checks before any of its program effects occur.
Declaration discovery and source loading are host preparation, not script
evaluation. Failure to resolve source, an API or a version is a check error.

`use module` imports a module once; `use module ver n` requests a host-declared
API version. Files have no written version; hosts supply identity for caching.
A function with a complete written signature is exported; an inferred one is
private. An exported host-callable signature is monomorphic. T3 defines the
exact annotation requirement; H1 defines import identity and cycle handling.

External declarations and file exports use the same language type system.
A declaration describes data types, functions, parameter names/defaults and
effect information. No Rust type, Kernel type or transport byte encoding is
part of Fipe type identity unless admitted by that declaration interface.

## Name lookup

For a name in expression position, search nearest lexical binding, then an
imported/host-declared API name, then the host's command namespace. Shadowing
is lexical. `cat x` uses a bound `x`; `cat "x"` forces text. A qualified name
may be a module member or field chain; checking resolves its first segment
then verifies every following segment. Unknown fields are check errors.

The prelude is implicit and contains the language's primitive traits,
constructors and functions. Names such as `fs`, `sys`, `cli`, `root` and `env`
are host declarations/bindings, not universal reserved words. An embedding
must provide any such name used by its program. Host PATH is a policy for
the command namespace, not an OS environment requirement.

## Environment and resources

The host environment is separate from lexical bindings. `let cwd = x` never
changes the host environment. `with key value { body }` evaluates its body
under a child environment whose lifetime is that evaluation; it is not an
assignment followed by restoration. Lookup, mutation, fallback and permissions
are declared host operations. A `with` level does not mutate its parent.

A resource value carries identity, not an owned handle. The host opens,
caches and closes execution capabilities and validates its replies against
their declared language types. Capability cleanup must occur on success and
Fault. Cleanup failures and environment capture by escaping closures are H1.

## Input, output and observation

A host provides an optional byte input and success/error output facilities.
An open one-argument top-level function may take host input through its spot's
cast policy; missing input or a different open arity is a check/run admission
error. A host may instead reject all implicit top-level output, as for a
callback-only module.

Returned values remain typed data until the host applies a sink policy.
Output formatting, whether to unwrap outcomes, process exit codes, streaming
and diagnostic presentation belong to that host's documented contract.
For backend comparison, fix one host contract rather than comparing hosts
whose presentation policies intentionally differ.

## Budget

The host supplies a finite budget before a run. A script cannot increase it
for the current run. Exhaustion produces a Fault before the chargeable action
whose cost cannot be paid. Runtime checks, dictionary calls and external
data processing cannot bypass charging by selecting a different backend.

The prior design calls for one unit per evaluated node and per external byte.
H1 must define the authoritative semantic nodes, charge points and byte count.
Charging raw VM instructions would make native/backend parity dependent on
optimization and is not an accepted substitute.

## Backend equivalence

Given the same checked unit, host responses, input, environment and budget,
all conforming backends must agree on:

```text
observation = (returned value OR source-located Fault,
               ordered host-call/effect trace,
               success/error output bytes,
               budget charges)
```

Internal addresses, allocation strategy and instruction layout are not
observations. A backend may optimize only while preserving this observation;
it cannot remove a specified charge or reorder visible host effects. Float
observations and host nondeterminism require E2/H1's final contracts.

The checker emits one typed intermediate form. The VM is the first backend
and the differential oracle for a later native backend. An LLVM backend, if
provided, is an optional separate crate. Conforming language programs do not
depend on which backend is selected.
