//! The IR module of a program: its functions and data, and an object
//! `clang` compiles from it. Equivalence with the VM is the native
//! backend's suite, run with `FIPE_BACKEND=llvm` (README §Checks).
use fipe::{check, decl::Declarations, lex::lex, parse::parse};
use std::path::Path;

fn program(source: &str) -> fipe::mir::Program {
    let module = parse(lex(source)).unwrap();
    let unit = check::check(
        &module,
        &Declarations::new(),
        &[],
        check::Options::default(),
    )
    .unwrap();
    fipe::compile::compile(&unit)
}

#[test]
fn a_module_defines_each_body_the_main_and_the_unit() {
    let program = program("double = n => n * 2\ndouble 21\n");
    let ir = fipe_llvm::module(&program, b"unit");
    for index in 0..program.bodies.len() {
        assert!(ir.contains(&format!("define internal i32 @body_{index}(")));
    }
    assert!(ir.contains("define i32 @fipe_main(ptr %out)"));
    assert!(ir.contains(&format!(
        "@fipe_bodies = constant [{} x ptr]",
        program.bodies.len()
    )));
    assert!(ir.contains("[4 x i8] c\"\\75\\6E\\69\\74\""), "{ir}");
}

#[test]
fn clang_compiles_the_module_to_an_object() {
    let program = program("xs = [1, 2, 3]\nxs | map (n => n + 1) | sum\n");
    let ir = fipe_llvm::module(&program, b"");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("module");
    std::fs::create_dir_all(&dir).unwrap();
    let object = dir.join("sum.o");
    fipe_llvm::compile(&ir, &object).unwrap();
    let bytes = std::fs::read(&object).unwrap();
    assert_eq!(&bytes[..4], b"\x7fELF");
    assert!(!dir.join("sum.ll").exists(), "the IR file is removed");
}
