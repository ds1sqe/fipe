# fipe-native-rt

The runtime linked by Fipe's native and LLVM backends on x86-64 Linux.
It supplies the `rt_*` operations and uses `fipe-host` for filesystem,
arguments, environment and output.

From the Fipe workspace:

```sh
cargo build -p fipe-native-rt --release --locked
fipe build hello.fipe -o hello --runtime target/release/libfipe_native_rt.a
```

This is a library package, not a command installed by `cargo install`.
To build the archive in another Cargo project, depend on the exact same
version of `fipe-native-rt` as the compiler, and run:

```sh
cargo rustc -p fipe-native-rt --release --lib --crate-type staticlib -- -C panic=abort
```

Cargo places the archive at `target/release/libfipe_native_rt.a` by default;
pass its path with `--runtime`. Set `panic = "abort"` in the consuming
project's development and release profiles when building native executables.
The workspace already sets this policy. Native formats must match the compiler
version. Rust APIs are experimental during the prerelease; final `0.2.x` patch
compatibility follows the support policy below.

See the [support policy](../../../Documentation/development.md#compatibility-and-support)
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

The runtime does not support genuine `Value::Resource` values. Input/reply
validation refuses them even inside containers or function captures and filled
arguments; resources have no native dispatch tag. The refusal scan borrows values,
walks iteratively and expands each shared allocation once. This is not a global
budget, arbitrary-depth execution or value-destruction guarantee. The internal `Value` layout has compile-time size-16/alignment-8 assertions.
These are internal Rust layout checks, not a resource wire ABI, and do not expand
native resource support.


Under the Standard Host, earlier main output is flushed before the next request
has an effect, and request output is flushed before generated code continues.
A write/flush error stops execution with a Host Fault; earlier effects remain.
Final output failure cannot return success (exit1); existing execution Faults
remain failures and pre-run input refusal keeps exit2. Standard poisons the whole
writer pair after its first error, discards unwritten/unattempted tails and never
replays an accepted prefix. No byte delivery/recovery or effect rollback is
promised. The host pipe protocol and resource refusal policy are unchanged.
