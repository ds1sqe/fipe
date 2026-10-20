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
  (`tests/fixtures/traces/*.txt`). The replay host answers the Nth host
  call with the Nth recorded reply; a call the trace does not record
  there is answered with a failure, which Faults the run at that call.
  Without a trace every host call fails.
- `budget`: the run's budget in the VM's units; without one the run is
  unbounded.

## Supported requests

| Request | What it does |
| --- | --- |
| `initialize` | answers `supportsConfigurationDoneRequest`, `supportsBreakpointLocationsRequest` and `supportsExceptionInfoRequest` |
| `launch` | checks and lowers the program, then sends `initialized`; a program that does not check fails the request, naming its first diagnostic in words; `stopOnEntry` stops before the first node |
| `setBreakpoints` | a line, or a line and the column of a pipe mark (`\|`, `\|:`, `\|!`, `\|\|`, `&&`) for a stage; a column with no mark, or a line nothing runs on, is not verified |
| `breakpointLocations` | the pipe marks of the lines asked for: the stage columns |
| `configurationDone` | runs the program to a breakpoint, its end or its Fault |
| `continue` | runs to the next breakpoint, the end or a Fault |
| `next` | to the next stage or line in this frame, or back in a caller |
| `stepIn` | as `next`, or into the body of the function a stage or call enters (a host call or a prelude operation has none, and is stepped over) |
| `stepOut` | back into the caller |
| `pause` | stops a running program at its next stop |
| `threads` | the one thread, `1` |
| `stackTrace` | the body frames, innermost first, named by their definition, `main`, a module's init or `arm at line:column`; at a pipe the column is its mark; after a Fault, one frame where it happened |
| `scopes` | Locals (each local the source names), Captures (a closure's), Globals (the module-level values stored so far, of the program and its file modules), Pipe (the top frame's pipe: `mark`, `in`, `side`, `stage`, `out`), Effects (the host calls so far with their replies) and Budget (`remaining`, `budget`, `charged`) |
| `variables` | a scope's items; a record opens by field label, a sequence or tuple by index |
| `evaluate` | the value of a name bound in the frame (a local, then a capture, then a module-level value); never an expression |
| `exceptionInfo` | the Fault's located description and reason |
| `disconnect` | ends the session |

Events: `output` with category `stdout` for each host call, written
as the trace fixture writes it (`call ns.label (args) -> reply`), and
one with category `console` for the run's value (`value …`) before
`exited` and `terminated`; `stopped` for a breakpoint, a step, a pause,
an entry stop or a Fault (reason `exception`).

Every value shown is the VM's own, written by the crate's structural
renderer; every stop is one the VM took. Stopping changes no
observation: a session stepped line by line through an example makes
the host calls, value or Fault of its trace fixture
(`tests/stepping.rs`).

## What it does not do

- `evaluate` reads a name only: no expression, field access, call or
  assignment, since checking an expression in a paused frame's scope
  would be a second checker.
- No `setVariable`, `restart`, `goto`, `stepBack`, data or function
  breakpoints, conditions or hit counts, `loadedSources` or
  `disassemble`; each is refused as unsupported.
- The standalone host only replays a trace fixture; a host doing real
  input and output is the embedding editor's.
- The success and error output of the conformance host's policy are
  not reproduced: they are that host's, applied to the value shown.
- No editor client has been run against it here (Neovim has no
  built-in DAP client and nvim-dap is not installed); the recorded
  session below is the binary over its real stdio.

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
