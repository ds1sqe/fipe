//! The runtime the code of Fipe's native backend links against, as
//! `libfipe_native_rt.a`. Machine code reaches it only through the C
//! exports of `abi`, the one module that touches raw pointers.
#![deny(unsafe_code)]
// The process entry is left out of the test harness, which has its own.
#![cfg_attr(test, allow(dead_code))]

mod abi;
pub mod codec;
mod host;
mod ops;
mod start;
