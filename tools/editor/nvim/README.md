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
- the debug adapter `tools/dap` as `dap.adapters.fipe` and two launch
  configurations `dap.configurations.fipe` (a trace replayed, or live
  on the std host), when nvim-dap is installed.
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
| `lsp` | `target/release/fipe-lsp` | the server; `false` leaves it out |
| `dap` | `target/release/fipe-dap` | the adapter; `false` leaves it out |
| `host` | `tests/fixtures/host` | the host declarations the server and the adapter check against |
| `parser` | `nil`: build from `tools/tree-sitter/src` | a path to a built `fipe.so`, or `false` to leave tree-sitter out |
| `trace` | the conformance trace `tests/fixtures/traces/<stem>.txt`, when one exists | the trace a launch replays: a path, or a function of the program's path |

The first configuration replays the trace (`tools/dap/README.md`);
the second, "Run this file on this machine", runs live on the std host
with the workspace folder as its working directory and write root.

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
an error diagnostic on a scratch file, signature help at `fs.list ` and
an inlay hint on a scratch binding, an nvim-dap session that
stops at a breakpoint on line 25 and continues to `terminated`, and a
live session on a scratch script that stops at a breakpoint and
writes a file in the scratch directory. Each step prints PASS or FAIL;
the exit status is 0 only if all pass.

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
   Locals, Globals, Effects and Budget scopes; `next` on to the
   `map .name` stage of line 30 adds Pipe (a scope only at a stop on a
   pipe stage); continue to the end, and dap-ui closes on `terminated`.

### Scripted

```sh
sh tools/editor/nvim/checklist.sh [scratch]
```

runs the eight items in your Neovim config: a copy, as the smoke's
`user` mode makes it, with lazy.nvim's update checker off, under
`unshare -rn` (no network) when that works. Neovim runs in a 220x60
terminal of a tmux server of its own; the script types each item's
keys there (`K`, your config's `gd`, `<leader>cr`, `<leader>db` and
`<leader>do`, insert-mode typing for nvim-cmp, `zc`, `:Inspect`) on
copies of the two example files, reads the screen with
`tmux capture-pane`, and reads state with the probes of
`checklist.lua` over `nvim --remote-expr`. Each item prints PASS or
FAIL, the exit status is 0 only if all pass, and with a scratch
directory named the screens stay in `<scratch>/checklist/screens`. It
asserts:

1. the health report lists `- fipe (id: N)` with `Attached buffers:`
   1 or more;
2. a floating window holds `area: F64 -> F64 -> F64` and the screen
   shows it;
3. the cursor lands in `geometry.fipe` on line 3, on `Point`, shown on
   the screen;
4. lines 13 and 14 read `label` after the rename, on the screen too;
5. nvim-cmp's open menu holds exactly `Error`, `list`, `remove` and
   `size` from the server, each drawn with the menu's `fipe` tag;
6. `use`, a comment, a doc comment, `Ok`, a `|`, `pub` and `if` are
   each drawn in the 24-bit colour of their tree-sitter capture's
   highlight group; `:Inspect` on `use` shows `@keyword.import.fipe`;
   after `zc` line 29 is a closed fold and its body is not drawn;
7. an error diagnostic on the new line, its message drawn after it;
8. the stop on 25 with dap-ui's windows open and its scopes element
   listing Locals, Globals, Effects and Budget (the first three on the
   screen); `next` reaching a stop whose scopes add Pipe, drawn with
   its `mark`; after `continue`, no session and no dap-ui window.

What it cannot judge: whether the colours look right in your theme
(item 6 checks that each token gets its group's colour, not that the
colour scheme gives the groups good colours), and whether Budget,
below a long Effects list, is in view without scrolling dap-ui's
scopes window.
