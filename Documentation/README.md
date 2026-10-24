# Fipe

Start with the route that matches your task. These pages describe the current
checkout; some development APIs are newer than the published `0.2.0-dev.2`
baseline. Use matching source and documentation, and consult the
[compatibility and support policy](development.md#compatibility-and-support)
for the selected Rust 1.98.1 floor and platform qualification gaps.

## Guides and examples

| Your task | Start here |
| --- | --- |
| Write scripts with the standard host | [Tour](tour/README.md), then [CLI reference](../tools/fipe/README.md) |
| Find a runnable script or Rust example | [Examples index](examples/README.md), including the practical dogfood scripts |
| Embed with an application-owned Host | [Minimal Rust example](../README.md#embed-in-rust), [custom Host example](../examples/custom_host.rs), then [embedding API](api/09-embed.md) |
| Choose between your Host and Standard | [Layer ownership](architecture.md) and [standard-host contract](../tools/host/README.md) |
| Write or change Fipe with an agent | [Agent guidance](agents.md) and the [language specification](spec/README.md) |
| Contribute or verify a checkout | [Development and CI](development.md); [publishing](publishing.md) owns package/release checks |

Core, Embed and the Host contract are available from `fipe` alone with `no_std` +
`alloc`. Standard is optional and supplies OS operations. The examples and owning
API chapters describe the boundary; this index does not add permission or resource
guarantees.

## Editors and tools

- [Language server](../tools/lsp/README.md): checking, navigation and editor features;
  use the declarations matching your Host.
- [Debug adapter](../tools/dap/README.md): current launch and Standard/trace behavior.
- [REPL](../tools/repl/README.md) and [REPL core](../tools/repl-core/README.md): terminal
  use or an application-driven loop without I/O.
- [Tree-sitter](../tools/tree-sitter/README.md): syntax trees and highlighting.
- [Neovim](../tools/editor/nvim/README.md) and [Helix](../tools/editor/helix/README.md):
  repository editor setup.
- [Native backend](../tools/native/README.md), [LLVM backend](../tools/llvm/README.md)
  and [runtime](../tools/native/runtime/README.md): x86-64 Linux executable tooling.
- [Experimental scalar providers](../tools/ffi/README.md) and
  [native library boundary](api/10-native-libraries.md): current checkout VM scalar
  proof and its limits, not resource or native/LLVM provider support.

Tool and editor links refer to the source repository. They do not imply those
manuals or configurations are bundled inside the Core crate archive. Each owner
records its actual platform and feature limits; selected release targets are not
claims that every platform or integration has passed qualification.

## Reference

The [specification index](spec/README.md) defines language rules;
[open decisions](spec/11-open.md) are unresolved proposals, not implemented features.
The [prelude source](../prelude/prelude.fipe) lists built-in declarations.

The API chapters follow the processing boundary:

1. [Lexing](api/01-lexer.md)
2. [Parsing](api/02-parser.md)
3. [Checking](api/07-check.md)
4. [MIR and compilation](api/08-mir.md)
5. [Embedding, sessions and execution](api/09-embed.md)
6. [Native library boundary](api/10-native-libraries.md)

The current prerelease remains experimental. Published `0.1.0` is the earlier
language; examples and APIs from it should not be mixed with this checkout.
