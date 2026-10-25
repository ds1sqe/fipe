# Publishing Fipe packages

Fipe's Rust packages are one workspace and are distributed independently on
crates.io. The last published workspace release is `0.2.0-dev.2`, which added nested
host namespaces and published the tools. The next prerelease candidate
is `0.2.0-dev.3`; source versioning alone does not publish any package.
The earlier core prerelease `0.2.0-dev.1` remains available.

The planned `0.2.0` release includes a Rust source compatibility change for
`fipe::value::Guard`, not present in published `0.2.0-dev.2`: its new `Drop`
implementation can reject owned field moves, destructuring and struct update
with E0509. See the [embedding migration guidance](api/09-embed.md#opaque-resource-references)
before updating consumers; the public fields and value/wire representation remain unchanged.

## Check the release

Run the [workspace checks](development.md), both embedding examples, and:

```sh
cargo package --workspace --locked
cargo publish --workspace --dry-run --locked
```

Cargo packages the workspace dependencies together and builds the extracted
sources. This catches files accidentally read from another package or omitted
from the archive. Inspect `target/package/*.crate` for the README, license
texts, host declarations and tree-sitter sources. Tool integration tests use
repository fixtures and run in the checkout, rather than being shipped.

During local preparation, add `--allow-dirty` to inspect uncommitted changes.
Before a release, review the exact diff and run the checks on the source being
published. Package generation does not upload anything; `publish --dry-run`
does not upload either.

## Publish

Authenticate locally with `cargo login` or a configured Cargo credential
provider. Keep credentials out of source files, command arguments and logs.
Then, for this release:

```sh
cargo publish --workspace --locked
```

Cargo publishes in dependency order. The graph is:

1. `fipe`.
2. `fipe-host`, `fipe-native-core`, `fipe-repl-core`, `fipe-lsp`, `tree-sitter-fipe`.
3. `fipe-llvm`, `fipe-repl`, `fipe-dap`, `fipe-native-rt`.
4. `fipe-native`, then `fipe-ffi` and `fipe-cli`.

`fipe-ffi` uses the native and LLVM packages in its development graph, so
include those edges when rehearsing the full workspace package set.

`fipe-native` also uses `fipe-llvm` in its development tests. Every internal
Cargo dependency carries an exact version and a local path: the checkout uses
the path, while the published manifest uses the registry version. Keep those
versions in sync. A future release must use a new version for any package
whose published contents change; crates.io versions cannot be overwritten.

After publication, verify from outside the checkout:

```sh
cargo install fipe-cli --version 0.2.0-dev.2 --locked
cargo install fipe-lsp fipe-dap --version 0.2.0-dev.2 --locked
```

Create a separate Rust project with `fipe = "=0.2.0-dev.2"` and run the README's
embedding example without path overrides. Native applications also need the
matching [runtime archive](../tools/native/runtime/README.md).

This process publishes Cargo packages. Editor configuration files and the
language specification live in the repository; tree-sitter's npm metadata is
not a Node binding or an npm release procedure.

## Support qualification

Use the [supported floor and target matrix](development.md#compatibility-and-support)
when checking archives. Clean `cargo install --locked` checks the shipped lock;
also check a fresh independent consumer on Rust 1.98.1 with Cargo's explicit
Rust-compatible fallback preference, then compile its resolved graph. Neither
path-only workspace checks nor resolver preference alone prove published package
contents or dependency compatibility. Record offline cache limits. Native
compiler/runtime artifacts require exact Fipe version pairing; Windows and both
macOS architectures still need actual VM/tooling runtime qualification.

## Consumer prerelease candidate

A bounded `0.2.0-dev.3` delivery can make current embedding and source continuation
available for consumer testing before final `0.2.0` acceptance. Its minimum archive
set is `fipe` followed by `fipe-repl-core`, with the latter's exact dependency on
the same version. Existing direct Session/Machine consumers need only `fipe`;
using `source_continuation` requires the second crate and a consumer-owned change.
No Standard, native runtime, FFI or language-server dependency is required.

All workspace packages inherit one version and exact internal requirements, so
prepare the version and lock coherently even when only these two archives are
selected. A two-package delivery does not publish the remaining eleven packages,
qualify native artifact pairing with an older version, or close final release
and platform gates. The current native/LLVM resource refusal remains unchanged.

Before uploading, retain the reviewed source revision, archive hashes and contents,
normalized registry manifests and exact dependency versions. Rehearse an extracted
archive-only consumer and an independent no_std build; then review the credential
method and explicit upload scope. Record each package's immutable version, archive
hash, acceptance/checksum and index visibility separately. After an interruption,
check registry state before retrying: a timeout does not establish upload failure,
and a published version cannot be overwritten. Publish the core before its exact
query dependency; verify the accepted bytes and clean registry-only consumer before
reporting availability. No upload is authorized by this document.

### Prerelease source migration

The candidate deliberately changes public Rust source APIs from published dev.2:

- Genuine `Value::Resource` and resource classification fields/diagnostics require
  downstream exhaustive matches and struct literals to migrate. Checked Tables
  include private resource identity state; construct them through public defaults
  and checked APIs rather than external full-field literals.
- `Guard`, `Closure` and materialized frame ownership now use iterative drop.
  Owned-field moves, destructuring and struct update can produce E0509; preserve
  owners or use `core::mem::take` for an owned field. See the
  [embedding migration](api/09-embed.md#opaque-resource-references).
- The pending composite API uses `ResourceStep`, `ResourceError` and
  `Machine::pending_resource_type_at`; witnesses prove language identity, not Host
  permission or liveness. A Host still owns its payload and lifecycle ledger.
- Adopting repl-core also requires its current request-token, Start/End/Reset and
  acknowledgment contract, not the old automatic owner assumptions. Its pure
  `source_continuation` query adds no Session construction or I/O.
- Native lowering/object/listing and LLVM module generation return fallible
  results and reject marked programs; this is not resource execution support.

A consumer handoff names the exact registry version, source and archive checksums,
API migration and measured limits. A registry upload does not prove the Reovim
consumer has updated its pins, re-enabled its ignored marked-pipe cases, removed
its workaround or passed grant/stale/cleanup/session checks. Those observations
remain consumer-owned acceptance gates. Final `0.2.0` compatibility, complete
workspace packaging and cross-platform release claims remain separately gated.
