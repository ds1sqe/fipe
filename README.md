# Fipe

Functional Pipeline Language, implemented in Rust without dependencies.

This first Fipe release continues the earlier `dlang` implementation:
a lexer, Pratt parser, bytecode compiler and VM, tree-walking evaluator,
closures, arrays, and an experimental Immix allocator. Its current values
are dynamically typed. The typed, embeddable pipeline design is the next
stage of the language.

## Library

```rust
use fipe::{
    bytecode_vm::{bytecode::compiler::Compiler, vm::VM},
    lexer::Lexer,
    object::{Int, Object},
    parser::Parser,
};

let source = "fn add(x, y) { x + y }; add(20, 22)";
let program = Parser::new(Lexer::new(source.to_owned())).parse().unwrap();
let mut compiler = Compiler::create().unwrap();
compiler.compile(program).unwrap();
let mut vm = VM::new(compiler.bytecode().unwrap());
while vm.is_runable() {
    vm.run_single().unwrap();
}
assert_eq!(vm.last_pop(), &Some(Object::Int(Int { value: 42 })));
```

## Command line

```sh
cargo install fipe
fipe mode=treewalker
fipe mode=bytecode
```

The tree-walker evaluates each input line. The bytecode mode is a stepping
debugger: enter a program, then press Enter for each VM instruction.
Enter `exit` to leave stepping mode; EOF exits either mode.

## Development

```sh
cargo test --all-targets
cargo test --doc
cargo +nightly miri test --test bytecode_regressions
cargo +nightly miri test --test allocator_regressions
```

The bytecode representation uses native-width, native-endian operands and
is an in-process representation. The APIs and language remain experimental.

## License

Licensed under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
