# fipe-native-core

Shared byte formats and operand layout for Fipe's native backend, LLVM
backend and native runtime. This crate keeps their encoding and decoding
in one implementation; it depends only on `fipe` and Rust's standard library.

Most embedders should start with `fipe`. Use `fipe-native` for code generation
and `fipe-native-rt` for the runtime archive. `fipe_native::codec` continues
to re-export this crate's `codec` module.

Keep all Fipe crates on the same prerelease version: the byte formats and
Rust APIs are experimental.
