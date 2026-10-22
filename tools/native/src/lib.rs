//! Fipe's native backend: x86-64 machine code for Linux and the System V ABI, written
//! into an ELF64 relocatable object and linked with `cc` against the
//! runtime `libfipe_native_rt.a`.
#![forbid(unsafe_code)]

pub mod link;
