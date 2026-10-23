# fipe

The `fipe` command: Fipe scripts on this machine, through the std host
([`tools/host/README.md`](../host/README.md) is the contract: the
declarations, the sink policy, input, exit codes and write roots).

```sh
fipe run FILE [--budget N] [--allow-write DIR]... [-- ARG...]
fipe check FILE...
fipe repl [--strict] [--budget N] [--compile-budget N] [--allow-write DIR]...
fipe build FILE -o EXE [--runtime LIB] [--llvm]
fipe help
```

`run` loads the script and the file modules it `use`s beside it, checks
the unit against the std host's declarations and runs it: the success
output to stdout and the error output to stderr as the run goes, stdin
as host input when the last line takes it, the arguments after `--` as
`cli.args`, `--budget` the VM's charge units (unbounded otherwise).
Exit 0 a value, 1 an `Err` value or a Fault, 2 when the script could
not run: it does not check (each diagnostic as `FILE:LINE:COLUMN:
reason`), or its input is refused.

`check` checks each file the same way and prints nothing for one that
checks: exit 0 when all do, 1 when one does not, 2 when one cannot be
read or the usage is wrong.

`repl` is `fipe-repl` ([`tools/repl/README.md`](../repl/README.md))
over the std host: each host call is answered on this machine, `put
out` writes to stdout in order with the lines, and a Fault in a
declaration is located under `tools/host/decl`. `:host DIR` still
switches to a directory's declarations, whose calls then Fault.

`build` compiles the script with `fipe-native`
([`tools/native/README.md`](../native/README.md)) into an x86-64
Linux executable carrying the std host's declarations: run without a
host pipe, it answers through the std host in the process, as `run`
does, with `FIPE_ALLOW_WRITE` (directories separated by `:`) as its
further write roots and its own arguments as `cli.args`. The runtime
archive is `--runtime LIB` or `FIPE_NATIVE_RT`. With `--llvm` the
object comes from the LLVM backend ([`tools/llvm`](../llvm/README.md)),
LLVM IR compiled by `clang-23`, linked against the same runtime. Exit 0
when built, 1 when it cannot be built or linked, 2 when the script does
not check.

One script gives equal stdout, stderr and exit code under `run` and
as an executable from either backend; `tests/cli.rs` holds that for
exit 0, 1 and 2.

## Development

```sh
cargo fmt --check
cargo test --locked --offline
cargo test --locked --offline --release
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
```

The tests build the native runtime with `cargo` and link with `cc`.
