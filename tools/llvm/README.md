# fipe-llvm

Fipe's LLVM backend: a checked program's MIR as textual LLVM IR,
compiled to an object by `clang` and linked against the native
backend's runtime ([`tools/native`](../native/README.md)). It is the
optional second backend the [documentation](../../Documentation/README.md)
names: a separate package, depending on `fipe` and `fipe-native-core`, with
no unsafe code.

```sh
fipe build FILE -o EXE --runtime libfipe_native_rt.a --llvm
```

## Shape

The IR has the x86-64 backend's shape, so both observe as the VM does
under the same runtime:

- one function per MIR body, `i32 @body_N(ptr args, ptr caps, ptr
  hidden, ptr out)`, whose frame is an array of opaque 16-byte slots,
  locals then scratch; every operation on a slot is a call to the
  runtime's C exports (`rt_assign`, `rt_call`, `rt_charge`, …), a status
  1 going to the epilogue, which drops the slots;
- a call to a body without witnesses is a direct call; a `Switch` and a
  gate's `Test` are LLVM `switch`es on the runtime's answer;
- `@fipe_main` runs each module's init body once, then the main;
  `@fipe_bodies` holds each body's address, `@fipe_unit` the unit's
  length and bytes, which the runtime compiles again at startup.

`module(program, unit)` gives the IR; `compile(ir, object)` writes it
beside the object as `.ll`, runs `clang -c -O2 -fPIC -x ir` and removes
it. The compiler is `FIPE_CLANG`, else `clang-23`.

## Stack

LLVM keeps a body's arguments in callee-saved registers, so its frame
is larger than the x86-64 backend's (264 bytes against 232 for a body
of twelve slots): a recursion a million calls deep overflows the
default 256 MiB stack that the x86-64 object just fits in. Overflow is
a resource limit outside the observation
([native](../native/README.md) §Running); raise `FIPE_STACK` or bound
the run with `FIPE_BUDGET`.

## Checks

```sh
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
(cd ../native && FIPE_BACKEND=llvm cargo test --locked --offline \
  --test equivalence)
```

The last runs the native backend's whole equivalence suite on LLVM
objects: the first milestone and its budget cuts, each example against
its trace fixture, every drift fixture, gates and a recursion without
end, each equal to the VM in value or Fault, host calls, output and
units charged.

See the [support policy](../../Documentation/development.md#compatibility-and-support)
for the Rust 1.98.1 floor and platform qualification status.
Executable support is x86_64-unknown-linux-gnu only. Match compiler, generated
artifacts and runtime to the exact Fipe version; this policy does not imply a
mechanical mismatched-version rejection guard.


Checked programs with resource declarations are refused by native execution,
including private or unused declarations. Native `lower`, `object`, `listing`
and LLVM `module` return `Result<_, String>`; callers must propagate refusal.
`Unit::new` and `Unit::compile` also refuse. Raw `Unit` source metadata still
encodes/decodes unchanged; transporting sources does not certify executable
support. Startup refuses reconstructed marked sources before opening the Host,
reading input or calling generated code. Raw assembler/ELF and LLVM text APIs
remain low-level interfaces without a checked-program support guarantee.
