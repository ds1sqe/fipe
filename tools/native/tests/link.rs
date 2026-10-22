//! Objects linked against the runtime with `cc` and run.
use fipe_native::link;
use std::process::Command;

mod common;

#[test]
fn an_object_holding_only_fipe_main_links_and_exits_zero() {
    let dir = common::scratch("only_main");
    let source = dir.join("main.s");
    std::fs::write(
        &source,
        ".intel_syntax noprefix\n.globl fipe_main\nfipe_main:\n\
         xor eax, eax\nret\n.section .note.GNU-stack,\"\",@progbits\n",
    )
    .unwrap();
    let object = dir.join("main.o");
    let status = Command::new("as")
        .arg("-o")
        .arg(&object)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let program = dir.join("main");
    link::link(&object, common::runtime(), &program).unwrap();
    assert_eq!(common::run(&program), 0);
}
