# Working with Fipe as a coding agent

Use the documentation and tools from the same Fipe prerelease as the project.
The current package version is `0.2.0-dev.2`; published `0.1.0` has a different
language. Do not infer Fipe syntax from shell, Rust or another language.

## Write a script

Start with the [tour](tour/README.md), then use the [specification index](spec/README.md)
for the relevant rule. The [prelude](../prelude/prelude.fipe) lists built-in
functions and types; [standard-host declarations](../tools/host/decl/) list
filesystem, environment and output operations. A custom embedding can expose
a different host, so check its declarations before using a command.

```fipe
text: String -> Natural = input => input | words | len
text
```

Save it as `count.fipe`. Validate before execution:

```sh
fipe check count.fipe
printf 'one two three\n' | fipe run count.fipe --budget 10000
```

The result is `3`. `check` emits diagnostics as `file:line:column: reason`
and runs no effects. Exit codes differ by command:

| Command | 0 | 1 | 2 |
| --- | --- | --- | --- |
| `check` | All files check | Check error | Unreadable input or invalid usage |
| `run` | Successful result | `Err` or Fault | Check error, refused input, loading or usage error |

Read stdout as program output and stderr as diagnostics. A VM budget bounds
execution work, not wall-clock time or memory. The standard host can read
files the process can access; writes are restricted to the working directory
and explicit `--allow-write` roots. Choose the working directory deliberately
when running scripts that modify files.

Functions use `x => body`; calls are `f x`; group nested calls as `f (g x)`.
A pipeline passes a value into the next stage. `Maybe` and `Result` need the
appropriate [outcome pipe or pattern](tour/03-outcomes.md). File modules are
loaded beside the script. Hosts may register dotted namespaces: `use text.open`
imports the submodule as `open`; `text.open.new` stays fully qualified, and
`use text.open.new` imports its public function. There is no shell process-spawning command in the
standard host.

## Change the language or tools

Follow [development](development.md) for workspace commands. Find the owning
[spec chapter](spec/README.md), implementation and conformance tests before
editing a rule. Update all three when behavior changes. [Open decisions](spec/11-open.md)
are unresolved proposals, not implemented features.

Use `cargo test -p PACKAGE` for a focused check, then the workspace checks
for changes shared by packages. `cargo run --example custom_host` exercises
the host request/reply boundary. Keep the core dependency-free and no_std;
add application effects through host declarations and replies.

The [publishing guide](publishing.md) describes the package verification and
release commands. Test extracted packages when changing manifests or bundled
assets: a workspace build alone cannot detect missing files in an archive.
