# The tools

```sh
fipe run FILE [--budget N] [--allow-write DIR]... [-- ARG...]
fipe check FILE...
fipe repl [--strict] [--budget N] [--compile-budget N] [--allow-write DIR]...
fipe build FILE -o EXE [--runtime LIB]
fipe help
```

`check` checks files without running them, exit 0 when all check.
`run` is the tour's runner. `--budget N` bounds the run in the VM's
units, so a recursion that never ends Faults instead of hanging.

## The REPL

`fipe repl` takes lines; an entry ends at a blank line, and its value
is shown in the language's spelling. `:help` lists the commands,
`:help NAME` a name's doc and type, `:t` the last line's type, `:r` what
a name resolves to. Host calls run for real, on the std host, with the
same write roots as `run`.

```text
fipe> ls ./ | map (e => e.name)

["notes.txt", "wc.fipe"]
fipe> :t ls
```

## Executables

`fipe build wc.fipe -o wc --runtime libfipe_native_rt.a` compiles the
script with the native backend into an x86-64 Linux executable. Run
without a host pipe, it answers through the std host in the process:
the same bytes and exit code as `fipe run`, its own arguments as
`cli.args`, `FIPE_ALLOW_WRITE` as the further write roots. The runtime
archive is built from `tools/native/runtime`.

## The editor

`tools/editor/nvim` is a Neovim module; `tools/editor/helix` the Helix
settings. Both run the language server (`tools/lsp`): diagnostics on
each change, go to definition, references, hover with docs and
inferred types, completion of names, members and flags, signature help
after a command, document and workspace symbols, inlay hints with each
binding's scheme, code actions (a missing `use`, an annotation, the
checker's own quick fixes), rename and formatting.

The debug adapter (`tools/dap`) runs a script live on the std host
under nvim-dap: a breakpoint on a line, `next` across the stages of a
pipe, the scopes of the stop (locals, the pipe's `in` and `out`, the
host calls so far) and the output as it is written.

## Where to go next

The [specification](../spec/README.md) states each rule; its
[open questions](../spec/11-open.md) list what is proposed and not yet
ruled, among them the prelude's breadth and the host's entry fields,
both raised by the scripts this tour is built on.
