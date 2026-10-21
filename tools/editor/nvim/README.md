# Fipe for Neovim

One Lua module, `require('fipe')`, for Neovim 0.12 or later. Its
`setup(opts)` gives:

- the `fipe` filetype for `*.fipe`;
- the language server `tools/lsp` as `vim.lsp.config('fipe', …)` and
  `vim.lsp.enable('fipe')`, with nvim-cmp's capabilities when
  `cmp_nvim_lsp` is installed;
- the tree-sitter parser of `tools/tree-sitter`, built from its `src/`
  with `cc` into `stdpath('cache')/fipe/parser/` (again when the
  sources are newer), with the queries of `tools/tree-sitter/queries`
  read in place; highlighting, `foldexpr` from the folds query, and
  nvim-treesitter's `indentexpr` from the indents query when
  nvim-treesitter is installed (Neovim has none of its own);
- the debug adapter `tools/dap` as `dap.adapters.fipe` and a launch
  configuration `dap.configurations.fipe`, when nvim-dap is installed.
  A lazy-loaded nvim-dap is configured on the first `fipe` buffer.

It works with both branches of nvim-treesitter and without it: the
parser is registered with `vim.treesitter.language.add`, not
installed through nvim-treesitter (`tools/tree-sitter/editors/nvim/`
has that snippet, for a parser fetched from the repository).

## Install

Build the server and the adapter:

```sh
(cd tools/lsp && cargo build --release)
(cd tools/dap && cargo build --release)
```

With lazy.nvim, add one spec, with your path to this directory:

```lua
{ dir = "/path/to/fipe/tools/editor/nvim", name = "fipe", main = "fipe", lazy = false, opts = {} },
```

Without a plugin manager:

```lua
vim.opt.runtimepath:prepend("/path/to/fipe/tools/editor/nvim")
require("fipe").setup({})
```

## Options

Every option is optional; the defaults are paths in this repository.

| Option | Default | What |
| --- | --- | --- |
| `lsp` | `tools/lsp/target/release/fipe-lsp` | the server; `false` leaves it out |
| `dap` | `tools/dap/target/release/fipe-dap` | the adapter; `false` leaves it out |
| `host` | `tests/fixtures/host` | the host declarations the server and the adapter check against |
| `parser` | `nil`: build from `tools/tree-sitter/src` | a path to a built `fipe.so`, or `false` to leave tree-sitter out |
| `trace` | the conformance trace `tests/fixtures/traces/<stem>.txt`, when one exists | the trace a launch replays: a path, or a function of the program's path |

The adapter only replays a trace (`tools/dap/README.md`): without one
every host call fails, and the run Faults at its first host call.

## Smoke

```sh
sh tools/editor/nvim/smoke.sh            # both configs
sh tools/editor/nvim/smoke.sh minimal    # or one: user, minimal
```

It builds the server and the adapter and runs `smoke.lua` in a
headless Neovim with every XDG directory under a scratch directory:

- `user`: a copy of `~/.config/nvim` with the spec above added, and
  links to `~/.local/share/nvim/{lazy,site}`, which lazy.nvim only
  reads while no plugin is missing (`FIPE_NVIM_CONFIG` and
  `FIPE_NVIM_DATA` name others); a config's dap-ui listeners are
  dropped, as headless Neovim has no screen for dap-ui;
- `minimal`: `minimal.lua`, the module with nvim-dap, nvim-treesitter
  and cmp-nvim-lsp from the lazy.nvim plugin root on the runtimepath.

It opens `Documentation/examples/05-modules.fipe`, waits for the
server, checks two highlight captures, the fold and indent queries,
an error diagnostic on a scratch file, and an nvim-dap session that
stops at a breakpoint on line 25 and continues to `terminated`. Each
step prints PASS or FAIL; the exit status is 0 only if all pass.

## Checklist by hand

In your own Neovim, with the spec installed, open
`Documentation/examples/05-modules.fipe`:

1. `:checkhealth vim.lsp` lists `fipe` attached to the buffer.
2. Hover (`K`) on `area` in `pub area` shows `area: F64 -> F64 -> F64`.
3. Definition on `Point` in `p: Point` opens `geometry.fipe` at the
   type.
4. Rename `name` in `main` (`vim.lsp.buf.rename`): both uses change.
5. On a new line, typing `z = fs.` offers `list`, `remove`, `size`
   and `Error` through nvim-cmp.
6. Keywords, comments, docs, constructors and pipes are coloured by
   the grammar (`:Inspect` on `use` shows `@keyword.import.fipe`);
   `zc` on the `with cwd /tmp {` block folds it.
7. Write `bad = find ./x | cat` on a new line: an error appears on it.
8. A breakpoint on line 25 (`e.name`), then `:lua
   require('dap').continue()`: dap-ui opens, stops on line 25 with
   Locals, Pipe, Effects and Budget scopes; continue to the end, and
   dap-ui closes on `terminated`.
