# Publishing Fipe packages

Fipe's Rust packages are one workspace and are distributed independently on
crates.io. The release version is `0.2.0-dev.2`. This release adds nested host namespaces to the core and publishes the tools.
The earlier core prerelease `0.2.0-dev.1` remains available.

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
4. `fipe-native`, then `fipe-cli`.

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
