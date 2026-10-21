#!/bin/sh
# The module's headless smoke: smoke.sh [user|minimal] [scratch].
#
# - user: a copy of your Neovim config (~/.config/nvim) with the
#   README's lazy.nvim spec added, and links to its plugins and
#   parsers (~/.local/share/nvim/{lazy,site}), which lazy.nvim only
#   reads while nothing is missing and no update runs.
# - minimal: minimal.lua, with nvim-dap, nvim-treesitter and
#   cmp-nvim-lsp from ~/.local/share/nvim/lazy on the runtimepath.
#
# With no mode both run. Every XDG directory is under the scratch
# directory (the second argument, or a temporary one). It builds the
# server and the adapter first. Exit 0 only if every step passes.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
config=${FIPE_NVIM_CONFIG:-$HOME/.config/nvim}
data=${FIPE_NVIM_DATA:-$HOME/.local/share/nvim}
modes=${1:-user minimal}
if [ $# -gt 1 ]; then
  scratch=$2
  mkdir -p "$scratch"
else
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
fi

(cd "$root/tools/lsp" && cargo build --locked --offline --release --quiet)
(cd "$root/tools/dap" && cargo build --locked --offline --release --quiet)

status=0
for mode in $modes; do
  echo "== $mode config"
  x=$scratch/$mode
  rm -rf "$x"
  mkdir -p "$x/config" "$x/data/nvim" "$x/state" "$x/cache" "$x/work"
  # The positional parameters carry the -u argument, quoted.
  case $mode in
    user)
      cp -a "$config" "$x/config/nvim"
      cat >"$x/config/nvim/lua/plugins/fipe.lua" <<EOF
return {
  { dir = "$here", name = "fipe", main = "fipe", lazy = false, opts = {} },
}
EOF
      ln -s "$data/lazy" "$data/site" "$x/data/nvim/"
      set -- -u "$x/config/nvim/init.lua"
      ;;
    minimal)
      set -- -u "$here/minimal.lua"
      ;;
    *)
      echo "unknown mode $mode" >&2
      exit 2
      ;;
  esac
  if ! (cd "$root" && XDG_CONFIG_HOME="$x/config" XDG_DATA_HOME="$x/data" \
    XDG_STATE_HOME="$x/state" XDG_CACHE_HOME="$x/cache" \
    FIPE_PLUGINS="$data/lazy" \
    nvim --headless "$@" -l "$here/smoke.lua" "$root" "$x/work"); then
    status=1
  fi
done
exit $status
