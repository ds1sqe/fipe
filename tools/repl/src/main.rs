//! The Fipe REPL: one pump over `fipe-repl-core`, with I/O from the
//! platform chosen by feature.

#![cfg_attr(feature = "linux", no_std, no_main)]
#![deny(unsafe_code)]

#[cfg(all(feature = "std", feature = "linux"))]
compile_error!("enable exactly one platform feature: `std` or `linux`");
#[cfg(not(any(feature = "std", feature = "linux")))]
compile_error!("enable one platform feature: `std` or `linux`");

extern crate alloc;

#[cfg(feature = "std")]
#[path = "platform/std.rs"]
mod platform;

#[cfg(feature = "linux")]
#[path = "platform/linux.rs"]
mod platform;

use alloc::collections::VecDeque;
use fipe_repl_core::{Out, Repl};

#[cfg(feature = "std")]
fn main() {
    run();
}

/// Read, feed, act on every output; stop at `Out::Exit` or end of input.
/// File paths are relative to the working directory.
fn run() {
    let mut repl = Repl::new();
    let mut outs = VecDeque::from(repl.feed(&[]));
    let mut buffer = [0; 4096];
    let mut end = false;
    loop {
        while let Some(out) = outs.pop_front() {
            match out {
                Out::Write(bytes) => platform::write(&bytes),
                Out::File(path) => {
                    let text = platform::read_file(&path);
                    outs.extend(repl.file(&path, text.as_deref()));
                }
                Out::Exit => return,
            }
        }
        if end {
            return;
        }
        let read = platform::read(&mut buffer);
        end = read == 0;
        outs.extend(if end {
            repl.flush()
        } else {
            repl.feed(&buffer[..read])
        });
    }
}
