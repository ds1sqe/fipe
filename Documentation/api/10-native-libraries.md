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

The proposed path is: a Fipe host declaration produces a call request; the
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
their exposure and release governed by the future host lifecycle contract.
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

**Rebase A: ownership, authority and run boundaries.** The provider layer must
consume the upstream host contract for opaque host-made resources, per-call
authorization, and completion, Fault and withdrawal boundaries. The host owns
and releases handles; the adapter must not invent public integer-to-resource
construction, a competing resource wire form or a separate lifecycle. Private
provider allocation tables remain implementation details. Today's
[`Value`](../../src/value.rs) has no dedicated resource variant, and the
current host chapter's resource description does not settle the requested
opaque wire representation or run-lifetime integration. Neither this note nor
the FFI scaffold changes that contract. Standard's `Rd` constructor is a
Standard declaration, not authority for every host to expose one.

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

## Open packaging and interface decisions

These questions remain proposals; this chapter selects no format or loader
behavior:

- **Native artifact location and discovery:** where separately built libraries
  live, how a host locates them, and how module identity maps to an artifact.
  A source-module stem or `use` is not an agreed native-library search path.
- **Compile-time flags and capabilities:** how a build records enabled native
  features and how declarations, artifacts and host capabilities are matched.
  Compatibility must account for the target and ABI as well as the functions
  a provider actually implements; matching types still does not grant runtime
  permission. No native-provider flag or negotiation scheme is defined here.
- **Offline interface declarations:** a proposed `.fipi` interface file, in
  the role of a Python `.pyi`, would let checking and tooling read a library's
  declarations without executing its native code. Its syntax, generation,
  association with an artifact, version/capability matching and integration
  with existing host declarations remain undecided. The current scaffold
  reads declaration text from a supplied descriptor; it has no `.fipi` loader.

These decisions and both upstream contracts must be resolved before provider
integration can claim the corresponding behavior. Existing Rust embedding
remains the reference path for a custom host.
