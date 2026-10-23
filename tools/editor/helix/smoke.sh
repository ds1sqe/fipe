#!/bin/sh
# The Helix smoke: smoke.sh [scratch]. Offline.
#
# It builds the server, writes a Helix config under the scratch
# directory (the argument, or a temporary one) joining the language,
# the server and the grammar from this checkout by path, builds the
# grammar, checks `--health fipe`, and drives Helix in a detached tmux
# server of its own over the examples. Every XDG directory is under the
# scratch directory. HELIX names the binary (default: `hx`, or `helix`
# when there is no `hx`); WAIT is the seconds a step waits for the
# screen (default 30). It never fetches. Exit 0 only if every step
# passes.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
if [ -n "${HELIX:-}" ]; then hx=$HELIX
elif command -v hx >/dev/null 2>&1; then hx=hx
else hx=helix
fi
keep=$#
if [ $# -gt 0 ]; then
  scratch=$(mkdir -p "$1" && cd "$1" && pwd)
else
  scratch=$(mktemp -d)
fi
# A tmux server of its own: no other session is seen or touched.
tmux="tmux -L fipe-helix-$$ -f /dev/null"
cleanup() {
  $tmux kill-server 2>/dev/null || true
  [ "$keep" -gt 0 ] || rm -rf "$scratch"
}
trap cleanup EXIT

fails=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; fails=$((fails + 1)); }
check() { # check NAME CMD...: PASS when CMD succeeds
  name=$1; shift
  if "$@"; then pass "$name"; else fail "$name"; fi
}

(cd "$root/tools/lsp" && cargo build --locked --offline --release --quiet)

cfg=$scratch/config/helix
rm -rf "$scratch/config" "$scratch/cache" "$scratch/data" "$scratch/state"
# `--grammar build` writes here but creates the directory only on fetch.
mkdir -p "$cfg/runtime/grammars" "$cfg/runtime/queries" "$scratch/work"
cp -R "$root/tools/tree-sitter/editors/helix/queries/fipe" \
  "$cfg/runtime/queries/fipe"
{
  # Build only this grammar: the default builds every fetched one.
  echo 'use-grammars = { only = ["fipe"] }'
  echo
  # The stanza of the grammar package, with this checkout as the source.
  sed -e '/^#/d' -e '/^\[\[grammar\]\]/,$d' \
    "$root/tools/tree-sitter/editors/helix/languages.toml" |
    sed -e '/^roots = /a\
language-servers = ["fipe-lsp"]'
  cat <<EOF
[language-server.fipe-lsp]
command = "$root/target/release/fipe-lsp"
args = ["--host", "$root/tests/fixtures/host"]

[[grammar]]
name = "fipe"
source = { path = "$root/tools/tree-sitter" }
EOF
} >"$cfg/languages.toml"
export XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache" \
  XDG_DATA_HOME="$scratch/data" XDG_STATE_HOME="$scratch/state"

echo "== $($hx --version)"
(cd "$scratch/work" && $hx --grammar build) >"$scratch/build.txt" 2>&1 || true
check "grammar build" test -f "$cfg/runtime/grammars/fipe.so"

esc=$(printf '\033')
$hx --health fipe 2>&1 | sed "s/$esc\[[0-9;]*m//g" >"$scratch/health.txt"
health() { grep -q "$1" "$scratch/health.txt"; }
check "health: server found" health "✓ fipe-lsp: $root/tools/lsp"
check "health: parser" health "Tree-sitter parser: ✓"
check "health: highlight queries" health "Highlight queries: ✓"
check "health: indent queries" health "Indent queries: ✓"
# The package has no textobjects query (11-treesitter.md lists five).
echo "INFO $(grep 'Textobject queries' "$scratch/health.txt")"

# A screen of 50 rows shows each example whole, so row N is line N.
$tmux new-session -d -s smoke -x 160 -y 50 -c "$root" \
  "TERM=xterm-256color $hx Documentation/examples/05-modules.fipe"
keys() { $tmux send-keys -t smoke "$@"; sleep 0.3; }
screen() { $tmux capture-pane -p -t smoke "$@"; }
poll() { # poll CMD...: CMD succeeds within WAIT seconds (default 30)
  n=$((${WAIT:-30} * 5))
  until "$@"; do
    n=$((n - 1)); [ "$n" -gt 0 ] || return 1
    sleep 0.2
  done
}
shows() { screen | grep -q -- "$1"; }
wait_for() { poll shows "$1"; }
fg() { # fg ROW TOKEN: the foreground colour of TOKEN's last use on ROW
  screen -e | sed -n "${1}p" | awk -v tok="$2" '
    { n = split($0, part, "\033\\[") }
    END {
      for (i = 2; i <= n; i++) {
        match(part[i], /^[0-9;]*m/)
        code = substr(part[i], 1, RLENGTH - 1)
        text = substr(part[i], RLENGTH + 1)
        k = split(code, f, ";")
        for (j = 1; j <= k; j++) {
          w = (f[j + 1] == 2) ? 4 : 2 # 38;2;r;g;b or 38;5;n
          if (f[j] == 38) {
            cur = f[j + 1] == 2 ? f[j + 2] ";" f[j + 3] ";" f[j + 4] \
              : f[j + 2]
            j += w
          } else if (f[j] == 48 || f[j] == 58) j += w
          else if (f[j] == 39 || f[j] == 0 || f[j] == "") cur = ""
        }
        if (text == tok || index(text, tok " ") == 1) found = cur
      }
      print found
    }'
}
differ() { # differ ROW TOKEN ROW TOKEN: both coloured, differently
  a=$(fg "$1" "$2") b=$(fg "$3" "$4")
  echo "  $2: $a, $4: $b"
  [ -n "$a" ] && [ -n "$b" ] && [ "$a" != "$b" ]
}
no_error_node() { # no_error_node NAME: the file's subtree, page by page
  tree=$scratch/tree-$1.txt
  keys '%'
  keys ':tree-sitter-subtree' Enter
  : >"$tree"
  last='' i=0
  while [ $i -lt 200 ]; do
    page=$(screen)
    # Unchanged after a scroll: the end, or a slow redraw; look again.
    if [ "$page" = "$last" ]; then
      sleep 1
      [ "$(screen)" = "$last" ] && break
      page=$(screen)
    fi
    printf '%s\n' "$page" >>"$tree"
    last=$page i=$((i + 1))
    keys C-d
  done
  keys Escape
  keys ';' # collapse the selection, which would colour the text
  grep -q '(file' "$tree" && ! grep -q 'ERROR\|MISSING' "$tree"
}
# The status line, not the prompt that typed the name.
open() { keys ":o $1" Enter; wait_for "NOR .*$(basename "$1")"; }
scope() { # scope REGEX NAME: the highlight at the end of REGEX's match
  keys "/$1" Enter
  keys ':tree-sitter-highlight-name' Enter
  # Helix names the theme's scope for the capture: NAME or a prefix of
  # it (the default theme has `keyword`, not `keyword.control.import`).
  pat=$(printf '%s' "$2" | awk -F. '{
    p = $1; out = p
    for (i = 2; i <= NF; i++) { p = p "[.]" $i; out = out "|" p }
    print "(" out ")" }')
  poll eval 'screen | grep -Eq "(^| )$pat( |\$|▐)"'
  ok=$?
  keys Escape
  return $ok
}

check "05-modules: opens" wait_for '05-modules.fipe'
check "05-modules: no error node" no_error_node 05-modules
check "05-modules: keyword and name coloured apart" differ 4 use 4 fs
check "05-modules: use is keyword.control.import" \
  scope '^use' keyword.control.import
check "05-modules: Point after || is a constructor" \
  scope '\| Point' constructor
keys '/pub area' Enter
keys Space k
# The source line and the hover both read the scheme.
hover() { [ "$(screen | grep -c 'area: F64 -> F64 -> F64')" -ge 2 ]; }
check "05-modules: hover on area" poll hover
keys Escape
keys '/: Point' Enter
keys g d
check "05-modules: definition of Point opens geometry.fipe" \
  wait_for 'geometry.fipe'

check "04-text: opens" open Documentation/examples/04-text.fipe
check "04-text: no error node" no_error_node 04-text
check "04-text: |: is an operator" scope '\|:' operator
check "04-text: -by is a variable.parameter" scope -by variable.parameter
check "04-text: cat, applied, is a function" scope '^cat' function
check "04-text: lines, not applied, is a variable" scope '\| lines' variable

# The line after an opener is indented one unit.
printf 'f = x => {\n}\n' >"$scratch/work/indent.fipe"
open "$scratch/work/indent.fipe" || true
keys o
keys y Escape
keys ':w' Enter
indented() { [ "$(sed -n 2p "$scratch/work/indent.fipe")" = '    y' ]; }
check "indent.fipe: the line after { is indented" poll indented

# A syntax error, to show the error-node check sees one.
printf 'add 2 - 1\n' >"$scratch/work/syntax.fipe"
open "$scratch/work/syntax.fipe" || true
if no_error_node syntax; then fail "syntax.fipe: error node found"
else pass "syntax.fipe: error node found"
fi

printf 'bad = find ./x | cat\n' >"$scratch/work/bad.fipe"
open "$scratch/work/bad.fipe" || true
check "bad.fipe: the server reports an error" wait_for '●'
screen >"$scratch/bad.txt"

keys ':qa!' Enter
poll eval '! $tmux has-session 2>/dev/null' || true
if [ "$fails" -eq 0 ]; then echo "helix smoke: all passed"
else echo "helix smoke: $fails failed"; exit 1
fi
