#!/bin/sh
# Builds the server and runs smoke.lua in a Neovim that reads no user
# config or data: init.lua only, and every XDG directory under a
# scratch directory (the first argument, or a temporary one).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
package=$(cd "$here/.." && pwd)
root=$(cd "$package/../.." && pwd)
if [ $# -gt 0 ]; then
  scratch=$1
  mkdir -p "$scratch"
else
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
fi
(cd "$package" && cargo build --locked --offline --release --quiet)
cd "$root"
XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/data" \
XDG_STATE_HOME="$scratch/state" XDG_CACHE_HOME="$scratch/cache" \
  nvim --headless -u tools/lsp/editor/init.lua -l tools/lsp/editor/smoke.lua
