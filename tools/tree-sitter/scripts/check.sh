#!/bin/sh
# Every check of the package, from the package directory or anywhere:
# the generated parser reproduced, the corpus and highlight tests, the
# corpus files parsed without an error node, the query copies current,
# the strict scanner build, the binding's tests, lints and docs, and
# the Neovim smoke when Neovim is installed. Offline throughout.
set -eu
package=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$package/../.." && pwd)
cd "$package"
status=0
step() {
    echo "=== $*"
    if ! sh -c "$*"; then
        echo "!!! FAILED: $*"
        status=1
    fi
}

step 'scratch=$(mktemp -d) && cp -r src "$scratch/src" &&
  tree-sitter generate && diff -r "$scratch/src" src; r=$?;
  rm -rf "$scratch"; exit $r'
step 'tree-sitter test'
step "tree-sitter parse -q --stat \
  '$repo'/Documentation/examples/*.fipe \
  '$repo'/Documentation/spec/02-syntax.fipe \
  '$repo'/prelude/prelude.fipe \
  '$repo'/tests/fixtures/opening.fipe \
  '$repo'/tests/fixtures/host/*.fipe"
step 'python3 scripts/queries.py --check'
step 'cc -std=c11 -Wall -Wextra -Werror -pedantic -Isrc -c src/scanner.c \
  -o /dev/null'
step 'cargo fmt --check'
step "FIPE_ROOT='$repo' cargo test --locked --offline"
step 'cargo clippy --locked --offline --all-targets -- -D warnings \
  -D clippy::all -D clippy::pedantic -D clippy::nursery'
step 'RUSTDOCFLAGS="-D warnings" cargo doc --locked --offline --no-deps'
if command -v nvim >/dev/null 2>&1; then
    step 'editors/nvim/smoke.sh'
else
    echo "=== editors/nvim/smoke.sh skipped: nvim not installed"
fi
echo "=== status $status"
exit $status
