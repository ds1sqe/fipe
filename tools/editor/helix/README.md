# Fipe for Helix

Helix needs three pieces, each kept with its package:

- the language and the grammar: `tools/tree-sitter/editors/helix/`
  (`languages.toml` and `queries/fipe/`, in Helix capture names);
- the language server: `tools/lsp/editor/languages.toml`.

The binary is `hx` upstream; some distributions install it as
`helix`. Helix 25.07 is the tested version; the queries rely on its
rule that a later highlight pattern wins.

## Install by hand

Build the server:

```sh
(cd tools/lsp && cargo build --release)
```

In `~/.config/helix/languages.toml`, take the `[[language]]` table
of the grammar stanza, add `language-servers = ["fipe-lsp"]` to it,
and add the `[language-server.fipe-lsp]` table of the server stanza,
with your paths. For the grammar, either keep the stanza's git
source (set `rev`, then `hx --grammar fetch`), or build from a
checkout with no network:

```toml
[[grammar]]
name = "fipe"
source = { path = "/path/to/fipe/tools/tree-sitter" }
```

Then (fetch makes the grammars directory; a path source does not):

```sh
mkdir -p ~/.config/helix/runtime/grammars
hx --grammar build
cp -R tools/tree-sitter/editors/helix/queries/fipe \
  ~/.config/helix/runtime/queries/fipe
hx --health fipe
```

`--grammar build` builds every configured grammar, and `--grammar
fetch` clones every one from the network; a top-level `use-grammars =
{ only = ["fipe"] }` limits both to this one. `--health fipe` lists
the server's path, the parser, and highlight and indent queries; the
package has no textobject query, so that line reads `✘`.

## Smoke

```sh
sh tools/editor/helix/smoke.sh [scratch]
```

It needs `tmux` and a C compiler, and no network (it never fetches).
It builds the server, writes the config above under the scratch
directory (the argument, or a temporary one) with this checkout's
paths, points every XDG directory there, builds the grammar and
checks `--health fipe`. It then runs Helix in a tmux server of its
own (`tmux -L`, no config) and checks:

- `Documentation/examples/05-modules.fipe` and `04-text.fipe` have no
  error node in `:tree-sitter-subtree` of the whole file, read page by
  page; a scratch `add 2 - 1` has one, so the check can fail;
- `use` and `fs` in different foreground colours, from `tmux
  capture-pane -e`;
- `:tree-sitter-highlight-name` on `use` (keyword), the constructor
  `Point`, `|:` (operator), the flag `-by` (`variable.parameter`),
  the applied `cat` (function) and the bare `lines` (variable); Helix
  names the theme's scope, which may be a prefix of the capture;
- hover (`space k`) on `area` shows `area: F64 -> F64 -> F64`;
- definition (`g d`) on `Point` in `p: Point` opens `geometry.fipe`;
- `o` after `f = x => {` indents the new line one unit;
- a scratch `bad = find ./x | cat` gets an error mark from the server.

Each step prints PASS or FAIL; the exit status is 0 only if all
pass. `HELIX` names the binary; `WAIT` is the seconds a step waits
for the screen (default 30), for a loaded machine.

## Checklist by hand

In your own Helix, with the config above, open
`Documentation/examples/05-modules.fipe`: comments, `##` docs and
strings are coloured apart from names, and `space k` on `area` and
`g d` on `Point` behave as in the smoke. The default theme gives a
flag (`variable.parameter`, `-by`) the colour of a name.
