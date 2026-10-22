#!/bin/sh
# The depgraph and unsafe probes of the backend's invariants, and the
# runtime's native libraries against `link::NATIVE_LIBS`. Prints one
# line per probe; exits 1 if any fails.
# Usage: tools/native/scripts/probes.sh  (from anywhere)
root=$(cd "$(dirname "$0")/../../.." && pwd)
fail=0
probe() { # probe LABEL: reads the probe's verdict from its exit status
    if [ "$2" -eq 0 ]; then echo "ok   $1"
    else echo "FAIL $1"; fail=$((fail + 1)); fi
}
cd "$root" || exit 1

# The crate depends on nothing; each package only on path packages.
[ "$(cargo tree --offline --prefix none | wc -l)" -eq 1 ]
probe "crate: cargo tree lists the crate alone" $?
for package in tools/native tools/native/runtime; do
    ! (cd "$package" && cargo tree --offline --prefix none) \
        | grep -v ' (/' | grep -q .
    probe "$package: cargo tree lists only path packages" $?
done

# The crate names no backend and stays no_std.
! rg -n -t rust -i -w 'native backend|x86|elf' src/
probe "crate: src/ names no backend" $?
cargo check --locked --offline --quiet --lib --no-default-features \
    --target x86_64-unknown-none
probe "crate: no_std check" $?

# Unsafe only in the runtime's abi.rs, besides the two root lint lines
# (Rust sources only: this script names the words too).
grep -q '^unsafe_code = "forbid"' Cargo.toml
probe "crate: forbids unsafe" $?
! rg -n -t rust 'unsafe|no_mangle|extern "C"' tools/native \
    | grep -v '^tools/native/runtime/src/abi.rs:' \
    | grep -v -x 'tools/native/src/lib.rs:[0-9]*:#!\[forbid(unsafe_code)\]' \
    | grep -v -x \
        'tools/native/runtime/src/lib.rs:[0-9]*:#!\[deny(unsafe_code)\]' \
    | grep .
probe "tools/native: unsafe only in runtime/src/abi.rs" $?

# rustc's native-static-libs for the runtime equal link::NATIVE_LIBS.
target=tools/native/target/probe-libs
(cd tools/native/runtime && cargo clean --quiet --offline \
    --target-dir "$root/$target" -p fipe-native-rt)
printed=$(cd tools/native/runtime && cargo rustc --locked --offline \
    --quiet --target-dir "$root/$target" --lib --crate-type staticlib \
    -- --print native-static-libs 2>&1 \
    | sed -n 's/.*native-static-libs: //p')
listed=$(sed -n '/NATIVE_LIBS/,/;/p' tools/native/src/link.rs \
    | grep -o '"-l[^"]*"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')
echo "     rustc: $printed"
[ -n "$printed" ] && [ "$printed" = "$listed" ]
probe "runtime: native-static-libs equal link::NATIVE_LIBS" $?

if [ "$fail" -eq 0 ]; then echo "probes: all passed"
else echo "probes: $fail failed"; exit 1; fi
