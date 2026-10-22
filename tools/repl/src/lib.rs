//! The Fipe REPL: one pump over `fipe-repl-core`, with I/O from the
//! platform chosen by feature. [`program`] is the `fipe-repl` command;
//! [`run`] is the pump under given [`Options`], answering host calls
//! through a [`Host`] when one is given.
//!
//! `fipe-repl [--cwd <dir>] [--host <dir>] [--strict] [--budget <n>]
//! [--compile-budget <n>]`: the
//! working directory changes first, so a relative host directory is found
//! from it; with `--host` the session starts as after `:host <dir>`.
//! `--strict` rejects open implicit checks; explicit gates remain allowed.
//! `--budget` sets the VM's charge units each line runs under, the REPL's
//! own default otherwise; `--compile-budget` the checker's steps each
//! compile-time evaluation may spend, the crate's default otherwise.

#![cfg_attr(feature = "linux", no_std)]
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
use fipe::{
    embed::{Reply, Request},
    ir::Tables,
};
use fipe_repl_core::{
    edit::{Editor, Event, Keys, View},
    highlight::Class,
    Out, Repl, BUDGET, COMPILE_BUDGET,
};

const USAGE: &str = "usage: fipe-repl [--cwd <dir>] [--host <dir>] \
    [--strict] [--budget <n>] [--compile-budget <n>]\n";

/// What the command line asks for.
pub struct Options {
    /// A directory of host declarations to start under, as `:host`.
    pub host: Option<String>,
    pub strict: bool,
    pub budget: u64,
    pub compile_budget: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            host: None,
            strict: false,
            budget: BUDGET,
            compile_budget: COMPILE_BUDGET,
        }
    }
}

/// A host answering the session's calls: its declaration sources by
/// namespace, `commands` first; `name`, where a Fault in one is
/// located, as `:host` names a directory; and the answer to each call.
pub struct Host<'a> {
    pub name: &'a str,
    pub sources: &'a [(&'a str, &'a str)],
    pub answer: &'a mut dyn FnMut(&Request, &Tables) -> Reply,
}

/// The `fipe-repl` command: the options from the command line, then
/// [`run`] with no host.
pub fn program() {
    let options = match arguments() {
        Ok(options) => options,
        Err(message) => {
            platform::write(message.as_bytes());
            platform::exit(2);
        }
    };
    run(&options, None);
}

/// Read, feed, act on every output; stop at `Out::Exit` or end of input.
///
/// At a terminal, input goes through the line editor in raw mode.
/// File paths are relative to the working directory. With a host the
/// session starts over its declarations and each call is answered by
/// it; otherwise over `options.host`'s directory, when there is one.
pub fn run(options: &Options, mut host: Option<Host<'_>>) {
    let mut repl = Repl::new();
    repl.strict(options.strict);
    repl.budget(options.budget);
    repl.compile_budget(options.compile_budget);
    let outs = VecDeque::from(match (&host, &options.host) {
        (Some(host), _) => {
            repl.hosted(true);
            repl.host_sources(host.name, host.sources)
        }
        (None, Some(dir)) => repl.host_directory(dir),
        (None, None) => repl.feed(&[]),
    });
    if platform::terminal() && platform::raw() {
        // Only at a terminal, so piped transcripts keep their bytes.
        platform::write(b"Fipe REPL: :help lists the commands, :q quits.\n");
        edited(&mut repl, outs, &mut host);
        platform::cooked();
    } else {
        piped(&mut repl, outs, &mut host);
    }
}

/// Act on every output in order; false at `Out::Exit`. With `last`,
/// the last byte written before a prompt, a prompt starts its own line,
/// since the editor's redraw returns to the line's start.
fn act(
    repl: &mut Repl,
    outs: &mut VecDeque<Out>,
    mut last: Option<&mut u8>,
    host: &mut Option<Host<'_>>,
) -> bool {
    while let Some(out) = outs.pop_front() {
        match out {
            Out::Write(bytes) => match last.as_deref_mut() {
                Some(last) => write_edited(&bytes, repl.prompt(), last),
                None => platform::write(&bytes),
            },
            Out::File(path) => {
                let text = platform::read_file(&path);
                if text.is_none() {
                    let message = format!("{path}: cannot read file\n");
                    match last.as_deref_mut() {
                        Some(last) => {
                            write_edited(message.as_bytes(), "", last);
                        }
                        None => platform::write(message.as_bytes()),
                    }
                }
                outs.extend(repl.file(&path, text.as_deref()));
            }
            Out::Call => {
                let Some(host) = host else {
                    unreachable!("a session not hosted asks for no call")
                };
                // ponytail: the host's own output bypasses `last`, so a
                // prompt after a `put out` without LF shares its line.
                outs.extend(
                    repl.answer(|request, tables| {
                        (host.answer)(request, tables)
                    }),
                );
            }
            Out::Exit => return false,
        }
    }
    true
}

/// Write `bytes`, with an LF before a trailing `prompt` when the output
/// before it, in these bytes or earlier, does not end one.
fn write_edited(bytes: &[u8], prompt: &str, last: &mut u8) {
    let prompt = prompt.as_bytes();
    let Some(body) = bytes.strip_suffix(prompt).filter(|_| !prompt.is_empty())
    else {
        platform::write(bytes);
        *last = bytes.last().copied().unwrap_or(*last);
        return;
    };
    platform::write(body);
    if body.last().copied().unwrap_or(*last) != b'\n' {
        platform::write(b"\n");
    }
    platform::write(prompt);
    // The line typed after the prompt ends with the LF Enter writes.
    *last = b'\n';
}

/// Input as it comes, from a pipe, a file or a terminal left cooked.
fn piped(
    repl: &mut Repl,
    mut outs: VecDeque<Out>,
    host: &mut Option<Host<'_>>,
) {
    let mut buffer = [0; 4096];
    // Input read but not yet fed: fed one line at a time, so each line's
    // output is written before the next line runs.
    let mut unfed: Vec<u8> = Vec::new();
    let mut end = false;
    loop {
        if !act(repl, &mut outs, None, host) {
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
fn edited(
    repl: &mut Repl,
    mut outs: VecDeque<Out>,
    host: &mut Option<Host<'_>>,
) {
    let color = !platform::no_color();
    let draw = |view: &View| draw(view, color);
    let mut editor = Editor::new();
    let mut keys = Keys::new();
    let mut buffer = [0; 256];
    // The banner ends its line.
    let mut ended = b'\n';
    loop {
        if !act(repl, &mut outs, Some(&mut ended), host) {
            return;
        }
        editor.prompt(repl.prompt());
        let read = read(&mut buffer);
        if read == 0 {
            outs.extend(repl.flush());
            act(repl, &mut outs, Some(&mut ended), host);
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
                    act(repl, &mut outs, Some(&mut ended), host);
                    return;
                }
            }
            if !act(repl, &mut outs, Some(&mut ended), host) {
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
