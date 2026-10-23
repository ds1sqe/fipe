# Examples

## Run a first script

From the repository root:

```sh
cargo run -p fipe-cli -- run Documentation/examples/hello.fipe
```

The result is `12`. Once installed, use
`fipe run Documentation/examples/hello.fipe` instead.

The [math and statistics modules](../../library/README.md) include a dataset
summary and interpolation example, with instructions for copying their source
modules beside your own script.

The [tour](../tour/README.md) walks through complete scripts. The practical
scripts in [dogfood](../dogfood/) include word counting, a calculator, CSV
summation and filesystem reports; each file's opening comment gives its input
and invocation. For example:

```sh
cargo run -p fipe-cli -- run Documentation/dogfood/calc.fipe \
  < Documentation/dogfood/input/sums.txt
```

## Embed in a Rust application

From the repository root:

```sh
cargo run --example embed
cargo run --example custom_host
```

[`embed.rs`](../../examples/embed.rs) checks and evaluates a pure pipeline.
[`custom_host.rs`](../../examples/custom_host.rs) declares `greet`, receives
its host call, validates the request and replies with a value. Both assert
the expected result. Replace the literal source with your application's input
and handle syntax/check errors and Faults instead of using `expect`.

## Explore the language

The numbered `.fipe` files in this directory demonstrate syntax and semantics,
including the conformance host. They are checked by the core test suite;
their filesystem paths and host assumptions are fixtures, so they are not
standalone tutorials for the standard host. `geometry.fipe` is the file module
used by `05-modules.fipe`. Consult the [specification](../spec/README.md) for
rules and the [standard-host contract](../../tools/host/README.md) for real I/O.
