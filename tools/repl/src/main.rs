//! The Fipe REPL: one pump over `fipe-repl-core`, with I/O from the
//! platform chosen by feature.
//!
//! `fipe-repl [--cwd <dir>] [--host <dir>] [--strict] [--budget <n>]
//! [--compile-budget <n>]`: the
//! working directory changes first, so a relative host directory is found
//! from it; with `--host` the session starts as after `:host <dir>`.
//! `--strict` rejects open implicit checks; explicit gates remain allowed.
//! `--budget` sets the VM's charge units each line runs under, the REPL's
//! own default otherwise; `--compile-budget` the checker's steps each
//! compile-time evaluation may spend, the crate's default otherwise.

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
use fipe_repl_core::{Out, Repl, BUDGET, COMPILE_BUDGET};

const USAGE: &str = "usage: fipe-repl [--cwd <dir>] [--host <dir>] \
    [--strict] [--budget <n>] [--compile-budget <n>]\n";

/// What the command line asks for.
struct Options {
    host: Option<String>,
    strict: bool,
    budget: u64,
    compile_budget: u64,
}

#[cfg(feature = "std")]
fn main() {
    run();
}

/// Read, feed, act on every output; stop at `Out::Exit` or end of input.
/// File paths are relative to the working directory.
fn run() {
    let Options {
        host,
        strict,
        budget,
        compile_budget,
    } = match arguments() {
        Ok(options) => options,
        Err(message) => {
            platform::write(message.as_bytes());
            platform::exit(2);
        }
    };
    let mut repl = Repl::new();
    repl.strict(strict);
    repl.budget(budget);
    repl.compile_budget(compile_budget);
    let mut outs = VecDeque::from(match host {
        Some(dir) => repl.host_directory(&dir),
        None => repl.feed(&[]),
    });
    let mut buffer = [0; 4096];
    let mut end = false;
    loop {
        while let Some(out) = outs.pop_front() {
            match out {
                Out::Write(bytes) => platform::write(&bytes),
                Out::File(path) => {
                    let text = platform::read_file(&path);
                    if text.is_none() {
                        platform::write(
                            format!("{path}: cannot read file\n").as_bytes(),
                        );
                    }
                    outs.extend(repl.file(&path, text.as_deref()));
                }
                Out::Exit => return,
            }
        }
        if end {
            return;
        }
        let read = match platform::read(&mut buffer) {
            Ok(read) => read,
            Err(()) => {
                platform::write(b"cannot read standard input\n");
                platform::exit(2);
            }
        };
        end = read == 0;
        outs.extend(if end {
            repl.flush()
        } else {
            repl.feed(&buffer[..read])
        });
    }
}

/// The options, after acting on `--cwd`.
fn arguments() -> Result<Options, String> {
    let mut args = platform::args().into_iter();
    let mut host = None;
    let mut strict = false;
    let mut budget = BUDGET;
    let mut compile_budget = COMPILE_BUDGET;
    while let Some(arg) = args.next() {
        if arg == "--strict" {
            strict = true;
            continue;
        }
        match (arg.as_str(), args.next()) {
            ("--host", Some(dir)) => host = Some(dir),
            ("--budget", Some(units)) => match units.parse() {
                Ok(units) => budget = units,
                Err(_) => return Err(USAGE.into()),
            },
            ("--compile-budget", Some(units)) => match units.parse() {
                Ok(units) => compile_budget = units,
                Err(_) => return Err(USAGE.into()),
            },
            ("--cwd", Some(dir)) if platform::chdir(&dir) => {}
            ("--cwd", Some(dir)) => {
                return Err(format!("{dir}: no such directory\n"))
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Options {
        host,
        strict,
        budget,
        compile_budget,
    })
}
