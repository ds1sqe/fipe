# fipe-repl-core

An embeddable Fipe REPL with no I/O. It uses `core` and `alloc` and depends
only on `fipe`. It is an optional Embed facility in the
[layer ownership model](../../Documentation/architecture.md); application Hosts
can use it without Standard. The driver owns I/O and output presentation.

Feed bytes to `Repl::feed`, handle each `Out` in order, and call `Repl::flush`
at end of input. The driver writes `Out::Write`, supplies file sources through
`Repl::file`, and answers host calls through `Repl::answer`. The `edit` and
`highlight` modules provide terminal editing and token classes.

Use `fipe-repl` for a terminal driver, or `fipe-cli` for the `fipe repl`
command with filesystem and environment access through the standard host.
See the crate's API documentation for the request/response protocol.

`Repl::host_sources` accepts declarations by their full module names, including
`text.open`. Supply `commands` first when present, then modules in dependency
order. The directory-based `:host` command still reads the conformance host's
fixed file list; custom hosts should supply named sources through the API.

Each `Out::File { id, path }` and `Out::Call { id }` carries an opaque
`RequestId`. Pass that ID to `file(&id, &path, text)` or
`answer(&id, |request, tables| reply)` and handle its `Result`. Every request gets
its own identity, including successive calls in one run and repeated file paths.
Cloning an ID preserves it. A stale, foreign or wrong-kind ID, or wrong file path,
returns `DriveError::WrongRequest` before changing state or invoking the answer
callback. Do not retry an old reply with the newest ID.

Every ordinary executable unit yields `Out::Start { run, resume }` before runtime
execution and `Out::End { run, result, resume }` after its VM terminal. Handle each
boundary, then pass its `ResumeId` to `Repl::resume(&resume)` and process the returned
outputs in order. Start and End are the last event in their batch. End acknowledgement
allows value/fault presentation, failed-line withdrawal and the next buffered unit.
`RunEnd::Success` includes language-level `Err` data; `RunEnd::Fault` carries the VM
fault. One `RunId` identifies both boundaries; each boundary has a fresh continuation.
A stale or foreign continuation returns `DriveError::WrongContinuation` without
changing the gate. Tokens have redacted Debug output and are not permission grants.

For example, a driver handles `Out::Start { resume, .. }` and
`Out::End { resume, .. }` by performing its boundary actions and extending its output
queue with `repl.resume(&resume)?`. File/Call handling still uses `RequestId`, never
`ResumeId`. The built-in terminal driver acknowledges boundaries immediately.

`host_sources` and `host_directory` return `DriveError::Busy` while any request,
boundary or replay is pending. Valid declarations yield
`Out::Reset { reason: ResetReason::HostReplacement, resume }` before replacing the
old session. Dispose its owner and establish the new owner context, then resume.
Invalid declarations preserve the old session and emit no Reset. Initial valid host
installation also yields Reset. Replay uses cached source texts, gives every runnable
line Start/End gates, suppresses historical runtime text and refuses all Host calls;
it never re-acquires old resources. Check diagnostics remain visible.

`interrupt` clears future input and abandons Start/Call with an End Abandoned.
The Start has not run; a suspended Call is stopped before the event. Acknowledge End
after revoking the run's new entries; only then is the current language acceptance
withdrawn. Earlier successful bindings remain. Interrupt preserves an already
computed End or Reset, including its continuation, and clears new input only.
At active replay Start it drops the remaining replay tail; an existing replay End
preserves that tail. No synchronous callback or running pure VM code is preempted.

To dispose explicitly, call `close()`, drain any returned Abandoned End, and handle
`ResetReason::Exit` before resuming to `Out::Exit`. A pending computed End or Reset
makes close return Busy unchanged; drain it and retry. :q uses this exit reset too.
`flush()` submits trailing input and drains normally; it does not dispose. At actual
EOF, custom drivers must flush, drain all gates, then close and drain Exit Reset.
The built-in driver performs that sequence on EOF; read errors discard partial
input, close, restore the terminal and then retain exit code2.

After Exit, close is idempotent; feed/flush/interrupt are empty no-ops and retain no
new input. Host replacement returns Closed. Old file/call/continuation IDs remain
invalid. Rust dropping a Repl without this protocol does not notify a Host.

Start gates runtime-unit execution, not checking/compilation's bounded pure VM
evaluation. Queries have no execution boundary. These events do not perform Host
cleanup, cancellation of native work, effect rollback or resource authorization.
The built-in `fipe-repl::Host` adapter immediately acknowledges boundaries and has
no owner hooks: resource-owning Hosts must use a lifecycle-aware repl-core driver.
Explicit Standard owner-adapter migration and actual owner proof remain required
before resource-enabled integration; event ordering alone does not prove release.

Resource values render as `<resource>`. This placeholder is display text, not executable source or replay authority; replacing host declarations never reacquires a resource during replay.
