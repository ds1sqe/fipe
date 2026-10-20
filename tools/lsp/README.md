# fipe-lsp

A language server for Fipe over the `fipe` crate, speaking the
Language Server Protocol over stdio. It analyses documents; it never
runs a program. The crate does every stage (lexing, parsing,
resolution, inference, checks) and the server presents what the crate
answers.

## Build and run

```sh
cd tools/lsp
cargo build --locked --offline --release
target/release/fipe-lsp --host ../../tests/fixtures/host
```

`--host <dir>` names the directory of the host's declaration files
(`commands.fipe` and one file per host module). A client may name it
instead in `initializationOptions` as `{"host": "<dir>"}`. File
modules (`use geometry`) are `geometry.fipe` beside the document,
read from the editor's open buffers first, then from disk.

Dependencies: `lsp-types`, `serde`, `serde_json`. The transport
(Content-Length framing, one thread, no async runtime) is written in
the package. The `fipe` crate keeps no dependency.

## Clients

- Neovim, built-in client: `editor/init.lua` (`nvim -u
  tools/lsp/editor/init.lua`, or copy its body into your config with
  your paths; `FIPE_LSP` and `FIPE_HOST` override the server and host
  paths).
- Helix: `editor/languages.toml`, to merge into your
  `languages.toml` with your paths. Not tested here: Helix was not
  installed when the package was written.

## Actions

| Request | What it does |
| --- | --- |
| diagnostics | on every open and change, from `check`: syntax errors first, then the diagnostics of the rest of the file; an error inside a used file is placed at its `use` |
| definition | locals, the module's names, names from file modules, host declarations (in their declaration file); on a `use` segment, the module's file or the imported name's declaration |
| references | every use in the document and its file modules, the `use` segments that import the name (in the sibling files too, for a name the document defines), and the declaration when asked |
| hover | a binding's inferred scheme, an expression's type, or a host or prelude declaration with its docs; on a `use` segment, the module or the imported declaration |
| completion | after `use`, after `.`, on a flag (`-label`, `-label=` for a non-Bool spot), and the names in scope |
| rename | the declaration and every use across the document, its file modules and the sibling files that import the name |
| formatting | the whole document: indentation by open brackets and continuing pipes, one space between tokens that do not touch |

Positions are UTF-8 when the client offers it, UTF-16 otherwise.

What the actions do not do:

- Formatting refuses a file with any diagnostic (syntax, resolution or
  type), and a result whose tokens would differ from the input's. It
  keeps the author's line breaks.
- Completion after a dot re-checks the document with the half-typed
  member removed, once per request: as costly as a full check.
- Rename refuses prelude, host, command, session-line and module
  names, and a new name of another case or not one segment.
- Workspace-wide operations stop at the document, its file modules
  and the `.fipe` files beside it.

## Checks

```sh
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery
RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps
sh editor/smoke.sh      # the headless Neovim smoke
```

The smoke builds the release server and runs Neovim with
`editor/init.lua` and every XDG directory under a scratch directory,
so it reads no user configuration or data. It opens
`05-modules.fipe`, sees a located check error after an edit and its
clearing, jumps to the definition of a used module's type in
`geometry.fipe`, and exits 0; any failed step exits 1.
