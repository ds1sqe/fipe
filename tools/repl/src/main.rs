//! The Fipe REPL: one pump over `fipe-repl-core`, with I/O from the
//! platform chosen by feature.
//!
//! `fipe-repl [--cwd <dir>] [--host <dir>] [--strict]`: the working directory
//! changes first, so a relative host directory is found from it; with
//! `--host` the session starts as after `:host <dir>`. `--strict` rejects
//! open implicit checks; explicit gates remain allowed.

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

use alloc::{collections::VecDeque, format, string::String};
use fipe_repl_core::{Out, Repl};

const USAGE: &str =
    "usage: fipe-repl [--cwd <dir>] [--host <dir>] [--strict]\n";

#[cfg(feature = "std")]
fn main() {
    run();
}

/// Read, feed, act on every output; stop at `Out::Exit` or end of input.
/// File paths are relative to the working directory.
fn run() {
    let (host, strict) = match arguments() {
        Ok(options) => options,
        Err(message) => {
            platform::write(message.as_bytes());
            platform::exit(2);
        }
    };
    let mut repl = Repl::new();
    repl.strict(strict);
    let first = host.map_or_else(String::new, |dir| format!(":host {dir}\n"));
    let mut outs = VecDeque::from(repl.feed(first.as_bytes()));
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

/// The host directory and strict mode, after acting on `--cwd`.
fn arguments() -> Result<(Option<String>, bool), String> {
    let mut args = platform::args().into_iter();
    let mut host = None;
    let mut strict = false;
    while let Some(arg) = args.next() {
        if arg == "--strict" {
            strict = true;
            continue;
        }
        match (arg.as_str(), args.next()) {
            ("--host", Some(dir)) => host = Some(dir),
            ("--cwd", Some(dir)) if platform::chdir(&dir) => {}
            ("--cwd", Some(dir)) => {
                return Err(format!("{dir}: no such directory\n"))
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok((host, strict))
}
