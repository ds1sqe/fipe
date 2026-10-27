# Publishing Fipe packages

## Published versions and selected cuts

Fipe's 13 workspace packages are distributed independently on crates.io.
`0.2.0-dev.2` is the last broad workspace/tooling baseline, not evidence that
all 13 packages were published: the current `fipe-ffi` API returns 404.
Only **`fipe` and `fipe-repl-core`** have published `0.2.0-dev.3` versions.
Their accepted immutable archives have SHA-256 checksums:

- `fipe`: `b4bde8862142888b0db5afff99e03270cfac7536807ca771d6f69e0ce6f7c2db`.
- `fipe-repl-core`: `652e6f30c1448ad569dbe1e2f00b1e7cb9355d3051ee03faf57f35dda5eab0bf`.

The other published tool packages remain at dev.2. Do not substitute a dev.2
Standard/native/runtime/tool package into a purported matched dev.3 release.
The earlier Core `0.2.0-dev.1` remains historical. No stable `0.2.0` or `0.2.1`
release is established by these facts.

The held-grant Standard cut at `a784bc36` has private dev.3 archive and extracted
consumer rehearsals. Those different archive bytes cannot replace published
dev.3. The selected next stabilization release is **unpublished `0.2.1`**;
its final source, compatibility decisions, versions and package set still need
approval. Source versioning, a private rehearsal and successful CI do not publish.

## Qualify the exact source and package set

Use a clean committed version cut, the [workspace checks](development.md), both
embedding examples, and actual supported-platform results. Retain the full
source byte/mode map, Git revision/tree and package inventory. For a full
workspace archive qualification, run:

```sh
cargo +1.98.1 package --workspace --locked
```

A separately selected partial rehearsal can package only its reviewed packages;
it does not close the remaining packages. Keep every selected archive's bytes,
checksum, contents, original and normalized manifests, shipped lock and genuine
`.cargo_vcs_info.json`. Cargo's normal verification builds extracted sources.
Do not bypass it with `--no-verify`, or use `--allow-dirty` as release evidence.
Diagnostic dirty inspection, if used, is not qualification.

Check README/licenses, Host declarations and notices, prelude/library assets,
examples and tree-sitter sources against include lists and the selected source.
Tool integration tests may use checkout fixtures and are not necessarily shipped;
run them separately. No checkout-relative include or copied-in missing asset may
rescue a package. A hosted member listing is not a retained archive checksum ledger.

Rehearse an outside-checkout locked public consumer using only verified extracted
packages. Explicit temporary extracted path/patch resolution is private rehearsal,
not registry proof. After publication, repeat without path/Git/patch overrides
and assert actual registry implementation paths and accepted archive checksums.
For Standard, exercise public `load`/`Policy`/`Host::run` allowed writes, appends,
removals, grant refusal and an unchanged outside sentinel.

## Version and dependency pairing

All workspace packages inherit a version and exact internal requirements. Review
versions and root lock coherently even when selecting only two packages. A changed
published package needs a new version; crates.io versions cannot be overwritten.
The full dependency order is:

1. `fipe`.
2. `fipe-host`, `fipe-native-core`, `fipe-repl-core`, `fipe-lsp`, `tree-sitter-fipe`.
3. `fipe-llvm`, `fipe-repl`, `fipe-dap`, `fipe-native-rt`.
4. `fipe-native`, then `fipe-ffi` and `fipe-cli`.

Full rehearsal also includes `fipe-ffi`'s native/LLVM development edges and
`fipe-native`'s LLVM development edge. Preserve external checksums. Review the
standalone provider proof lock, linked-provider example lock and both `.in`
manifests under `tools/ffi`, plus the custom-host fixture's exact Fipe pin and
lock under `tests/fixtures/custom-host`. Validate each independent graph with
`--locked`; root workspace lock success does not validate those locks.

## Floor, platforms and independent consumers

Use [compatibility and support](development.md#compatibility-and-support).
Rust 1.98.1 is the selected floor. VM/tooling require actual Linux, Windows and
both macOS architectures; installed terminal/editor behavior is separate from
hosted tests. Native/LLVM executable support is x86-64 Linux only. Preserve the
explicit unsupported-host diagnostic and separate no_std/platform feature checks.
Missing target components are prerequisites, not passing negative tests.

`cargo install --locked` tests shipped locks. Also resolve a fresh external
consumer explicitly, then compile the actual graph:

```sh
cargo +1.98.1 generate-lockfile --config 'resolver.incompatible-rust-versions="fallback"'
cargo +1.98.1 check --locked
```

Fallback preference alone does not guarantee compatible dependencies. Record
cache/index state and offline limits. Independently check a genuine no_std + alloc
library using only Core and REPL-core on the host and an installed bare target;
actual metadata/normal edges must show REPL→Core and Core→nothing, no Standard,
provider or tool dependency. An allocator/executable/hardware run is a further gate.
The alternate Linux REPL executable needs its exclusive feature and panic-abort
policy; neither/both features must fail with the intended diagnostic.

Native generated objects and the [runtime archive](../tools/native/runtime/README.md)
must match the compiler's exact Fipe version. This is not a promise that every
mismatch is mechanically refused. A Cargo runtime package is not itself an
installed archive; build its static library with the documented panic-abort policy.
Current native/LLVM APIs refuse checked resource programs and genuine Resource
values, including nested captures/filled arguments. This is refusal support, not
resource execution, a stable memory ABI or a new resource wire format.

## Publish only the approved cut

This document authorizes no upload. Select exact packages and versions after
source, archive, consumer, platform and compatibility review. Use standard Cargo
and an approved credential method; never put tokens in source, argv or logs.
From that selected source, replace the explicit package placeholder:

```sh
cargo +1.98.1 publish -p '<SELECTED_PACKAGE>' --registry crates-io --dry-run --locked
cargo +1.98.1 publish -p '<SELECTED_PACKAGE>' --registry crates-io --locked
```

The dry run is a separate preflight, not publication. Compare its actual archive
with the accepted bytes. Do not recommend blanket workspace upload for a partial
cut. Publish dependencies first and independently verify acceptance, checksum,
index visibility and official download before the next dependent package.
After timeout/interruption, reconcile registry state before any retry: unknown
outcome does not mean failure, and an accepted version cannot be replaced.
Then require a clean registry-only consumer before reporting availability.

Historical dev.2 tooling checks, **not commands for the new cut**, are:
`cargo install fipe-cli --version 0.2.0-dev.2 --locked` and
`cargo install fipe-lsp fipe-dap --version 0.2.0-dev.2 --locked`.
New-cut install tests must name its approved available versions; no dev.3 native
runtime/tool pair is implied by the two published dev.3 libraries.

## Current dev.3 source migration and handoff

In the current source cut, native `lower`, `object`, `listing` and LLVM
`module` return fallible `Result`s. Updating callers must handle those results;
this does not imply that dev.3 native packages are published.

Published Core dev.3 has genuine `Value::Resource`, classification changes and
private Tables identity state. Migrate exhaustive matches/literals and construct
Tables through public defaults/checked APIs. Guard/Closure/frame iterative Drop
can make owned-field moves/destructuring/struct update fail with E0509; borrow,
preserve owners or use `core::mem::take`. See [embedding migration](api/09-embed.md#opaque-resource-references).
Pending composite witnesses (`ResourceStep`, `ResourceError`,
`Machine::pending_resource_type_at`) prove language identity, not grants/liveness.
REPL-core dev.3 adoption includes its request-token/Start/End/Reset/acknowledgment
contract; pure `source_continuation` performs no Session construction or I/O.

A handoff names exact versions/source/checksums, migrations and measured limits.
Publication does not prove Reovim adoption, ignored-case/workaround removal or
consumer grant/stale/cleanup/Session acceptance. Pending future Core layout or
import compatibility decisions are not implemented by this guide. Final 13-package,
platform and installed-consumer gates remain distinct from the two-library cut.
Editor configuration/specification remain repository assets; tree-sitter npm
metadata is neither a Node binding nor an npm publication procedure.
