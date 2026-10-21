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

use alloc::{collections::VecDeque, format, string::String, vec::Vec};
use core::fmt::Write as _;
use fipe_repl_core::{
    edit::{Editor, Event, Keys, View},
    highlight::Class,
    Out, Repl, BUDGET, COMPILE_BUDGET,
};

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
/// At a terminal, input goes through the line editor in raw mode.
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
    let outs = VecDeque::from(match host {
        Some(dir) => repl.host_directory(&dir),
        None => repl.feed(&[]),
    });
    if platform::terminal() && platform::raw() {
        edited(&mut repl, outs);
        platform::cooked();
    } else {
        piped(&mut repl, outs);
    }
}

/// Act on every output in order; false at `Out::Exit`.
fn act(repl: &mut Repl, outs: &mut VecDeque<Out>) -> bool {
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
            Out::Exit => return false,
        }
    }
    true
}

/// Input as it comes, from a pipe, a file or a terminal left cooked.
fn piped(repl: &mut Repl, mut outs: VecDeque<Out>) {
    let mut buffer = [0; 4096];
    // Input read but not yet fed: fed one line at a time, so each line's
    // output is written before the next line runs.
    let mut unfed: Vec<u8> = Vec::new();
    let mut end = false;
    loop {
        if !act(repl, &mut outs) {
            return;
        }
        if !unfed.is_empty() {
            let line = unfed
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(unfed.len(), |at| at + 1);
            let line: Vec<u8> = unfed.drain(..line).collect();
            outs.extend(repl.feed(&line));
            continue;
        }
        if end {
            return;
        }
        let read = read(&mut buffer);
        end = read == 0;
        if end {
            outs.extend(repl.flush());
        } else {
            unfed.extend_from_slice(&buffer[..read]);
        }
    }
}

/// Keys from a terminal in raw mode, through the line editor; each
/// line entered is fed with its LF. The line is coloured by token class
/// unless `NO_COLOR` is set.
fn edited(repl: &mut Repl, mut outs: VecDeque<Out>) {
    let color = !platform::no_color();
    let draw = |view: &View| draw(view, color);
    let mut editor = Editor::new();
    let mut keys = Keys::new();
    let mut buffer = [0; 256];
    loop {
        if !act(repl, &mut outs) {
            return;
        }
        editor.prompt(repl.prompt());
        let read = read(&mut buffer);
        if read == 0 {
            outs.extend(repl.flush());
            act(repl, &mut outs);
            return;
        }
        for key in keys.push(&buffer[..read]) {
            let mut event = editor.key(key);
            if let Event::Complete(before) = event {
                event = editor.complete(repl.complete(&before));
            }
            match event {
                Event::None | Event::Complete(_) => continue,
                Event::Draw => {
                    draw(&editor.view());
                    continue;
                }
                Event::List(candidates) => {
                    let list = candidates.join("  ");
                    platform::write(format!("\n{list}\n").as_bytes());
                    draw(&editor.view());
                    continue;
                }
                Event::Line(mut line) => {
                    let prompt = repl.prompt();
                    let cursor = prompt.len() + line.chars().count();
                    draw(&View::new(prompt, line.clone(), cursor));
                    platform::write(b"\n");
                    line.push('\n');
                    outs.extend(repl.feed(line.as_bytes()));
                }
                Event::Interrupt => {
                    platform::write(b"^C\n");
                    outs.extend(repl.interrupt());
                }
                Event::End => {
                    outs.extend(repl.flush());
                    act(repl, &mut outs);
                    return;
                }
            }
            if !act(repl, &mut outs) {
                return;
            }
            editor.prompt(repl.prompt());
        }
    }
}

/// Redraw the line in place: to its start, the prompt and text, clear
/// the rest, then the cursor to its column, which counts text only.
fn draw(view: &View, color: bool) {
    // ponytail: a line wider than the terminal wraps and the redraw
    // misses the rows above; track rows when long lines matter.
    let mut bytes = format!("\r{}", view.prompt);
    if color {
        let chars: Vec<char> = view.text.chars().collect();
        let mut at = 0;
        for (range, class) in &view.classes {
            bytes.extend(&chars[at..range.start]);
            let _ = write!(bytes, "\x1b[{}m", sgr(*class));
            bytes.extend(&chars[range.clone()]);
            bytes.push_str("\x1b[0m");
            at = range.end;
        }
        bytes.extend(&chars[at..]);
    } else {
        bytes.push_str(&view.text);
    }
    bytes.push_str("\x1b[K\r");
    if view.cursor > 0 {
        let _ = write!(bytes, "\x1b[{}C", view.cursor);
    }
    platform::write(bytes.as_bytes());
}

/// The SGR foreground of a token class.
const fn sgr(class: Class) -> u8 {
    match class {
        Class::Keyword => 35,
        Class::String => 32,
        Class::Number => 33,
        Class::Comment => 90,
        Class::Constructor => 36,
        Class::Operator => 34,
    }
}

/// Bytes from stdin; 0 at end of input. A read error ends the REPL.
fn read(buffer: &mut [u8]) -> usize {
    platform::read(buffer).unwrap_or_else(|()| {
        platform::write(b"cannot read standard input\n");
        platform::exit(2);
    })
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
