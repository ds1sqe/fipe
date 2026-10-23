# Embedding, modules and backends

A host embeds Fipe by providing declarations, environment, an optional
budget and input/output. No editor, OS or transport is prescribed. H1 in
[open decisions](11-open.md) holds what conformance still needs.

## Compilation units

A compilation unit is one complete line, or a module file with its resolved
imports. A file is a module named by its stem. A host may register declaration modules
with dotted names, such as `text.open`; these form nested host namespaces.
The host resolves file modules and loads sources before
evaluation. It supplies each module's identity, its stem in this draft:
one identity is one module within a run and a session.
Items are collected before the executable top-level lines, the implicit
main. The whole unit checks before any effect; a source, API or version
that fails to resolve is a check error. A Rust driver supplies lines and
file sources through the [embedding API](../api/09-embed.md).

`use module` runs a module once per unit, where its first `use` stands:
its bindings in order, effects included; a later `use` of it only brings
names into scope. `use module n` requests a host-declared API version.
A `use` path is followed through its module segments: `use text.open` binds
`open` to the submodule of `text`; `use text.open.new` imports its public
`new` declaration. Existing `use text.trim` imports `trim` when it is a
public declaration rather than a submodule. `use text` binds the parent,
so `text.open.new` names the same declaration. A module import binds its
last segment, not the module's exports. Several paths may follow `use`. A session line whose unit
dispatches on a session meaning no earlier line fixed is the check error
naming it; any other line runs, and a value once computed is never
recomputed when a later line fixes the meaning. A session line whose run
Faults is withdrawn: its names, types and claims are as if it had not been
given, and its effects stand. A used module holds items and bindings
only; another top-level line in it is the check error "a used module has
no main", and a `use` that reaches its own module is the check
error "import cycle" at that `use`. What `pub` exports is
[syntax](02-syntax.md)'s §Definitions; a module exports its final bindings, a
name when its final binding is `pub`, and an exported host-callable
signature is monomorphic. This draft has no `mod`.

## Nested host namespaces

A host registers each module by its full dotted identity, for example
`Declarations::module("text.open", source)`. Every segment must be a lowercase
Fipe name, with no empty segment or surrounding whitespace. `prelude` and
`commands` are reserved root identities. Missing parents are empty namespaces;
a host may supply declarations for a parent before or after its children.

Submodules are visible namespace members. A submodule and a declared value
cannot share a name in the same parent; registration rejects either ordering
of that collision without replacing existing declarations. Replacing a valid
module preserves its children. Export privacy still applies to declarations
inside a module. A missing child or private export is a check error.

For a host supplying `text.open` and `client.open`, these names remain distinct
through checking and execution. A host call's declaration identifies its full
module, so both modules may export `new` without a wire-name collision.

```text
use text.open
text.open.new h
open.new h
```

All three lines above assume the host declares `new` and the program binds
`h`. A local value named `text` shadows that module in expressions; the dot
then accesses value members. This feature adds no `mod` item or filesystem
search convention: a file module is still supplied by its stem. Tool hosts
that load declaration files by filename may use `text.open.fipe` for the
`text.open` identity.

## Host declarations

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
capabilities; the backend validates replies against their declared types,
and a reply that does not fit Faults `Host` "the reply is not a T" at the
call. It
cleans up on success and Fault; a failed cleanup is written to the error
output and changes neither the value nor the Fault. A closure captures
values, not the environment: called after its `with` ended, it sees the
environment open at the call.

## Input, output and observation

A host provides optional byte input and success and error output. Host
input enters the one open spot of the function the unit's last top-level
line makes ([checks](07-checks.md)), decoded by the host to the spot's
type and checked against it as a reply is: a misfit Faults `Host` "the
input is not a T" at that line before the run starts. A last line of
another open arity, or one that is not such a function, is refused before
any effect.
A host may reject implicit top-level output, as a callback-only module
does.

Writing is a declared host command with a value-last spot, never an
operator; values stay typed data until the host's sink policy applies; a
sink that cannot format or write a value fails that command, which Faults
`Host` at the write.
Output formatting, outcome unwrapping, exit codes, streaming and
diagnostic presentation are the host's documented contract, so backends
are compared under one host contract.
The std host is a tool, not part of this specification: `tools/host`
declares its own modules and documents its contract in its
[README](../../tools/host/README.md).

## Budget

The host may supply a budget before a run; a script cannot raise it.
With none, the run is unbounded. With one, the run Faults `Budget` before
the first action it cannot pay, with that action's source location;
earlier effects stand, and no runtime check or member call escapes the
budget. What one unit measures is the backend's and is documented with
it: the VM's is in the [MIR chapter](../api/08-mir.md) §Running. A unit
common to every backend, and limits on memory or time, are open (H1).

The host also sets the compile budget and the strict session
([checks](07-checks.md) §Modes). The compile budget bounds each
compile-time evaluation in the checker's own steps, 2^20 unless the host
sets another; it is part of the
checked unit, as host constants are, so every backend receives the same
acceptance.

## Backend equivalence

Given the same checked unit, host responses, input and environment, a run
that no budget stops produces the same observation on every conforming
backend:

```text
observation = (returned value OR source-located Fault,
               ordered host-call/effect trace,
               success/error output bytes)
```

Where a budget stops a run is the backend's.

Addresses, allocation and instruction layout are not observations; a
backend may optimize only while preserving the observation. The checker
emits one typed intermediate form. The VM is the first backend and the
oracle; the native backend (`tools/native`, x86-64 Linux) is the second,
and its runs equal the VM's observation and units. An LLVM backend would
be a separate package.
Host nondeterminism is open in H1.
