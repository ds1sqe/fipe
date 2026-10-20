#!/bin/sh
# Builds the parser into a scratch directory and runs smoke.lua in a
# Neovim that reads no user config or data: `--clean`, and every XDG
# directory under the scratch directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
package=$(cd "$here/../.." && pwd)
examples=$(cd "$package/../../Documentation/examples" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/runtime/queries" "$scratch/xdg"
ln -s "$package/queries" "$scratch/runtime/queries/fipe"
(cd "$package" && tree-sitter build -o "$scratch/fipe.so" . >/dev/null)
XDG_CONFIG_HOME="$scratch/xdg/config" XDG_DATA_HOME="$scratch/xdg/data" \
XDG_STATE_HOME="$scratch/xdg/state" XDG_CACHE_HOME="$scratch/xdg/cache" \
  nvim --headless --clean -l "$here/smoke.lua" \
    "$scratch/fipe.so" "$scratch/runtime" "$examples"
