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
| `initialize` | answers `supportsConfigurationDoneRequest`, then sends `initialized` |
| `launch` | checks and lowers the program; a program that does not check fails the request |
| `configurationDone` | runs the program: one `output` event per host call, written as the trace writes it; `exited` and `terminated` at its end; `stopped` with reason `exception` and the located Fault otherwise |
| `threads` | the one thread, `1` |
| `disconnect` | ends the session |

Breakpoints, stepping, scopes and `evaluate` are not supported yet.

## Checks

```sh
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
```
