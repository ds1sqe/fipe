# fipe-repl

The Fipe REPL: one pump over `fipe-repl-core`, built with the `std`
platform (default) or for x86_64 Linux with no `std`
(`--no-default-features --features linux`). The package is a library,
`fipe_repl::run` with its `Options` and an optional `Host` answering
the session's calls, and the `fipe-repl` binary over it; `fipe repl`
([`tools/fipe`](../fipe/README.md)) is the same pump over the std host.

```sh
fipe-repl [--cwd <dir>] [--host <dir>] [--strict] [--budget <n>] \
  [--compile-budget <n>]
```

An entry is the input up to a blank line; `....> ` prompts for its next
line. A line starting with `:` is a command; `:help` lists them, one line
each, and `:help name` prints a name's doc and type as `:t` does.

At a terminal the line is edited in place: Left, Right, Home and End
(Ctrl-A, Ctrl-E) move; Backspace, Delete, Ctrl-K, Ctrl-U and Ctrl-W
delete; Up and Down recall earlier lines; Tab completes the names in
scope, the commands, a module's members (`fs.li` to `fs.list`) and a
value's fields and members, and a second Tab lists several candidates.
The line is coloured by token class unless `NO_COLOR` is set. Ctrl-C
drops the entry; Ctrl-D on an empty line ends input. Piped input is read
as it comes, with no editing, colour or banner.

A lone Esc typed just before `O` or `[` takes the next key with it as an
escape sequence: the editor reads with no timeout.

`python3 tools/repl/tests/pty.py BIN...`, from the repository root,
drives a binary through a terminal.

The driver drains trailing input at EOF, explicitly closes the REPL through its
Exit Reset, and restores terminal settings. Input errors discard partial input and
perform the closing protocol before exit code2. Ctrl-C abandons an unfinished
Start/Call, preserving an already computed terminal or Reset.

The optional `Host` adapter answers synchronous scalar/data calls and immediately
acknowledges lifecycle boundaries; it has no owner cleanup hook. A Host requiring
run/session resource cleanup must use a lifecycle-aware
[`fipe-repl-core`](../repl-core/README.md) driver and perform its owner actions before
acknowledging Start/End/Reset. No automatic payload cleanup or rollback is promised.

See the [support policy](../../Documentation/development.md#compatibility-and-support)
for the Rust 1.98.1 floor and platform qualification status.
The std platform is selected for Linux, Windows and both macOS architectures;
real Windows/macOS terminal qualification is pending. The exclusive linux feature
remains x86-64 Linux only and requires panic abort.

The `Host.answer` callback returns `Result<Reply, String>`: `Ok` includes ordinary
recoverable language faults; `Err` reports a fatal driver/transport failure. The
CLI adapter checks its Host output before and after effects. A rendering error or
fatal callback stops later input/effects, preserves already computed End/Reset
outcomes, closes before acknowledging another Start, restores cooked mode and
exits with code 2. Output stops after its first error; partial bytes and discarded tails are
not retried or recoverable. This is explicit disposal, not payload cleanup or
rollback. Raw mode requires both stdin and stdout to be terminals. Cleanup applies
to observed errors; external process termination, including an inherited fatal
SIGPIPE disposition, has no restoration guarantee.

Private output tests use a direct `rustc --test` harness with std dependency
artifacts built using a task-local `profile.dev.panic="unwind"` override; Cargo's
library test=false stays unchanged. The linux private worker remains panic abort
and links the actual main against a cfg(test) rlib with `-nostartfiles`. Test-only
workers inject failure through `run_driver`, not a separate cleanup implementation.
`pty.py --worker-std TEST_BINARY --worker-linux WORKER --cli FIPE BIN...` adds
live-terminal restoration and piped failure controls to the healthy sessions.
