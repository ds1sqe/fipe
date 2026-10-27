# Embed Fipe in your application

Use this walkthrough when your application supplies the effects. It starts with
pure evaluation, then a host call, then a small no_std application Host with memory
sources, private resource payloads and per-call permission checks. You do not need
Standard, a terminal REPL, a native provider or an OS integration to embed Core.

This guide targets published Core `0.2.0-dev.3`, qualified through an outside
registry-only custom Host and no_std consumer. Only Core and REPL-core dev.3 are
published; this does not announce Standard dev.3 or a final release. For the
historical dev.2 pure example, see the [README](../README.md#embed-in-rust).

## Choose who performs effects

| Your application needs | Choose | Who owns effects |
| --- | --- | --- |
| Evaluate checked pure code | `fipe` alone | There are no declared Host effects. |
| Expose application or Kernel operations | `fipe` alone, your declarations and request driver | Your Host owns payloads, permissions, output and cleanup. |
| Run scripts against files, arguments and environment | Optional `fipe-host` (Standard) | Its documented OS operations, sink and policy apply. |
| An input loop without terminal I/O, or the source-continuation query | Optional `fipe-repl-core` | Your driver handles its request and boundary tokens. |

Core and the embedding contract are `no_std` + `alloc`. Standard is an alternative
Host, not something a custom Host must link or inherit. It is not a universal
sandbox or permission check. Its current unmarked `Rd<Api>` path contract is not
the marked `Value::Resource` carrier used by the application example below.

Use matching published Core and Standard versions; do not mix Core dev.3 with
Standard dev.2 or assume a Standard dev.3 package is available. See
[layer ownership](architecture.md), the [Standard contract](../tools/host/README.md)
and the [support policy](development.md#compatibility-and-support).
Native/LLVM executables currently reject marked resource programs; the resource
example below uses the VM.

## Create an independent project

Use installed Rust 1.98.1 or the supported toolchain described in the support
policy. Start in an empty directory outside the Fipe checkout. Copy these files,
creating the destination directories as needed; use your editor or your platform's
copy command. Keep the fixture package name for this walkthrough, since its tests
import that name.

| Pinned source asset | Destination in your project |
| --- | --- |
| [Custom Host manifest](https://github.com/ds1sqe/fipe/blob/fe7f3d6a27b4bb18482becc645ab41e99729ec86/tests/fixtures/custom-host/Cargo.toml.in) | `Cargo.toml` |
| [Actual no_std Host](https://github.com/ds1sqe/fipe/blob/fe7f3d6a27b4bb18482becc645ab41e99729ec86/tests/fixtures/custom-host/src/lib.rs) | `src/lib.rs` |
| [Four contract tests](https://github.com/ds1sqe/fipe/blob/fe7f3d6a27b4bb18482becc645ab41e99729ec86/tests/fixtures/custom-host/tests/contract.rs) | `tests/contract.rs` |
| [Pure program](https://github.com/ds1sqe/fipe/blob/fe7f3d6a27b4bb18482becc645ab41e99729ec86/examples/embed.rs) | `src/main.rs` |

These links pin the example source, including the later custom Host fixture; they
do not imply that the fixture is included in a published crate.

The manifest is a template only to keep the checked-in fixture from becoming a
nested Cargo package. Once copied as `Cargo.toml`, it defines a normal independent
project, `fipe-custom-host-proof`, with exactly one dependency:

```toml
[dependencies]
fipe = "=0.2.0-dev.3"
```

Copy the whole manifest, including its independent `[workspace]`; do not add
`[patch]`, `path`, Git overrides or Standard. Do not import the Host library with a
module path or `include_str!` pointing back into the Fipe checkout. Your copied
application files must stand alone. For your own application, rename the package
and its imports together after this baseline works.

**Do not copy the fixture's Cargo.lock.** That lock was generated for a source-patched
proof and has no registry source/checksum association for Fipe. For published
dev.3, resolve a fresh lock from your external project directory:

```sh
cargo +1.98.1 --config 'resolver.incompatible-rust-versions="fallback"' generate-lockfile
```

Keep this registry lock for subsequent `--locked` commands. Do not change the
pin to dev.2 to make the custom Host compile; the APIs must match the example.

## First run: pure code

With `examples/embed.rs` copied to `src/main.rs`, run:

```sh
cargo +1.98.1 run --locked
```

The program parses, checks and compiles a pipeline, runs a Machine with no Host
declarations, and asserts the result is `Value::Int(12)`. It prints `Int(12)` using
Rust Debug. Standard's textual output policy is not involved. A program that does
not check must not be run; applications should present the diagnostics rather than
copying the example's `expect` calls as an error-handling policy.

## Answer one Host call

Replace only `src/main.rs` with the existing
[custom Host example](https://github.com/ds1sqe/fipe/blob/fe7f3d6a27b4bb18482becc645ab41e99729ec86/examples/custom_host.rs), then run the same command. It
registers `app.greetings`, receives the actual `Request::Call`, checks its arguments,
answers through `Machine::reply` and asserts `Hello, Fipe!`.

Declarations describe available operations; they do not perform them. Core calls
no application callback, loads no provider and reads no file on its own. A Call
names a declaration index with arguments in parameter order. In a persistent
Machine, inspect that declaration using `machine.program()`'s joined tables;
together its namespace and label distinguish equal leaf names. The small one-unit
greeting example uses its single Program's tables.

A Host returns `Reply::Value(Ok(value))` or a failure message. The VM checks the
reply's declared boundary shape; a Host failure becomes a source-located Fault.
Host permission and liveness checks remain necessary even when language type fit
succeeds. [The embedding API](api/09-embed.md#requests-and-replies) owns the precise
request, reply and input contracts.

## Exercise an application-owned Kernel

The `src/lib.rs` copied earlier is the complete small Host implementation. It is
`#![no_std]` with `alloc`, not a no_std wrapper around a std callback. The separate
integration tests supply their test executor and allocator; the Host itself owns
its Session/Machine loop, in-memory sources, typed output, permissions and private
payload slots. Run them from your external project:

```sh
cargo +1.98.1 test --test contract --locked
```

All four tests assert runtime outcomes, rather than just compiling API signatures:

1. **Namespace, source and output.** `text.open` and `client.open` each declare
   `read`. Requests reach the correct full namespace. A genuine acquired Buffer
   reads the Kernel's actual private `String`: one member returns its text, the
   other its length. `use config` requests source by stem, answered from memory
   exactly once. Ordinary lines `1` and `2` go to the driver's typed output vector;
   Core does not print them or choose a sink.
2. **Denied call.** The Buffer is correctly typed, but the Kernel removes its
   current read grant. The first read returns a located Host Fault before touching
   the payload or calling the later member. Restoring the grant permits a healthy
   read of the same retained entry.
3. **Failed run cleanup.** An earlier successful Buffer remains live. A later run
   acquires a new entry and then fails; the Host revokes and physically drops only
   that run's new payload, withdraws the latest accepted language line, and can
   still read the older entry. Previous Host effects are not rolled back.
4. **Aliases and disposal.** Functions reuse the acquired entry, and containers
   retain the same Resource Rc without another acquisition. Dropping language
   state does not release the Kernel-owned entry. A real retained Rust alias is checked directly by that
   Kernel after language disposal; this is not a fresh-Tables VM invocation.
   Explicit owner disposal releases the payload once, and a later policy check
   rejects the closed entry before payload access.

The Kernel issues a `Value::Resource` only after a real acquisition Call provides
its pending checked type witness. Its entry key and payload stay Host-owned; a
script cannot automatically fabricate one through structural construction or
byte decoding. A Host may explicitly supply a factory, `Read` or `Default`
operation that accepts data and returns an issued reference; its reply is still
validated. The witness proves language identity, not grants or liveness. Cloning
a reference transfers no ownership. See [resource identity](spec/03-types.md#resource-identity)
and [resource projection](api/09-embed.md#new-resources-inside-composite-replies) for
full generic and composite-result contracts.

A source request is not necessarily a filesystem operation. Answer
`Session::line`'s `Request::File` with the text for that stem, then continue the
Session until it yields a checked report. Compile only a successful report. Passing
the same file text directly as a session line does not give it a file module's
identity. Dispatch each Machine Call through the current joined tables and your
own policy, then continue until Done. The example's checked-source `expect` calls
are fixture assertions, not a production policy for arbitrary invalid user input.

The driver chooses when to collect `Machine::lines`, how to present values and
Faults, and whether to perform a requested effect. Core is not an implicit printer.
On success the Host retains entries; on Fault or abandonment it revokes only the
run's new entries, preserving earlier successful ones. The four-case fixture tests
Fault cleanup; the broader [embedding lifecycle contract](api/09-embed.md#abandon-unfinished-execution)
also defines abandonment. Explicit close/disposal logically revokes aliases even
if physical release fails. This small fixture does not implement a general lease,
cleanup-retry, async or cooperative-cell manager.

## Check the same Host without std

The runtime tests above execute the no_std Host library under a normal test
harness. If the bare target is already installed, also check that same library:

```sh
cargo +1.98.1 check --lib --target thumbv7em-none-eabihf --locked
```

This checks the entire Host implementation, not only a pending-witness helper.
It does not execute on hardware or supply an allocator for your target. Keep the
library's `core`/`alloc` boundary and put your application's platform work behind
its own driver. Consult the support policy for selected and qualified targets;
missing target components are prerequisites, not passing checks.

## When you need Standard or a REPL

For an application that wants the existing filesystem/environment/arguments host,
use the [Standard runner and policy](../tools/host/README.md), with matching package
versions. It supplies `load`, `Policy` and `Host::run`; a custom Kernel does not need
to adopt its modules or write policy. This walkthrough intentionally declares its
own `text.open` and `client.open`. Only `prelude` and `commands` are reserved.

For a REPL, [repl-core](../tools/repl-core/README.md) is an optional no-I/O facility.
Its `RequestId` and boundary `ResumeId` have different jobs: acknowledge actual
Start/End/Reset gates after your owner actions, and reject stale File/Call replies.
The pure
`source_continuation` query does not construct a Session or perform I/O; it reports
whether to submit for diagnostics, not that the text is valid.

## Qualification and limits

The pinned custom Host source matches the published Core dev.3 fixture. Its
four contract tests, same-library bare-target check, pure example and exact
consumer→Core graph passed from registry-only inputs without Standard. Separate
Core/REPL-core host and bare checks retained the independent three-node graph.
These results do not establish a newer checkout, Standard or final release.

Check your fresh project's resolution:

```sh
cargo +1.98.1 metadata --locked --format-version 1
```

It should contain only your project and registry `fipe 0.2.0-dev.3`, without a
path/Git override or Standard. Check the lock's registry checksum against the
published artifact, then run the commands above. A source-patched success does
not establish this registry association.

The [API reference](api/09-embed.md), [language specification](spec/README.md) and
[publishing procedure](publishing.md) describe the detailed contracts. This guide
adds no permission grant, payload schema or native resource transport, and does
not announce a final `0.2.x` release.
