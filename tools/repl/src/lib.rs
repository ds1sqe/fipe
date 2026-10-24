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
///
/// This adapter acknowledges boundaries immediately, without owner cleanup hooks.
/// Resource-owning hosts must use a lifecycle-aware `fipe-repl-core` driver.
pub struct Host<'a> {
    pub name: &'a str,
    pub sources: &'a [(&'a str, &'a str)],
    /// `Ok(Reply::Value(Err(_)))` is a recoverable script fault.
    /// `Err` is a fatal driver failure: dispose, restore cooked mode and exit code 2.
    pub answer: &'a mut dyn FnMut(&Request, &Tables) -> Result<Reply, String>,
}

/// The `fipe-repl` command: the options from the command line, then
/// [`run`] with no host.
pub fn program() {
    let options = match arguments() {
        Ok(options) => options,
        Err(message) => {
            let _ = platform::write(message.as_bytes());
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
///
/// # Panics
/// If internal request ordering or initial-session state is inconsistent.
pub fn run(options: &Options, host: Option<Host<'_>>) {
    if run_driver(options, host, &mut Output::new(platform::write)).is_err() {
        platform::exit(2);
    }
}

struct Output {
    writer: fn(&[u8]) -> Result<(), ()>,
    failed: bool,
}
impl Output {
    const fn new(writer: fn(&[u8]) -> Result<(), ()>) -> Self {
        Self {
            writer,
            failed: false,
        }
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if self.failed {
            return Err(());
        }
        let result = (self.writer)(bytes);
        self.failed = result.is_err();
        result
    }
}

fn run_driver(
    options: &Options,
    mut host: Option<Host<'_>>,
    output: &mut Output,
) -> Result<(), ()> {
    let mut repl = Repl::new();
    repl.strict(options.strict);
    repl.budget(options.budget);
    repl.compile_budget(options.compile_budget);
    let mut outs = VecDeque::from(match (&host, &options.host) {
        (Some(host), _) => {
            repl.hosted(true);
            repl.host_sources(host.name, host.sources)
                .expect("a new REPL has no pending request")
        }
        (None, Some(dir)) => repl
            .host_directory(dir)
            .expect("a new REPL has no pending request"),
        (None, None) => repl.feed(&[]),
    });
    let result = if platform::terminal() && platform::raw() {
        output
            .write(b"Fipe REPL: :help lists the commands, :q quits.\n")
            .and_then(|()| edited(&mut repl, &mut outs, &mut host, output))
    } else {
        piped(&mut repl, &mut outs, &mut host, output)
    };
    if result.is_err() {
        dispose(&mut repl, &mut outs);
    }
    platform::cooked();
    result
}

/// Close before any new Start; retain already computed End/Reset boundaries.
fn dispose(repl: &mut Repl, outs: &mut VecDeque<Out>) {
    loop {
        if outs.iter().any(|out| matches!(out, Out::Exit)) {
            return;
        }
        match repl.close() {
            Ok(closed) => {
                *outs = VecDeque::from(closed);
                if outs.is_empty() {
                    return;
                }
            }
            Err(fipe_repl_core::DriveError::Busy) => {}
            Err(error) => panic!("cannot close REPL: {error:?}"),
        }
        let boundary = outs
            .iter()
            .position(|out| matches!(out, Out::End { .. } | Out::Reset { .. }));
        if let Some(at) = boundary {
            let out = outs.remove(at).expect("located boundary");
            #[cfg(all(test, feature = "std"))]
            output_tests::record(&out);
            let (Out::End { resume, .. } | Out::Reset { resume, .. }) = out
            else {
                unreachable!()
            };
            *outs = VecDeque::from(
                repl.resume(&resume).expect("current terminal boundary"),
            );
        } else if outs.iter().any(|out| matches!(out, Out::Exit)) {
            return;
        } else {
            panic!("missing current disposal boundary");
        }
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
    output: &mut Output,
) -> Result<bool, ()> {
    while let Some(out) = outs.pop_front() {
        match out {
            Out::Write(bytes) => match last.as_deref_mut() {
                Some(last) => {
                    write_edited(output, &bytes, repl.prompt(), last)?;
                }
                None => output.write(&bytes)?,
            },
            Out::File { id, path } => {
                let text = platform::read_file(&path);
                if text.is_none() {
                    let message = format!("{path}: cannot read file\n");
                    match last.as_deref_mut() {
                        Some(last) => {
                            write_edited(output, message.as_bytes(), "", last)?;
                        }
                        None => output.write(message.as_bytes())?,
                    }
                }
                outs.extend(
                    repl.file(&id, &path, text.as_deref())
                        .expect("the emitted file request is current"),
                );
            }
            Out::Call { id } => {
                let Some(host) = host else {
                    unreachable!("a session not hosted asks for no call")
                };
                // ponytail: the host's own output bypasses `last`, so a
                // prompt after a `put out` without LF shares its line.
                let mut fatal = false;
                outs.extend(
                    repl.answer(&id, |request, tables| {
                        match (host.answer)(request, tables) {
                            Ok(reply) => reply,
                            Err(reason) => {
                                fatal = true;
                                Reply::Value(Err(reason))
                            }
                        }
                    })
                    .expect("the emitted host call is current"),
                );
                if fatal {
                    return Err(());
                }
            }
            Out::Start { resume, .. }
            | Out::End { resume, .. }
            | Out::Reset { resume, .. } => {
                outs.extend(
                    repl.resume(&resume)
                        .expect("the emitted boundary is current"),
                );
            }
            Out::Exit => return Ok(false),
        }
    }
    Ok(true)
}

/// Write `bytes`, with an LF before a trailing `prompt` when the output
/// before it, in these bytes or earlier, does not end one.
fn write_edited(
    output: &mut Output,
    bytes: &[u8],
    prompt: &str,
    last: &mut u8,
) -> Result<(), ()> {
    let prompt = prompt.as_bytes();
    let Some(body) = bytes.strip_suffix(prompt).filter(|_| !prompt.is_empty())
    else {
        output.write(bytes)?;
        *last = bytes.last().copied().unwrap_or(*last);
        return Ok(());
    };
    output.write(body)?;
    if body.last().copied().unwrap_or(*last) != b'\n' {
        output.write(b"\n")?;
    }
    output.write(prompt)?;
    // The line typed after the prompt ends with the LF Enter writes.
    *last = b'\n';
    Ok(())
}

/// Input as it comes, from a pipe, a file or a terminal left cooked.
fn piped(
    repl: &mut Repl,
    outs: &mut VecDeque<Out>,
    host: &mut Option<Host<'_>>,
    output: &mut Output,
) -> Result<(), ()> {
    let mut buffer = [0; 4096];
    // Feed one line at a time, writing its output before the next line.
    let mut unfed: Vec<u8> = Vec::new();
    loop {
        if !act(repl, outs, None, host, output)? {
            return Ok(());
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
        match read(output, &mut buffer) {
            Ok(0) => {
                finish_input(repl, outs, None, host, true, output)?;
                return Ok(());
            }
            Ok(read) => unfed.extend_from_slice(&buffer[..read]),
            Err(()) => {
                finish_input(repl, outs, None, host, false, output)?;
                return Err(());
            }
        }
    }
}

/// Drain final input on EOF, or discard it after a read error; then close explicitly.
fn finish_input(
    repl: &mut Repl,
    outs: &mut VecDeque<Out>,
    mut last: Option<&mut u8>,
    host: &mut Option<Host<'_>>,
    flush: bool,
    output: &mut Output,
) -> Result<(), ()> {
    if flush {
        outs.extend(repl.flush());
        if !act(repl, outs, last.as_deref_mut(), host, output)? {
            return Ok(());
        }
    }
    outs.extend(
        repl.close()
            .expect("all emitted boundaries have been drained"),
    );
    act(repl, outs, last, host, output)?;
    Ok(())
}

/// Keys from a terminal in raw mode, through the line editor; each
/// line entered is fed with its LF. The line is coloured by token class
/// unless `NO_COLOR` is set.
fn edited(
    repl: &mut Repl,
    outs: &mut VecDeque<Out>,
    host: &mut Option<Host<'_>>,
    output: &mut Output,
) -> Result<(), ()> {
    let color = !platform::no_color();

    let mut editor = Editor::new();
    let mut keys = Keys::new();
    let mut buffer = [0; 256];
    // The banner ends its line.
    let mut ended = b'\n';
    loop {
        if !act(repl, outs, Some(&mut ended), host, output)? {
            return Ok(());
        }
        editor.prompt(repl.prompt());
        let read = match read(output, &mut buffer) {
            Ok(0) => {
                finish_input(repl, outs, Some(&mut ended), host, true, output)?;
                return Ok(());
            }
            Ok(read) => read,
            Err(()) => {
                finish_input(
                    repl,
                    outs,
                    Some(&mut ended),
                    host,
                    false,
                    output,
                )?;
                return Err(());
            }
        };
        for key in keys.push(&buffer[..read]) {
            let mut event = editor.key(key);
            if let Event::Complete(before) = event {
                event = editor.complete(repl.complete(&before));
            }
            match event {
                Event::None | Event::Complete(_) => continue,
                Event::Draw => {
                    draw(output, &editor.view(), color)?;
                    continue;
                }
                Event::List(candidates) => {
                    let list = candidates.join("  ");
                    output.write(format!("\n{list}\n").as_bytes())?;
                    draw(output, &editor.view(), color)?;
                    continue;
                }
                Event::Line(mut line) => {
                    let prompt = repl.prompt();
                    let cursor = prompt.len() + line.chars().count();
                    draw(
                        output,
                        &View::new(prompt, line.clone(), cursor),
                        color,
                    )?;
                    output.write(b"\n")?;
                    line.push('\n');
                    outs.extend(repl.feed(line.as_bytes()));
                }
                Event::Interrupt => {
                    output.write(b"^C\n")?;
                    outs.extend(repl.interrupt());
                }
                Event::End => {
                    finish_input(
                        repl,
                        outs,
                        Some(&mut ended),
                        host,
                        true,
                        output,
                    )?;
                    return Ok(());
                }
            }
            if !act(repl, outs, Some(&mut ended), host, output)? {
                return Ok(());
            }
            editor.prompt(repl.prompt());
        }
    }
}

/// Redraw the line in place: to its start, the prompt and text, clear
/// the rest, then the cursor to its column, which counts text only.
fn draw(output: &mut Output, view: &View, color: bool) -> Result<(), ()> {
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
    output.write(bytes.as_bytes())
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
fn read(output: &mut Output, buffer: &mut [u8]) -> Result<usize, ()> {
    platform::read(buffer).inspect_err(|()| {
        let _ = output.write(b"cannot read standard input\n");
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

/// Advance a syscall write without spinning on zero progress.
#[cfg(any(feature = "linux", test))]
fn advance_write(bytes: &mut &[u8], written: isize) -> Result<(), ()> {
    if written == -4 {
        return Ok(());
    }
    let count = usize::try_from(written).map_err(|_| ())?;
    if count == 0 || count > bytes.len() {
        return Err(());
    }
    *bytes = &bytes[count..];
    Ok(())
}

#[cfg(all(test, feature = "linux"))]
fn output_worker() {
    fn fail(_: &[u8]) -> Result<(), ()> {
        assert!(platform::saved());
        platform::write(b"RAW-READY").unwrap();
        let mut byte = [0u8; 1];
        assert_eq!(platform::read(&mut byte), Ok(1));
        Err(())
    }
    assert!(platform::terminal());
    assert!(
        run_driver(&Options::default(), None, &mut Output::new(fail)).is_err()
    );
    assert!(!platform::saved());
    platform::exit(2);
}
#[cfg(all(test, feature = "std"))]
mod output_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WRITES: AtomicUsize = AtomicUsize::new(0);
    std::thread_local! { static DISPOSAL: std::cell::RefCell<Vec<Out>> = const {std::cell::RefCell::new(Vec::new())}; }
    pub(super) fn record(out: &Out) {
        DISPOSAL.with(|trace| trace.borrow_mut().push(out.clone()));
    }
    fn trace() -> Vec<Out> {
        DISPOSAL.with(|trace| core::mem::take(&mut *trace.borrow_mut()))
    }

    fn fail(_: &[u8]) -> Result<(), ()> {
        WRITES.fetch_add(1, Ordering::SeqCst);
        Err(())
    }
    fn good(_: &[u8]) -> Result<(), ()> {
        Ok(())
    }
    #[test]
    fn output_latch_stops_later_fragments() {
        WRITES.store(0, Ordering::SeqCst);
        let mut output = Output::new(fail);
        let mut last = b'x';
        assert!(
            write_edited(&mut output, b"bodyprompt", "prompt", &mut last)
                .is_err()
        );
        assert!(output.write(b"later").is_err());
        assert_eq!(WRITES.load(Ordering::SeqCst), 1);
    }
    fn hosted() -> (Repl, VecDeque<Out>) {
        let mut repl = Repl::new();
        repl.hosted(true);
        let outs = repl
            .host_sources("fixture", &[("commands", "pub touch: Integer")])
            .unwrap();
        let mut outs = VecDeque::from(outs);
        act(
            &mut repl,
            &mut outs,
            None,
            &mut None,
            &mut Output::new(good),
        )
        .unwrap();
        (repl, outs)
    }
    #[test]
    fn output_fatal_callback_closes_before_buffered_effect() {
        let (mut repl, mut outs) = hosted();
        outs.extend(repl.feed(b"touch\n\ntouch\n\n"));
        let mut calls = 0;
        let mut answer = |_: &Request, _: &Tables| {
            calls += 1;
            Err(String::from("transport"))
        };
        let mut host = Some(Host {
            name: "fixture",
            sources: &[],
            answer: &mut answer,
        });
        assert!(act(
            &mut repl,
            &mut outs,
            None,
            &mut host,
            &mut Output::new(good)
        )
        .is_err());
        drop(host);
        assert_eq!(calls, 1);
        assert!(outs.iter().any(|out| matches!(
            out,
            Out::End {
                result: fipe_repl_core::RunEnd::Fault(_),
                ..
            }
        )));
        dispose(&mut repl, &mut outs);
        assert!(repl.close().unwrap().is_empty());
    }
    #[test]
    fn output_write_failure_cancels_queued_start() {
        let (mut repl, mut outs) = hosted();
        outs.push_back(Out::Write(b"fail".to_vec()));
        outs.extend(repl.feed(b"touch\n\n"));
        assert!(act(
            &mut repl,
            &mut outs,
            None,
            &mut None,
            &mut Output::new(fail)
        )
        .is_err());
        assert!(outs.iter().any(|out| matches!(out, Out::Start { .. })));
        dispose(&mut repl, &mut outs);
        assert!(repl.close().unwrap().is_empty());
    }
    #[test]
    fn output_recoverable_reply_keeps_later_call() {
        let (mut repl, mut outs) = hosted();
        outs.extend(repl.feed(b"touch\n\ntouch\n\n"));
        let mut calls = 0;
        let mut answer = |_: &Request, _: &Tables| {
            calls += 1;
            Ok(Reply::Value(Err(String::from("ordinary"))))
        };
        let mut host = Some(Host {
            name: "fixture",
            sources: &[],
            answer: &mut answer,
        });
        assert!(act(
            &mut repl,
            &mut outs,
            None,
            &mut host,
            &mut Output::new(good)
        )
        .unwrap());
        drop(host);
        assert_eq!(calls, 2);
        dispose(&mut repl, &mut outs);
    }
    #[test]
    fn output_dispose_preserves_busy_end_and_reset_without_next_start() {
        let (mut repl, mut outs) = hosted();
        outs.extend(repl.feed(b"touch\n\ntouch\n\n"));
        let start = outs
            .iter()
            .find_map(|out| {
                if let Out::Start { resume, .. } = out {
                    Some(resume.clone())
                } else {
                    None
                }
            })
            .unwrap();
        outs = VecDeque::from(repl.resume(&start).unwrap());
        let call = outs
            .iter()
            .find_map(|out| {
                if let Out::Call { id } = out {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .unwrap();
        outs = VecDeque::from(
            repl.answer(&call, |_, _| {
                Reply::Value(Ok(fipe::value::Value::Int(1)))
            })
            .unwrap(),
        );
        assert!(outs.iter().any(|out| matches!(
            out,
            Out::End {
                result: fipe_repl_core::RunEnd::Success,
                ..
            }
        )));
        trace();
        let original = outs
            .iter()
            .find_map(|out| {
                if let Out::End {
                    run,
                    result,
                    resume,
                } = out
                {
                    Some((run.clone(), result.clone(), resume.clone()))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(repl.close(), Err(fipe_repl_core::DriveError::Busy));
        assert!(outs.iter().any(|out|matches!(out,Out::End{run,result,resume} if (run,result,resume)==(&original.0,&original.1,&original.2))));
        dispose(&mut repl, &mut outs);
        assert!(repl.resume(&original.2).is_err());
        let terminals = trace();
        assert!(matches!(
            &terminals[..],
            [
                Out::End {
                    result: fipe_repl_core::RunEnd::Success,
                    ..
                },
                Out::End {
                    result: fipe_repl_core::RunEnd::Abandoned,
                    ..
                },
                Out::Reset {
                    reason: fipe_repl_core::ResetReason::Exit,
                    ..
                }
            ]
        ));
        assert!(repl.close().unwrap().is_empty());
        let (mut repl, mut outs) = hosted();
        outs.extend(repl.feed(b"retained = 1\n\n"));
        act(
            &mut repl,
            &mut outs,
            None,
            &mut None,
            &mut Output::new(good),
        )
        .unwrap();
        trace();
        outs.extend(
            repl.host_sources(
                "replacement",
                &[("commands", "pub next: Integer")],
            )
            .unwrap(),
        );
        assert!(outs.iter().any(|out| matches!(
            out,
            Out::Reset {
                reason: fipe_repl_core::ResetReason::HostReplacement,
                ..
            }
        )));
        dispose(&mut repl, &mut outs);
        let terminals = trace();
        assert!(matches!(
            &terminals[..],
            [
                Out::Reset {
                    reason: fipe_repl_core::ResetReason::HostReplacement,
                    ..
                },
                Out::End {
                    result: fipe_repl_core::RunEnd::Abandoned,
                    ..
                },
                Out::Reset {
                    reason: fipe_repl_core::ResetReason::Exit,
                    ..
                }
            ]
        ));
        assert!(repl.close().unwrap().is_empty());
    }
    #[test]
    #[ignore = "requires a live PTY and FIPE_OUTPUT_WORKER"]
    fn output_pty_worker() {
        if std::env::var_os("FIPE_OUTPUT_WORKER").is_none() {
            return;
        }
        fn worker_fail(_: &[u8]) -> Result<(), ()> {
            assert!(platform::saved());
            platform::write(b"RAW-READY").unwrap();
            let mut byte = [0u8; 1];
            assert_eq!(platform::read(&mut byte), Ok(1));
            Err(())
        }
        assert!(platform::terminal());
        assert!(run_driver(
            &Options::default(),
            None,
            &mut Output::new(worker_fail)
        )
        .is_err());
        assert!(!platform::saved());
        platform::exit(2);
    }
    #[test]
    fn output_linux_progress() {
        let original = b"abcd";
        let mut bytes = original.as_slice();
        advance_write(&mut bytes, 2).unwrap();
        assert_eq!(bytes, b"cd");
        advance_write(&mut bytes, -4).unwrap();
        assert_eq!(bytes, b"cd");
        assert!(advance_write(&mut bytes, 0).is_err());
        assert!(advance_write(&mut bytes, -32).is_err());
        assert!(advance_write(&mut bytes, 3).is_err());
    }
}
