//! The `fipe-repl` command: [`fipe_repl::program`] on the platform the
//! feature chose; the `linux` platform enters at its own `_start`.
#![cfg_attr(feature = "linux", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
fn main() {
    fipe_repl::program();
}

#[cfg(feature = "linux")]
extern crate fipe_repl;
