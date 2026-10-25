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


## Bounded replies

`codec::MAX_REPLY_BYTES` is 64 MiB of encoded Host reply payload, excluding its
eight-byte framing prefix. `encode_reply` accepts exactly that size and returns
`Error::TooLong(actual_bytes)` for larger replies before the caller can send a
header. This new public enum variant requires exhaustive callers to update their
matches; previously unbounded large replies must be reduced or chunked through
application operations. The check follows serialization, so it is not a general
encoder allocation quota. Slice-only decoding, runtime-to-Host frames and
in-process Standard behavior are unchanged.

An excessive incoming length is refused before runtime allocation/body reads.
Short, malformed or failed reply transports are terminal: runtime exit 2, no Done
frame, no transport reuse. Hosts close both ends and reap their child with a
supervisor deadline; they must not wait for Done after a failed write. Normal
encoded Host failures and typed mismatch still use ordinary Fault/Done exit 1.
This length/error policy does not add resource transport or change scalar ABI.

## Finite float payloads

Serialized `F32` and `F64` values must be finite, as required by the language.
The shared codec refuses NaN and either infinity as `Error::Malformed` when
encoding or decoding an actual float leaf, including nested data and frame
arguments, environment, lines, and results. Previously accepted nonfinite
wire bits now refuse; finite bit patterns, signed zero and subnormals retain
the same bytes. Runtime malformed replies use the existing poisoned-transport
exit2/no-Done behavior.

Rust producers can use `fipe::value::float::Float::F32(number).finite()` or
`fipe::value::float::Float::F64(number).finite()` to construct a checked `Value`;
failure is `Reason::NotFinite`. This does not
change the contract of raw Rust `Value` producers or add a scan of opaque
function captures, a valid-value fee, a new tag, or resource transport. FFI's
separate scalar conversion already rejects nonfinite `F64` values.
