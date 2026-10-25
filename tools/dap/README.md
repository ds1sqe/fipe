# fipe-dap

A debug adapter for Fipe: the Debug Adapter Protocol over stdio, as a
package beside the crate. It depends on `fipe` and `fipe-host` by path
and on `serde_json`; the crate depends on nothing of it. Every value
the adapter shows is the VM's and every stop is one the VM took
(`vm::Machine::begin`, `begin_with_input`, `next_stop`, `answer`).

## Launch configuration

```json
{
  "type": "fipe",
  "request": "launch",
  "program": "/path/to/program.fipe",
  "host": "/path/to/host-declarations",
  "trace": "/path/to/trace.txt",
  "cwd": "/path/to/run/in",
  "allowWrite": ["/another/dir"],
  "args": ["a", "b"],
  "input": "text for the last line\n",
  "budget": 1000000
}
```

- `program`: the file to run. File modules it `use`s are read from its
  directory by stem.
- `host`: a directory of host declaration files; `commands.fipe` is the
  command namespace, every other `*.fipe` a host module by its stem.
  Without one, the std host's own declarations (`tools/host/decl`).
- `trace`: a trace fixture of the crate's conformance host
  (`tests/fixtures/traces/*.txt`, its format in
  `tests/fixtures/traces/README.md`). The replay host answers the Nth host
  call with the Nth recorded reply; a call the trace does not record
  there is answered with a failure, which Faults the run at that call.
  Without a trace the run is live on the std host
  ([`tools/host/README.md`](../host/README.md)): `cwd` (the working
  directory, the adapter's own by default) and each `allowWrite`
  directory are the write roots, `args` is `cli.args`, and `input` is
  the host input the last line takes (a launch whose `input` the
  program cannot take fails, before any effect).
- `budget`: the run's budget in the VM's units, a natural number;
  without one the run is unbounded. Anything else fails the launch.

## Supported requests

| Request | What it does |
| --- | --- |
| `initialize` | answers `supportsConfigurationDoneRequest`, `supportsBreakpointLocationsRequest` and `supportsExceptionInfoRequest`; `linesStartAt1` and `columnsStartAt1` set how every line and column is counted both ways (from one by default) |
| `launch` | checks and lowers the program, then sends `initialized`; a program that does not check fails the request, naming its first diagnostic in words; `stopOnEntry` stops before the first node |
| `setBreakpoints` | a line, or a line and the column of a pipe mark (`\|`, `\|:`, `\|!`, `\|\|`, `&&`) for a stage; a column with no mark, or a line nothing runs on, is not verified; a line breakpoint stops each time a frame arrives at its line, not again while the line runs a closure written on it or when the run comes back to a `with` or pattern on it after their code; every call arrives anew, a recursive one included |
| `breakpointLocations` | the pipe marks of the lines asked for: the stage columns |
| `configurationDone` | runs the program to a breakpoint, its end or its Fault |
| `continue` | runs to the next breakpoint, the end or a Fault; after a Fault the run cannot go on, so `continue`, `next`, `stepIn` and `stepOut` end it (`exited` with code 1, `terminated`) |
| `next` | to the next stage or line in this frame, or back in a caller |
| `stepIn` | as `next`, or into the body of the function a stage or call enters (a host call or a prelude operation has none, and is stepped over) |
| `stepOut` | back into the caller |
| `pause` | stops a running program at its next stop |
| `threads` | the one thread, `1` |
| `stackTrace` | the body frames, innermost first, named by their definition, `main`, a module's init or `arm at line:column`; at a pipe the column is its mark; after a Fault, one frame where it happened |
| `scopes` | for a `frameId` of the stop (an unknown one is refused): Locals (each local the source names), Captures (a closure's), Globals (the module-level values stored so far, of the program and its file modules), Pipe (the top frame's pipe: `mark`, `in`, `side`, `stage`, `out`), Effects (the host calls so far with their replies) and Budget (`remaining`, `budget`, `charged`) |
| `variables` | a scope's items; a record opens by field label, a sequence or tuple by index |
| `evaluate` | the value of a name bound in the frame (`frameId`, the top one when absent; an unknown one is refused): a local, then a capture, then a module-level value; never an expression |
| `exceptionInfo` | the Fault's located description and reason |
| `disconnect` | ends the session |

Events: `output` events carry, by category:

- `stdout` and `stderr`: the program's output, each top-level line's
  value when its line ends, by the conformance host's policy (`()` and
  `Nothing` write nothing, `Just v` and `Ok v` write `v`, `Err e`
  writes `e` to `stderr`; the header of
  `tests/fixtures/host/commands.fipe`), and a Fault's located
  description on `stderr`; live, the std host's own bytes by its sink
  policy (a `String` as its text, `put out` in order with the lines),
  sent at each stop and after each call;
- `console`: the trace's lines, each host call as the trace fixture
  writes it (`call ns.label (args) -> reply`), then the run's value
  (`value …`) or its Fault;
- `important`: a frame on the input the adapter could not read (no
  `Content-Length`, a body longer than 16 MiB, not JSON, not a request
  object); the session goes on, and a request without a `command` is
  answered with a failure.

`exited` and `terminated` end a run; `stopped` reports a breakpoint, a
step, a pause, an entry stop or a Fault (reason `exception`).

Every value shown is the VM's own, written by the crate's structural
renderer; every stop is one the VM took. Stopping changes no
observation: a session stepped line by line through an example sends
the host calls, value or Fault, success and error output of its trace
fixture (`tests/stepping.rs`).

## What it does not do

- `evaluate` reads a name only: no expression, field access, call or
  assignment, since checking an expression in a paused frame's scope
  would be a second checker.
- No `setVariable`, `restart`, `goto`, `stepBack`, data or function
  breakpoints, conditions or hit counts, `loadedSources` or
  `disassemble`; each is refused as unsupported.
- Under a trace, writes to a sink (`put`, `append`) are replayed as
  host calls, not applied: the success output holds the lines' values
  only, which is what the examples' fixtures hold. Live, they land
  under the write roots.
- The one editor client run against it is nvim-dap, through the
  module `tools/editor/nvim` (its smoke stops at a breakpoint and runs
  to `terminated`); the recorded session below is the binary over its
  real stdio.

## Checks

Scripted sessions: `tests/sessions/*.json` (one per example with a
trace fixture, run by `tests/sessions.rs`) and `tests/stepping.rs`.
`scripts/record.sh` records a whole session of `02-outcomes.fipe`
(breakpoint on a stage, `next` across its pipes, Pipe, Effects and
Budget scopes, `evaluate`, `continue`) over the real binary, one JSON
message per line.

```sh
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
```

## Command discovery

```sh
fipe-dap --version
fipe-dap --help
fipe-dap -h
```

`--version` prints `fipe-dap <package-version>` and exits successfully.
Help prints usage and exits successfully. These commands take no protocol input
and run before a session starts. Version accepts no extra operands.
Other process arguments are rejected with usage on stderr and exit 1; older
prerelease builds ignored them. Launch settings belong in DAP requests. With no
arguments, stdio remains protocol-only and EOF retains its successful exit.

Resource values display as `<resource>` without expandable payloads. Recorded text cannot reconstruct a resource; ordinary data replay remains available.


## Header and body limits

Headers are limited to 8192 bytes per line and 32768 bytes per header section,
including LF/CRLF terminators and the empty separator. Inter-frame blank lines
also count toward the section limit. Exactly-at-limit headers are accepted; an
over-limit header ends input with an `InvalidData` error, without reading its
body or trying to find a later frame. These are Fipe limits, not protocol-standard
limits. The adapter reports the framing error on stderr and exits 1. Other reader or writer I/O errors also fail the process; ordinary EOF and disconnect remain successful.

The existing body limit is 16 MiB; oversized-body handling and malformed-JSON
recovery remain distinct from fatal header overflow. Byte limits do not impose a
wall-clock deadline on a peer that stops sending. External supervisors own process
deadlines and cleanup. No general arbitrary-stream resynchronization is promised.
