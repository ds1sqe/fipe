# fipe-native-core

Shared byte formats and operand layout for Fipe's native backend, LLVM
backend and native runtime. This crate keeps their encoding and decoding
in one implementation; it depends only on `fipe` and Rust's standard library.

Most embedders should start with `fipe`. Use `fipe-native` for code generation
and `fipe-native-rt` for the runtime archive. `fipe_native::codec` continues
to re-export this crate's `codec` module.

Keep all Fipe crates on the same prerelease version: the byte formats and
Rust APIs are experimental.


Checked programs with resource declarations are refused by native execution,
including private or unused declarations. Native `lower`, `object`, `listing`
and LLVM `module` return `Result<_, String>`; callers must propagate refusal.
`Unit::new` and `Unit::compile` also refuse. Raw `Unit` source metadata still
encodes/decodes unchanged; transporting sources does not certify executable
support. Startup refuses reconstructed marked sources before opening the Host,
reading input or calling generated code. Raw assembler/ELF and LLVM text APIs
remain low-level interfaces without a checked-program support guarantee.

Genuine `Value::Resource` values are also refused by `codec::Frame::encode` and
`codec::encode_reply` with `Error::Opaque`. This includes resources inside
containers, function captures or filled arguments, and shown values that would
otherwise become function placeholders. The preflight borrows values, walks
iteratively and expands each shared allocation once; it does not add resource
transport or promise a global work budget, arbitrary-depth encoding or stack-safe
destruction of every input value.
