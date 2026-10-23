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
../../target/release/fipe-lsp --host ../../tests/fixtures/host
```

`--host <dir>` names the directory of the host's declaration files
(`commands.fipe` and one file per host module). A dotted filename such as
`text.open.fipe` registers the nested host module `text.open`; its parent
namespace is created automatically. A client may name it
instead in `initializationOptions` as `{"host": "<dir>"}`. File
modules (`use geometry`) are `geometry.fipe` beside the document,
read from the editor's open buffers first, then from disk.

Dependencies: `lsp-types`, `serde`, `serde_json`. The transport
(Content-Length framing, one thread, no async runtime) is written in
the package. The `fipe` crate keeps no dependency.

The server follows the protocol's lifecycle: a request before
`initialize` is refused (-32002), as is a second `initialize` and any
request after `shutdown` (-32600); `exit` ends the process with 0
after `shutdown` and 1 without it. A message that is not JSON is
answered with -32700, and one longer than 64 MiB is dropped unread and
answered the same way; the server goes on. Documents synchronize
incrementally. Each document is analysed once per change and the
analysis serves every request until the next change; a change to an
open document republishes the other open documents that use it, and
`workspace/didChangeWatchedFiles` re-analyses everything, for files
changed on disk.

## Offline interface inputs

Editors can explicitly select declaration-only `.fipi` sources during initialization:

```json
{
  "initializationOptions": {
    "interfaces": [
      { "module": "demo.math", "path": "/absolute/interfaces/arithmetic.fipi" }
    ]
  }
}
```

Each entry uses an explicit dotted module identity and an absolute native filesystem
path ending in `.fipi`; paths are not URIs and have no tilde/environment expansion.
The basename does not choose identity. List dependencies first: legacy `host`
declarations load first, then interfaces in the supplied order. The existing host
directory scan still loads only `.fipe`. Missing `interfaces` or an empty list keeps
the existing behavior.

Selection is atomic. Duplicate explicitly selected modules, including conflicts
with legacy host sources or repeated identical module/path entries, fail with both source paths.
Synthesized namespace parents are not selected sources. Invalid configuration,
read/UTF-8 failures and declaration errors return initialize error -32602 with
module/path context and source line/column when available. Failed initialization
leaves the server waiting (-32002 for analysis requests); a corrected initialize
request may retry, though editor retry behavior varies. A missing or invalid
selected interface never falls back to a sibling implementation or provider.

Interfaces use existing host declaration syntax, including written constants,
defaults, type limits and impls. Editing/checking needs no provider binary, native
loading, build/generator execution, SDK or device. Existing bounded pure checking
is allowed; runtime-dependent defaults remain runtime work. Hover/completion and
definition use the selected text, with exact Unicode/line-ending spans and actual
`.fipi` paths; definition URIs use the existing canonical path behavior.

The selected module/path list stays fixed until a new server is initialized.
Selected `didOpen`/`didChange`/`didClose` and `workspace/didChangeWatchedFiles` events
reload the entire interface batch atomically, using open text before disk. Full
and incremental edits use the negotiated position encoding. Closing a buffer
returns to disk; a watched event cannot overwrite unsaved text. Legacy host
sources stay frozen. Clients must send document/watcher notifications; this server
does not dynamically register filesystem watchers.

A failed reload retains every last accepted declaration/source and reports the
failure on the selected source. All ordinary open clients also receive an error
warning that last accepted metadata is in use. Repair clears those diagnostics.
Hover/completion can continue from that visibly marked metadata, but navigation
to all selected interfaces is suppressed until the batch is valid. Without an
open buffer, a disk edit before its watcher event also suppresses stale locations.
With a matching accepted open buffer, navigation targets that buffer even if its
disk file was deleted.

Events recognize only each original selected path and its canonical path captured
at initialization, using the existing file-URI decoder (including percent escapes).
Both open spellings must have identical text; conflicting buffers fail until they
agree or one closes. Original open URI wins when both agree, then canonical open
URI; disk-only locations use the original path with matching text. Other symlink
aliases are not discovered. Retargeting a symlink requires a new server/selection;
delete/recreate at the same target is supported. Renaming or creating unselected
files cannot change module selection. Windows URI/watcher behavior still needs
its platform qualification.

Selected declaration buffers bypass ordinary script diagnostics. Client documents
use their admitted metadata; semantic features and formatting *inside* selected
`.fipi` buffers remain unavailable. These protocol guarantees do not establish a
real editor's watcher configuration or full interface-authoring support.

This text-only input carries existing unmarked data declarations. It does not
infer resources from opaque types, import resource-bearing artifact metadata,
load libraries, or establish runtime artifact identity/authority. Typed resource
classification and its preservation require the shared resource contract.

## Clients

- Neovim: the module `tools/editor/nvim` (its README) for your own
  config. `editor/init.lua` is a config of that module alone, without
  the debug adapter (`nvim -u tools/lsp/editor/init.lua`; `FIPE_LSP`
  and `FIPE_HOST` override the server and host paths).
- Helix: `editor/languages.toml`, to merge into your
  `languages.toml` with your paths; `tools/editor/helix` (its README)
  joins it with the grammar and has the smoke (hover, definition and
  a diagnostic, in Helix 25.07).

## Actions

| Request | What it does |
| --- | --- |
| diagnostics | on every open and change, from `check`: syntax errors first, then the diagnostics of the rest of the file; a used file's errors are shown once at its `use`, and what the file's own errors cause in the document is not repeated |
| definition | locals, the module's names, names from file modules, host and prelude declarations (the prelude is written once to a file under the system's temporary directory, since it has none), a type member (`Point.origin`) and a value's field or member (`p.dist`, `self.v`) by the head's type; on a `use` segment, the module's file or the imported name's declaration |
| references | every use in the document and its file modules, the `use` segments that import the name (in the sibling files too, for a name the document defines), and the declaration when asked |
| hover | a binding's inferred scheme, an expression's or a pattern's type, a declared name in an item (type, constructor, field, impl member) with its docs, a flag's spot with its doc line, or a host or prelude declaration with its docs; on a `use` segment, the module or the imported declaration |
| completion | after `use`, after `.`, on a flag (`-label`, `-label=` for a non-Bool spot), at a flag's value (the names in scope), and the names in scope; nothing inside a quote, a comment or a doc |
| rename | the declaration and every use across the document, its file modules and the sibling files that import the name |
| formatting | the whole document: indentation by open brackets and continuing pipes, one space between tokens that do not touch; CRLF files keep CRLF |
| signature help | on a space or `(` after a command with labeled spots (a host command, a module's function, a prelude or document function): one signature `name (label: type)…`, each spot a parameter with its doc line; the active spot is the first the checker left open at that call, else the count of operands written |
| document symbols | the top-level bindings (functions by their scheme), signature-only declarations and types with their fields and constructors, hierarchical |
| workspace symbols | the same symbols of every open document and the file modules beside them, matched by the query's characters in order, ignoring case |
| inlay hints | an unannotated binding's scheme after its name (`: Integer`); `initializationOptions.inlayHints = false` turns them off |
| code actions | on "unknown name", `Import \`m.name\`` for each host module or sibling file exporting it, inserted after the last `use`; on an unannotated binding, `Annotate \`name: scheme\``, offered only when the document checks with it; the crate's suggestions as quick fixes: double the parentheses where both call readings bind, and `\|\|` or `\|:` where an outcome enters a text place |

Positions are UTF-8 when the client offers it, UTF-16 otherwise.

What the actions do not do:

- Formatting refuses a file with any diagnostic (syntax, resolution or
  type), and a result whose tokens would differ from the input's. It
  keeps the author's line breaks.
- Completion after a dot re-checks the document with the half-typed
  member removed, once per request: as costly as a full check.
- Definition of a member chain longer than one member answers nothing:
  only the head's type is known.
- Rename refuses prelude, host, command, session-line and module
  names, a field or member (rename at the chain's head renames the
  head), a new name of another case or not one segment, and a new name
  already written in the file, which could capture or shadow it.
- Workspace-wide operations stop at the document, its file modules
  and the `.fipe` files beside it.
- Signature help knows labeled spots only: a function of bare spots
  answers nothing. Inlay hints and the annotation action write the
  scheme as the crate spells it, with its variables and obligations.

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
