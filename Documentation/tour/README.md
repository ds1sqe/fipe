# A tour of Fipe

This tour is for scripts using the standard host. For an application-owned Host,
start with [embedding in Rust](../../README.md#embed-in-rust); the
[documentation index](../README.md) routes to APIs, tools and contributor guides.

Five pages, each built on the scripts under
[`dogfood/`](../dogfood/), which run on the std host with the `fipe`
command ([`tools/fipe`](../../tools/fipe/README.md)). Every fenced
`fipe` snippet is a whole script that checks; `tools/fipe/tests/tour.rs`
checks each one.

1. [A first script](01-first-script.md): lines, values, the output and
   the exit code.
2. [Pipes and functions](02-pipes-and-functions.md): `|`, lambdas,
   `map`, `filter`, `fold`, the script's arguments.
3. [Outcomes](03-outcomes.md): `Maybe` and `Result`, their patterns
   and pipes, errors a script reports.
4. [Files and the host](04-files-and-host.md): `ls`, `cat`, `put`,
   the write roots, dry runs.
5. [The tools](05-tools.md): `check`, `repl`, `build`, the editor.

The [specification](../spec/README.md) defines each rule the tour
shows; the [examples index](../examples/README.md) distinguishes runnable programs
from language and conformance-host fixtures.
