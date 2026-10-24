# Native library boundary

This chapter records the boundary for separately authored C or Rust libraries
called from Fipe. It distinguishes existing embedding from proposed provider
integration. The [host specification](../spec/09-host.md) owns language rules;
the [embedding API](09-embed.md) owns the Rust request/reply protocol.

## Existing embedding

Rust embeds Fipe by supplying declarations, checking and compiling source, and
driving a `Machine` or `Session`. Fipe calls back into Rust by returning
`Request::Call`; the driver chooses an implementation and resumes execution
with `Reply::Value`. The crate itself invokes no host callback.
[`examples/custom_host.rs`](../../examples/custom_host.rs) demonstrates both
directions without Standard.

Core and Embed use `no_std` plus `alloc`. File-module lookup is also a returned
request, so a custom host can resolve a stem from memory or its own resource
system. Neither these layers nor the host contract requires Standard's OS
operations. [`fipe-host`](../../tools/host/README.md) supplies Standard's own
declarations, permissions and output policy. The
[native runtime host](../../tools/native/runtime/src/host.rs) currently answers
calls through a pipe or an in-process Standard host; that is separate from a
native library provider.

## Proposed provider layer and current scaffold

The explicit provider path is: a Fipe host declaration produces a call request; the
driver authorizes it and dispatches to a C or Rust provider; the driver returns
the result through the ordinary embedding reply boundary. Providers implement
declared modules without putting a C ABI into Core or requiring a custom host
to link Standard. The adapter converts supported values; it must not introduce
a second resource model or scheduler.

[`tools/ffi`](../../tools/ffi/) is an experimental scalar adapter. Its
[Rust adapter](../../tools/ffi/src/lib.rs),
[ABI layouts](../../tools/ffi/src/abi.rs) and
[C header](../../tools/ffi/include/fipe.h) describe a synchronous descriptor and
call mechanism. Experimental ABI version 2 carries only Unit, signed i64,
finite F64 and UTF-8 text; the earlier structured layout is rejected. The
adapter checks argument and result declarations against those prelude scalar
types before invocation, rejecting opaque types even when their runtime value
is text or integer. Fipe Integer values outside i64 range are rejected without
truncation. The adapter registers declaration text and copies emitted scalar
replies across the boundary. It requires the driver to
authorize calls and pass results through the embedding validator. Its provider
wrapper is neither `Send` nor `Sync`.

Rust callback fixtures in [tests/scalars.rs](../../tools/ffi/tests/scalars.rs)
exercise scalar type rejection, malformed replies and driver denial. The
[repository C/Rust proof](../../tools/ffi/README.md#repository-crust-proof)
verified independently compiled providers, C/Rust layout agreement and actual
VM request/reply parity on x86_64-unknown-linux-gnu with Rust 1.98.1 and GCC 16.2.1.
Native/LLVM provider execution, resources, GPU support, other platforms, dynamic
loading and installed author examples remain unverified. Version and
descriptor-size checks do not provide a stable ABI or compatibility promise.

## Trust and allocation

An in-process native provider is trusted code: declaration checking and reply
validation do not sandbox it or establish pointer validity. The driver must
decide permission on every call, including the call's environment. A checked
type describes a value's shape; it grants no authority. A refused call can
return `Reply::Value(Err(message))`, which Faults the run.

Library-specific CPU and device allocations must remain provider-owned, with
their exposure and release governed by the selected Host lifecycle policy and
separately qualified resource implementation.
The scaffold borrows input storage for the call and copies the provider's
reply during its emission callback. The provider must keep that reply storage
valid until the callback returns, must not retain borrowed inputs or the
callback, and must not unwind across the ABI. The experimental synchronous
interface requires invocation and reply emission on the invoking thread,
without overlapping calls or reentry into the provider. The Rust wrapper's
`Send`/`Sync` restriction cannot enforce foreign callback behavior. Provider
code and descriptor storage must remain alive while registered. This does not
define a general allocator-transfer or resource-lifetime API.

## Two separate upstream integrations

**Rebase A: ownership, authority and run boundaries.** Host resource ownership and
run/session policy are selected separately from this scalar adapter. The Host owns
payloads, grants and explicit revocation; copied typed references do not transfer
ownership. Successful-run entries remain session-owned; Fault/abandonment cleanup
revokes only newly created entries. Resource carrier implementation/qualification
and resource-native/provider transport are separate gates. This scalar ABI refuses
resource declarations/values; it does not define a competing wire form or permit
integer-to-resource construction. Neither the linked scalar proof nor offline
interface loading settles broader native-resource release support.

**Rebase B: async and parallel execution.** A separate upstream contract must
define request identity, multiple outstanding calls, reply matching,
completion, threading and cancellation. Today's
[`Request`, `Reply` and `Outcome`](../../src/embed.rs) carry no request IDs;
the scaffold invokes providers synchronously. Async completion, cancellation
and provider unloading must follow that future contract rather than a private
adapter scheduler. The two rebases have separate ownership and must be
integrated and verified separately.

GPU remains required tensor-library scope. Device allocation, transfers,
completion and release must respect both future ownership and concurrency
contracts, including cancellation or a Fault while device work is outstanding.
The current value-copy scaffold does not implement those guarantees.

## Offline interfaces and explicit linked delivery

Editors can select absolute `.fipi` declaration paths and reload their text
atomically through the [LSP interface contract](../../tools/lsp/README.md#offline-interface-inputs).
Invalid updates retain last-good declarations with diagnostics and source-safe
navigation. These paths never load or invoke a provider. An editor's latest text
is not automatically associated with a runtime artifact.

The [linked author example](../../tools/ffi/examples/linked-provider/README.md)
uses a C object or separate Rust provider crate explicitly linked into one custom
VM host. The host reads one saved `.fipi` snapshot and compares its expected module
and exact bytes with the descriptor using existing getters before fresh registration
and checking. This scalar association is driver policy, not a new implicit check in
all `Provider::call` users. Exact bytes include comments/line endings; they do not
prove native code trust, resource identity or a live schema contract. Per-call
permissions and reply validation remain required.

The example ships manifest templates for materialization outside the adapter
package and uses the matching shipped header. Its Cargo-generated lock, serial
C/Rust test, both runs and actual package asset contents passed source qualification
on Rust 1.98.1/x86_64 GNU Linux. Fresh external archive dependency execution remains
a separate gate. Current
exact `0.2.0-dev.2` dependencies use explicit task-only source/archive patches for
prepublication checks, without assuming registry availability. The linked image
and immutable static metadata outlive each descriptor/Provider borrow; no unloading
operation is supplied.

Dynamic discovery/loading, artifact search policy, other provider targets,
CPU/GPU capabilities, resource-native transport and async execution still need
explicit owner decisions and separate implementation/qualification. The selected
linked scalar author route is the minimum proof, not removal of those obligations.
Source `use`, valid types, artifact hashes and editor completion grant no execution
authority. Current repository C/Rust qualification remains x86_64 GNU Linux VM;
no new installed/native/LLVM/resource support claim follows from this chapter.
