# Experimental scalar providers

`fipe-ffi` adapts explicitly registered trusted native providers to Fipe's existing
host request/reply boundary. It is optional and does not depend on Standard.
ABI version 2 is an experimental, target-specific C-compatible layout; version 1's
unpublished structured layout is rejected. The repository proof verifies separately
compiled C and Rust providers through the VM on x86_64-unknown-linux-gnu with
Rust 1.98.1 and GCC 16.2.1. Native/LLVM execution, other targets, dynamic loading
and installed author packages remain separate conformance work.

Only `()`, prelude `Integer`, `F64` and `String` arguments/results are supported.
Integer transport is the signed i64 subset; larger Fipe integers fail before the
provider is called, with no truncation. Source I64 lowers to Integer and uses that
same transport; other erased refinements do not acquire new ABI guarantees.
F64 must be finite. Text is length-delimited UTF-8, including embedded NUL; a null
pointer is accepted only for an empty buffer. Opaque types are rejected from the
checked declaration even when their host-chosen runtime value is text or integer.
There are no structured values, declared Result/Err outcomes, resources or async
callbacks in this proof.

The driver registers the declarations, checks/compiles the source, authorizes each
request and its environment, invokes `Provider::call`, and supplies its result to
`Machine::reply`. Reply validation remains necessary. Registration/type checking
never grants authority. Status 0 carries one scalar; status 1 carries a TEXT host
Fault; unknown statuses, missing/duplicate emissions and invalid scalar metadata
are contract errors.

## Native safety contract

The descriptor, its code and metadata buffers remain immutable and valid for the
Provider borrow. All nonempty buffers must designate valid, initialized, aligned
storage within one live allocation, with a length fitting Rust's isize/PTRDIFF_MAX;
metadata checks cannot prove pointer provenance or allocation liveness. Initialize
every FipeValue field, including inactive ones (for C, start with
`FipeValue reply = {0};` then set tag/payload). Padding need not be initialized.
Backing storage for a pointer field is required only when its tag selects it;
inactive pointer fields may be null. Providers validate function labels, argument
counts and tags before accessing arguments.

Inputs, function-name bytes, context and emitter are borrowed only during Call;
retain none of them. Emit exactly once, synchronously on the invoking thread,
without overlap or reentrancy. Emitted Value/text storage remains valid until
emit returns; text is copied then. Never unwind Rust panics or C++ exceptions
across either callback boundary. Provider state must be stateless/safely reentrant
across independent wrappers, or every invocation sharing mutable state must be
externally serialized. Provider being neither Send nor Sync alone does not protect
state shared by multiple wrappers.

Native providers have process privileges. Fipe budgets cannot bound their time or
memory, and invalid native pointers or aborts cannot be recovered by validation.
The header is [include/fipe.h](include/fipe.h). Offline `.fipi` loading, runtime
artifact discovery/distribution and CPU/GPU build capability selection are separate
planned work. This crate does not load libraries or define a resource model or
scheduler.

## Repository C/Rust proof

This isolated fixture workspace was verified with Rust 1.98.1 and `cc` (GCC 16.2.1)
on x86_64-unknown-linux-gnu. It requires a C11 compiler accepting the listed GCC-style
flags and refuses cross compilation and unqualified
targets; a missing compiler fails explicitly. From the repository root:

```sh
proof_target="$PWD/tmp/ffi-provider-proof/target"
proof_build="$PWD/tmp/ffi-provider-proof/build"
RUSTC_WRAPPER= cargo +1.98.1 test --manifest-path tools/ffi/tests/providers/Cargo.toml --test c_rust --locked --offline --target-dir "$proof_target" --config "build.build-dir=\"$proof_build\""
```

The one serial test compares every ABI layout field/tag and exercises actual C
and separately compiled Rust callbacks through checked VM requests and replies.
It asserts i64 endpoints, signed zero, UTF-8/NUL and empty text, argument order,
copying C reply text before its local buffer is overwritten, located Host Faults,
wrong-result rejection, ABI mismatch, duplicate registration and driver denial.
Normal adapter builds do not compile this C fixture; root workspace tests do not
run the nested proof. These repository fixtures are excluded from the published
adapter package and do not prove a self-contained installed author example.
