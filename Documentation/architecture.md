# Layer ownership

Core → Embed → Host → Standard names logical ownership boundaries, not four
required crates. Dependencies point inward: Standard implements the Host contract
and uses Embed and Core. An application's Host is an alternative to Standard.

| Layer | Owns | Implementation and dependencies |
| --- | --- | --- |
| Core | Language rules, checking, compilation and evaluation | `fipe`; uses only `core` and `alloc` |
| Embed | Driver-facing requests, replies, outcomes, sessions and stepping; exposes line values for the driver | `fipe`; uses Core. Optional `fipe-repl-core` supplies a REPL state machine without I/O |
| Host | Declarations, source lookup, answering calls, permission and resource policy, and presentation of values | The request/reply contract is available from `fipe` alone; an application implements it through Embed, without a mandatory Rust trait |
| Standard | OS-backed source loading, filesystem and environment operations, write roots and sink policy | `fipe-host`; depends on `fipe` and uses `std` |

Core, Embed and the Host contract are `no_std` + `alloc` and perform no OS
I/O: they do not read paths, environment variables or clocks. `no_std` alone
would not guarantee this; no OS effects is a separate boundary. Fipe returns a
`Request` instead of calling the host; the driver supplies a `Reply` and resumes
execution through an `Outcome`. See the [embedding API](api/09-embed.md).
A source request's stem is a module identity; only the host decides whether it
maps to a filesystem path, an in-memory source or an application resource.

Only `prelude` and `commands` are reserved root identities. Standard's `text`,
`fs`, `sys` and `cli`, including its `Rd` constructor, are its declarations, not
requirements of the Host contract. Calls retain their full namespace identity,
so an application may supply its own `text.open` and `fs.fs`. See the
[host specification](spec/09-host.md#nested-host-namespaces).

## Standard CLI path

`fipe run` in `fipe-cli` uses `fipe-host` to load and check sources and run the
VM. Core checks and evaluates; Embed returns call requests and line values;
Standard answers with OS operations and applies its sink policy. Standard owns
its [write roots and output rules](../tools/host/README.md), rather than Core
or Embed granting OS access. `fipe repl` also selects Standard.

## Application Host path

An application such as Reovim depends on `fipe` alone, supplies its own
declarations and drives `Session` and `Machine` through requests and replies.
Its adapter resolves source requests and routes calls through its Kernel, which
owns permission checks and handles. Standard is not required. Line values remain
data for the application's output presentation; session acceptance and VM
execution are separate operations. Optionally, the application can use
[`fipe-repl-core`](../tools/repl-core/README.md) as its line loop, servicing its
output, source and call events through its own driver.

This describes the embedding boundary, not a verified Reovim integration.
Opaque resources use checked typed aliases; the Host owns private payloads,
per-call permission and liveness checks, and its creation ledger. Drivers handle
execution boundaries and Host cleanup under the
[embedding resource and lifecycle contract](api/09-embed.md#opaque-resource-references)
and [Host specification](spec/09-host.md#environment-and-resources).
Dropping a language alias does not automatically dispose of its payload.

## Current tooling and support goals

The live DAP and native runtime currently depend on Standard; they are not
generic application-Host adapters. The LSP does not depend on `fipe-host`.
The [native runtime](../tools/native/runtime/README.md) currently targets x86-64
Linux. Linux, Windows and macOS form the release target matrix under preparation,
not a claim of current verified support across all three systems or backends.
