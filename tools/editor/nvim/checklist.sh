#!/bin/sh
# The README's checklist by hand, scripted: checklist.sh [scratch].
#
# It runs Neovim with a copy of your config (as smoke.sh's `user`
# mode does) in a terminal: a detached tmux server of its own, 220x60,
# whose socket is in the scratch directory. It types each item's keys
# there like a user, reads the screen with `tmux capture-pane`, and
# reads state through `nvim --remote-expr` with the probes of
# checklist.lua. Each item prints PASS or FAIL; with a scratch
# directory named, the screens stay in <scratch>/checklist/screens.
# Exit 0 only if all eight pass. Needs tmux; runs under `unshare -rn`
# when that works, so the config's plugins cannot reach the network.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
config=${FIPE_NVIM_CONFIG:-$HOME/.config/nvim}
data=${FIPE_NVIM_DATA:-$HOME/.local/share/nvim}
timeout=${FIPE_CHECKLIST_TIMEOUT:-60}
tmp=
if [ $# -gt 0 ]; then
  scratch=$1
  mkdir -p "$scratch"
else
  scratch=$(mktemp -d)
  tmp=$scratch
fi

(cd "$root/tools/lsp" && cargo build --locked --offline --release --quiet)
(cd "$root/tools/dap" && cargo build --locked --offline --release --quiet)

x=$scratch/checklist
rm -rf "$x"
mkdir -p "$x/config" "$x/data/nvim" "$x/state" "$x/cache" "$x/work" \
  "$x/screens"
cp -a "$config" "$x/config/nvim"
# lazy.nvim's update checker would fetch from the network.
cat >"$x/config/nvim/lua/plugins/fipe.lua" <<EOF
local options = require("lazy.core.config").options
options.checker.enabled = false
options.change_detection.enabled = false
return {
  { dir = "$here", name = "fipe", main = "fipe", lazy = false, opts = {} },
}
EOF
ln -s "$data/lazy" "$data/site" "$x/data/nvim/"
# Edits go to copies; the trace is found by the stem.
cp "$root/Documentation/examples/05-modules.fipe" \
  "$root/Documentation/examples/geometry.fipe" "$x/work/"

sock=$x/nvim.sock
helper=$here/checklist.lua
t() { tmux -S "$x/tmux.sock" -f /dev/null "$@"; }
trap 't kill-server 2>/dev/null || :; [ -z "$tmp" ] || rm -rf "$tmp"' EXIT
isolate=
if unshare -rn true 2>/dev/null; then
  isolate="unshare -rn"
fi
t new-session -d -x 220 -y 60 -c "$x/work" \
  "$isolate env XDG_CONFIG_HOME=$x/config XDG_DATA_HOME=$x/data \
  XDG_STATE_HOME=$x/state XDG_CACHE_HOME=$x/cache TERM=xterm-256color \
  nvim --listen $sock 05-modules.fipe; sleep 600"

# q 'probe(args)': a probe's result in the running Neovim.
q() {
  nvim --server "$sock" --remote-expr \
    "luaeval(\"dofile('$helper').$1\")" 2>/dev/null || :
}
# until_q 'probe(args)' pattern: wait until the result matches.
until_q() {
  waited=0
  while :; do
    # shellcheck disable=SC2254
    case $(q "$1") in $2) return 0 ;; esac
    waited=$((waited + 1))
    [ $waited -ge $((timeout * 5)) ] && return 1
    sleep 0.2
  done
}
# until_screen pattern: wait until a screen line matches (grep -E).
until_screen() {
  waited=0
  until t capture-pane -p | grep -Eq "$1"; do
    waited=$((waited + 1))
    [ $waited -ge $((timeout * 5)) ] && return 1
    sleep 0.2
  done
}
keys() { t send-keys "$@"; }
# typed text: one key at a time, as a user types.
typed() {
  printf '%s\n' "$1" | fold -w1 | while IFS= read -r c; do
    t send-keys -l "${c:- }"
    sleep 0.1
  done
}
shot() { t capture-pane -p >"$x/screens/$1.txt"; }
# A normal-mode cursor move, then a wait until it lands.
goto() {
  keys Escape ":$1" Enter
  until_q 'where()' "05-modules.fipe:$1:*"
}

failures=0
item() {
  if [ "$2" = ok ]; then
    echo "PASS $1"
  else
    echo "FAIL $1: $2"
    failures=$((failures + 1))
  fi
}

until_q 'attached()' 1 || {
  echo "FAIL the fipe server never attached"
  exit 1
}

# 1. :checkhealth vim.lsp lists fipe attached.
r=ok
keys ":checkhealth vim.lsp" Enter
until_screen '^ *[0-9]+ +- fipe \(id: [0-9]+\)' || r="no fipe client"
shot 1-checkhealth
awk '/- fipe \(id:/ { f = 1 } f && /Attached buffers: [1-9]/ { ok = 1 }
  f && /^ *[0-9]+ *$/ { exit } END { exit !ok }' \
  "$x/screens/1-checkhealth.txt" || r="fipe has no attached buffer"
keys ":bwipeout" Enter
until_q 'where()' '05-modules.fipe:*' || r="the health buffer stays"
item "checkhealth lists fipe attached" "$r"

# 2. K on `area` shows its type.
r=ok
goto 10
keys 0w K
until_q 'floats()' '*area: F64 -> F64 -> F64*' || r="no hover"
# The source line has the text too: the hover is a second line.
waited=0
until [ "$(t capture-pane -p | grep -c 'area: F64 -> F64 -> F64')" -ge 2 ]; do
  sleep 0.2
  waited=$((waited + 1))
  [ $waited -ge $((timeout * 5)) ] && r="the hover is not on the screen" && break
done
shot 2-hover
keys j k
item "K on area shows area: F64 -> F64 -> F64" "$r"

# 3. gd (the config's definition key) on `Point` in `p: Point`.
r=ok
goto 14
keys ^ f: w
until_q 'where()' '05-modules.fipe:14:Point' || r="the cursor is not on Point"
keys g d
until_q 'where()' 'geometry.fipe:*' || r="geometry.fipe does not open"
w=$(q 'where()')
[ "$w" = geometry.fipe:3:Point ] || r="lands at $w"
shot 3-definition
grep -q 'pub type Point' "$x/screens/3-definition.txt" ||
  r="the type is not on the screen"
keys C-o
until_q 'where()' '05-modules.fipe:*' || r="C-o does not come back"
item "definition on Point opens geometry.fipe at the type ($w)" "$r"

# 4. Rename `name` in main with the config's rename key.
r=ok
goto 13
keys ^
until_q 'where()' '05-modules.fipe:13:name' || r="the cursor is not on name"
keys Space c r
sleep 1
keys C-w
typed label
keys Enter
until_q 'lines(13, 14)' '*label = first*read label*' || r="not renamed"
until_screen 'label = first args' || r="the screen lacks the binding"
until_screen 'read label \|\|' || r="the screen lacks the use"
shot 4-rename
keys Escape ":e!" Enter
until_q 'lines(13, 13)' '*name = first*' || r="the buffer does not reload"
item "rename name in main changes both uses" "$r"

# 5. Typing `z = fs.` on a new line opens nvim-cmp with fs's members.
r=ok
until_q 'attached()' 1 || r="the server detached"
keys G o
until_q 'mode()' i || r="no insert mode"
typed "z = fs."
until_q 'cmp()' 'Error,list,remove,size' || r="cmp shows $(q 'cmp()')"
for word in list remove size Error; do
  until_screen "[^a-z]$word +fipe" || r="the menu lacks $word"
done
shot 5-completion
keys C-e Escape u
item "z = fs. offers list, remove, size, Error through nvim-cmp" "$r"

# 6. Grammar colours, :Inspect on `use`, zc on the `with cwd` block.
r=ok
goto 1
for at in "4 1 use" "1 1 comment" "9 1 doc" "16 5 Ok" "15 7 pipe" \
  "10 1 pub" "36 14 if"; do
  set -- $at
  # A sign column coming or going moves the text: dump and ask again.
  n=0
  while :; do
    t capture-pane -p -e >"$x/screens/6-colours.ansi"
    c=$(q "colour('$x/screens/6-colours.ansi', $1, $2)")
    n=$((n + 1))
    case $c in ok*) break ;; esac
    [ $n -ge 5 ] && break
    sleep 1
  done
  echo "     $3 at $1:$2: $c"
  case $c in ok*) ;; *) r="$3 at $1:$2: $c" ;; esac
done
goto 4
keys 0 ":Inspect" Enter
until_screen '@keyword\.import\.fipe' || r=":Inspect lacks @keyword.import.fipe"
shot 6-inspect
keys Escape
goto 29
keys z c
until_q 'foldclosed(29)' 29 || r="zc does not fold line 29"
shot 6-fold
grep -q 'map \.name' "$x/screens/6-fold.txt" && r="the block is still drawn"
keys z o
item "grammar colours, :Inspect on use, zc folds with cwd" "$r"

# 7. An error on a new line `bad = find ./x | cat`.
r=ok
keys Escape G o
until_q 'mode()' i || r="no insert mode"
typed "bad = find ./x | cat"
keys Escape
until_q 'error(37)' '?*' || r="no error on line 37"
# The screen draws the message after the line (the config's virtual
# text); a message of a half-typed line may come first.
waited=0
until m=$(q 'error(37)') && [ -n "$m" ] &&
  t capture-pane -p | grep 'bad = find ./x | cat ' | grep -qF -- "$m"; do
  sleep 0.2
  waited=$((waited + 1))
  [ $waited -ge $((timeout * 5)) ] && r="the screen shows no error" && break
done
shot 7-error
echo "     $(q 'error(37)')"
keys u
item "bad = find ./x | cat shows an error" "$r"

# 8. A breakpoint on line 25 (the config's key), then `:lua
# require('dap').continue()`: dap-ui opens at the stop with Locals,
# Globals, Effects and Budget. Pipe is a scope only at a stop on a pipe
# stage: `next` (the config's key) goes on to line 30's `map .name`.
# Continuing ends the run, and dap-ui closes.
r=ok
goto 25
keys Space d b
until_q 'breakpoints()' 1 || r="no breakpoint"
q 'stops()' >/dev/null
keys ":lua require('dap').continue()" Enter
until_q 'dap()' stopped:25 || r="no stop on line 25 ($(q 'dap()'))"
until_q 'dapui()' '*dapui_scopes*' || r="dap-ui does not open"
until_q 'scopes()' 'Locals,Globals,Effects,Budget' ||
  r="the scopes at 25 are $(q 'scopes()')"
for scope in Locals Globals Effects; do
  until_screen "^ *[^ ]* *$scope:" || r="dap-ui shows no $scope"
done
shot 8-dapui
echo "     at $(q 'dap()'): $(q 'scopes()'); windows $(q 'dapui()')"
steps=1
until q 'scopes()' | grep -q Pipe; do
  steps=$((steps + 1))
  [ $steps -gt 8 ] && break
  keys Space d o
  until_q 'stops()' $steps || :
done
until_screen '^ *[^ ]* *Pipe:' || r="dap-ui shows no Pipe scope"
until_screen 'mark = \|' || r="the Pipe scope shows no mark"
shot 8-pipe
echo "     at $(q 'dap()'): $(q 'scopes()')"
keys ":lua require('dap').continue()" Enter
until_q 'dap()' none || r="the run does not end"
until_q 'dapui()' '' || r="dap-ui stays open"
shot 8-terminated
item "dap-ui opens at 25 with its scopes, Pipe on a stage, closes" "$r"

keys Escape ":qa!" Enter
# Neovim writes its shada as it exits: wait before the cleanup.
until_q 'mode()' '' || :
if [ $failures -eq 0 ]; then
  echo "checklist: all 8 items pass"
else
  echo "checklist: $failures failed"
fi
exit $((failures != 0))
