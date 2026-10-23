#!/bin/sh
# Build the adapter and record a whole debugging session of
# 02-outcomes.fipe over its real stdio, one JSON message per line.
set -e
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
(cd "$here" && cargo build --locked --offline -q)
exec python3 -I "$here/scripts/record.py" "$root/target/debug/fipe-dap" "$root"
