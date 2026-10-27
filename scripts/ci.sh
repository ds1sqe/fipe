#!/usr/bin/env bash
# The same bounded checks run locally and in Actions; installation is not a mode.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=${1:?usage: scripts/ci.sh quality|linux|vm|features|package [debug|release]}
profile=${2:-debug}
case "$profile" in
    debug) profile_arg= ;;
    release) profile_arg=--release ;;
    *) echo "unknown profile: $profile" >&2; exit 2 ;;
esac
export RUSTUP_AUTO_INSTALL=0
export CARGO_TERM_COLOR=never
unset FIPE_BLESS

cargo_ci() { cargo +1.98.1 "$@"; }
identity() {
    rustc +1.98.1 -Vv
    cargo_ci -V
    if [ -n "${CI_EXPECTED_HOST:-}" ]; then
        host=$(rustc +1.98.1 -Vv | sed -n 's/^host: //p')
        if [ "$host" != "$CI_EXPECTED_HOST" ]; then
            echo "expected host $CI_EXPECTED_HOST, found $host" >&2
            exit 1
        fi
    fi
}
# Native test helpers invoke offline Cargo in separate target directories.
fetch() { cargo_ci fetch --locked; }
expect_rejection() {
    text=$1
    shift
    log=$(mktemp)
    if cargo_ci "$@" >"$log" 2>&1; then
        cat "$log"
        rm -f "$log"
        echo "expected rejection: $text" >&2
        exit 1
    fi
    cat "$log"
    if ! grep -F -- "$text" "$log" >/dev/null; then
        rm -f "$log"
        echo "missing intended rejection: $text" >&2
        exit 1
    fi
    rm -f "$log"
}

identity
case "$mode" in
    quality)
        fetch
        cargo_ci fmt --all --check
        cargo_ci clippy --workspace --locked --all-targets --offline -- \
            -D warnings -D clippy::all -D clippy::pedantic -D clippy::nursery
        RUSTDOCFLAGS='-D warnings' cargo_ci doc --workspace --locked --offline --no-deps
        ;;
    linux)
        if [ "$(rustc +1.98.1 -Vv | sed -n 's/^host: //p')" != x86_64-unknown-linux-gnu ]; then
            echo 'linux mode requires x86_64-unknown-linux-gnu' >&2
            exit 2
        fi
        "${FIPE_CLANG:-clang-23}" --version
        cc --version
        fetch
        cargo_ci fetch --manifest-path tools/ffi/tests/providers/Cargo.toml --locked
        cargo_ci test --workspace --locked --offline ${profile_arg:+"$profile_arg"}
        FIPE_BACKEND=llvm cargo_ci test -p fipe-native --test equivalence \
            --locked --offline ${profile_arg:+"$profile_arg"}
        cargo_ci test --manifest-path tools/ffi/tests/providers/Cargo.toml \
            --test c_rust --locked --offline ${profile_arg:+"$profile_arg"}
        # This rehearses source-built processes, not final installed archives.
        (
            test -z "${CARGO_BUILD_TARGET:-}"
            unset PYTHONOPTIMIZE
            timeout --kill-after=5s 600s cargo +1.98.1 build \
                -p fipe-cli -p fipe-repl -p fipe-dap \
                --locked --offline --jobs 8 ${profile_arg:+"$profile_arg"}
            process_bin=$(python3 -c \
                'import os,sys; print(os.path.abspath(sys.argv[1]) + "/" + sys.argv[2])' \
                "${CARGO_TARGET_DIR:-$PWD/target}" "$profile")
            work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/fipe-process-${profile}.XXXXXX")
            trap 'rm -rf -- "$work"' EXIT
            mkdir -p "$work/drivers" "$work/Documentation/examples" \
                "$work/tests/fixtures/host" "$work/tests/fixtures/traces"
            cp tools/repl/tests/pty.py tools/dap/tests/installed.py "$work/drivers/"
            cp Documentation/examples/02-outcomes.fipe "$work/Documentation/examples/"
            cp tests/fixtures/traces/02-outcomes.txt "$work/tests/fixtures/traces/"
            for module in commands cli config fs sys text; do
                cp "tests/fixtures/host/$module.fipe" "$work/tests/fixtures/host/"
            done
            printf 'Source-built process rehearsal: profile=%s source=%s cwd=%s\n' \
                "$profile" "$(git rev-parse HEAD)" "$work"
            for command in fipe fipe-repl fipe-dap; do
                test -f "$process_bin/$command" && test -x "$process_bin/$command"
            done
            sha256sum "$process_bin/fipe" "$process_bin/fipe-repl" \
                "$process_bin/fipe-dap" > "$work/binary-sha256.txt"
            cat "$work/binary-sha256.txt"
            timeout --kill-after=5s 10s "$process_bin/fipe" --version
            timeout --kill-after=5s 10s "$process_bin/fipe-dap" --version
            cd "$work"
            sha256sum drivers/*.py Documentation/examples/02-outcomes.fipe \
                tests/fixtures/host/*.fipe tests/fixtures/traces/02-outcomes.txt \
                > fixture-sha256.txt
            stat -c '%a %n' drivers/*.py Documentation/examples/02-outcomes.fipe \
                tests/fixtures/host/*.fipe tests/fixtures/traces/02-outcomes.txt \
                > fixture-modes.txt
            timeout --kill-after=5s 120s python3 drivers/pty.py "$process_bin/fipe-repl"
            timeout --kill-after=5s 120s python3 drivers/pty.py \
                --cli-interactive "$process_bin/fipe"
            timeout --kill-after=5s 60s python3 drivers/pty.py --cli "$process_bin/fipe"
            timeout --kill-after=5s 210s python3 drivers/installed.py \
                --binary "$process_bin/fipe-dap" --root "$work"
            sha256sum --check fixture-sha256.txt
            sha256sum --check binary-sha256.txt
            stat -c '%a %n' drivers/*.py Documentation/examples/02-outcomes.fipe \
                tests/fixtures/host/*.fipe tests/fixtures/traces/02-outcomes.txt \
                > fixture-modes-after.txt
            cmp fixture-modes.txt fixture-modes-after.txt
            printf 'PASS source-built process rehearsal: %s\n' "$profile"
        )
        ;;
    vm)
        fetch
        cargo_ci build -p fipe-cli -p fipe-lsp -p fipe-dap \
            --locked --offline ${profile_arg:+"$profile_arg"}
        cargo_ci test -p fipe -p fipe-repl-core -p fipe-lsp -p fipe-dap \
            --locked --offline ${profile_arg:+"$profile_arg"}
        # Keep Linux-native CLI cases filtered; Host suites run on every VM host.
        cargo_ci test -p fipe-host --test declarations --locked --offline ${profile_arg:+"$profile_arg"}
        cargo_ci test -p fipe-host --lib --test run --test write_roots \
            --locked --offline ${profile_arg:+"$profile_arg"}
        cargo_ci clippy -p fipe-host --all-targets --locked --offline \
            ${profile_arg:+"$profile_arg"} -- \
            -D warnings -D clippy::all -D clippy::pedantic -D clippy::nursery
        cargo_ci test -p fipe-cli --test tour --test dogfood \
            --locked --offline ${profile_arg:+"$profile_arg"}
        for test in the_repl_shows_values_and_performs_effects_through_the_std_host \
            check_exits_by_the_worst_file \
            native_build_rejects_unsupported_hosts_before_effects \
            repl_eof_drains_trailing_input_and_effects_without_quit; do
            cargo_ci test -p fipe-cli --test cli --locked --offline \
                ${profile_arg:+"$profile_arg"} "$test" -- --exact
        done
        cargo_ci run -p fipe --example embed --locked --offline ${profile_arg:+"$profile_arg"}
        cargo_ci run -p fipe --example custom_host --locked --offline ${profile_arg:+"$profile_arg"}
        ;;
    features)
        fetch
        cargo_ci build -p fipe-repl --no-default-features --features linux --locked --offline
        expect_rejection 'enable one platform feature: `std` or `linux`' \
            check -p fipe-repl --lib --no-default-features --locked --offline
        expect_rejection 'enable exactly one platform feature: `std` or `linux`' \
            check -p fipe-repl --lib --all-features --locked --offline
        expect_rejection 'the `linux` platform is x86_64 Linux only' \
            check -p fipe-repl --lib --no-default-features --features linux \
            --target x86_64-pc-windows-msvc --locked --offline
        consumer=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/fipe-no-std.XXXXXX")
        mkdir -p "$consumer/src"
        cat >"$consumer/Cargo.toml" <<TOML
[package]
name = "fipe-no-std-consumer"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[dependencies]
fipe = { path = "$PWD" }
fipe-repl-core = { path = "$PWD/tools/repl-core" }
TOML
        cat >"$consumer/src/lib.rs" <<'RS'
#![no_std]
extern crate alloc;
pub fn boundary() -> (fipe::decl::Declarations, fipe_repl_core::Repl) {
    (fipe::decl::Declarations::new(), fipe_repl_core::Repl::new())
}
RS
        cargo_ci generate-lockfile --manifest-path "$consumer/Cargo.toml" --offline \
            --config 'resolver.incompatible-rust-versions="fallback"'
        cargo_ci check --manifest-path "$consumer/Cargo.toml" --lib \
            --target thumbv7em-none-eabihf --locked --offline
        cargo_ci tree --manifest-path "$consumer/Cargo.toml" --edges normal --locked --offline
        cargo_ci metadata --manifest-path "$consumer/Cargo.toml" --offline \
            --format-version 1 >"$consumer/metadata.json"
        python3 - "$consumer/metadata.json" <<'PY'
import json, sys
packages = json.load(open(sys.argv[1]))['packages']
assert {p['name'] for p in packages} == {'fipe-no-std-consumer', 'fipe', 'fipe-repl-core'}
PY
        metadata=$(mktemp)
        cargo_ci metadata --no-deps --locked --offline --format-version 1 >"$metadata"
        python3 - "$metadata" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
packages = [p for p in m['packages'] if p['id'] in m['workspace_members']]
assert len(packages) == 13
assert all(p['rust_version'] == '1.98.1' for p in packages)
PY
        rm -f "$metadata"
        ;;
    package)
        fetch
        cargo_ci package --workspace --locked
        ;;
    *) echo "unknown mode: $mode" >&2; exit 2 ;;
esac
