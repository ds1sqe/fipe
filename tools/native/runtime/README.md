# fipe-native-rt

The runtime linked by Fipe's native and LLVM backends on x86-64 Linux.
It supplies the `rt_*` operations and uses `fipe-host` for filesystem,
arguments, environment and output.

From the Fipe workspace:

```sh
cargo build -p fipe-native-rt --release --locked
fipe build hello.fipe -o hello --runtime target/release/libfipe_native_rt.a
```

This is a library package, not a command installed by `cargo install`.
To build the archive in another Cargo project, depend on the exact same
version of `fipe-native-rt` as the compiler, and run:

```sh
cargo rustc -p fipe-native-rt --release --lib --crate-type staticlib -- -C panic=abort
```

Cargo places the archive at `target/release/libfipe_native_rt.a` by default;
pass its path with `--runtime`. Set `panic = "abort"` in the consuming
project's development and release profiles when building native executables.
The workspace already sets this policy. Native formats and Rust APIs are
experimental and must match the compiler version.
