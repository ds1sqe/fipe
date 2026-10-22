# fipe-dap

A debug adapter for Fipe: the Debug Adapter Protocol over stdio, as a
package beside the crate. It depends on `fipe` by path and on
`serde_json`; the crate depends on nothing of it. Every value the
adapter shows is the VM's and every stop is one the VM took
(`vm::Machine::begin`, `next_stop`, `answer`).

## Launch configuration

```json
{
  "type": "fipe",
  "request": "launch",
  "program": "/path/to/program.fipe",
  "host": "/path/to/host-declarations",
  "trace": "/path/to/trace.txt",
  "budget": 1000000
}
```

- `program`: the file to run. File modules it `use`s are read from its
  directory by stem.
- `host`: a directory of host declaration files; `commands.fipe` is the
  command namespace, every other `*.fipe` a host module by its stem.
- `trace`: a trace fixture of the crate's conformance host
  (`tests/fixtures/traces/*.txt`, its format in
  `tests/fixtures/traces/README.md`). The replay host answers the Nth host
  call with the Nth recorded reply; a call the trace does not record
  there is answered with a failure, which Faults the run at that call.
  Without a trace every host call fails.
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
  description on `stderr`;
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
- The standalone host only replays a trace fixture; a host doing real
  input and output is the embedding editor's.
- Writes to a sink (`put`, `append`) are replayed as host calls, not
  applied: the success output holds the lines' values only, which is
  what the examples' fixtures hold.
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
