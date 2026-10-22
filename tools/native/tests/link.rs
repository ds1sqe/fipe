//! Objects linked against the runtime with `cc` and run.
use fipe_native::{
    elf::{self, Object, Relocation, RelocationKind, Section, Symbol},
    link,
    x64::{self, Cond, Function, Inst, Label, Reg},
};
use std::{path::Path, process::Command};

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

/// `fipe_main` in the backend's frame: a slot at `[rbp-16]` that
/// `rt_unit` fills, its status the program's.
fn unit_main() -> Vec<Function> {
    let (fault, done) = (Label(0), Label(1));
    vec![Function {
        name: "fipe_main".into(),
        global: true,
        body: vec![
            Inst::Push(Reg::Rbp),
            Inst::Mov(Reg::Rbp, Reg::Rsp),
            // One slot, which keeps rsp 16-byte aligned at the call.
            Inst::SubRsp(16),
            Inst::Lea(Reg::Rdi, -16),
            Inst::Call("rt_unit".into()),
            Inst::TestEax,
            Inst::Jcc(Cond::Ne, fault),
            Inst::MovImm(Reg::Rax, 0),
            Inst::Jmp(done),
            Inst::Bind(fault),
            Inst::MovImm(Reg::Rax, 1),
            Inst::Bind(done),
            Inst::Mov(Reg::Rsp, Reg::Rbp),
            Inst::Pop(Reg::Rbp),
            Inst::Ret,
        ],
    }]
}

/// `listing` equals `tests/golden/<name>.lst`; `FIPE_BLESS=1` rewrites
/// the file instead.
fn golden(name: &str, listing: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.lst"));
    if std::env::var_os("FIPE_BLESS").is_some() {
        std::fs::write(&path, listing).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), listing);
}

#[test]
fn a_hand_built_object_calling_rt_unit_links_and_exits_zero() {
    let functions = unit_main();
    golden("unit", &x64::listing(&functions));
    let text = x64::assemble(&functions).unwrap();
    let mut symbols = text.symbols;
    symbols.push(Symbol {
        name: "fipe_source".into(),
        section: Section::Rodata,
        offset: 0,
        size: 5,
        global: false,
    });
    symbols.push(Symbol {
        name: "fipe_bodies".into(),
        section: Section::DataRelRo,
        offset: 0,
        size: 8,
        global: true,
    });
    let object = Object {
        text: text.bytes,
        text_relocations: text.relocations,
        rodata: b"unit\0".to_vec(),
        data: vec![0; 8],
        data_relocations: vec![Relocation {
            offset: 0,
            symbol: "fipe_main".into(),
            kind: RelocationKind::Absolute,
            addend: 0,
        }],
        symbols,
    };
    let dir = common::scratch("unit");
    let path = dir.join("unit.o");
    std::fs::write(&path, elf::write(&object).unwrap()).unwrap();

    let readelf = Command::new("readelf")
        .args(["-a", "-W"])
        .arg(&path)
        .output()
        .unwrap();
    let report = String::from_utf8(readelf.stdout).unwrap();
    assert!(readelf.status.success());
    assert_eq!(String::from_utf8_lossy(&readelf.stderr), "");
    assert!(!report.to_lowercase().contains("warning"), "{report}");
    for expected in [
        "R_X86_64_PLT32         0000000000000000 rt_unit - 4",
        "R_X86_64_64            0000000000000000 fipe_main + 0",
        "FUNC    GLOBAL DEFAULT    1 fipe_main",
        "OBJECT  LOCAL  DEFAULT    2 fipe_source",
        "OBJECT  GLOBAL DEFAULT    3 fipe_bodies",
        "NOTYPE  GLOBAL DEFAULT  UND rt_unit",
    ] {
        assert!(report.contains(expected), "{expected} in\n{report}");
    }

    let program = dir.join("unit");
    link::link(&path, common::runtime(), &program).unwrap();
    assert_eq!(common::run(&program), 0);
}

#[test]
fn two_symbols_of_one_name_are_refused() {
    let symbol = Symbol {
        name: "twice".into(),
        section: Section::Rodata,
        offset: 0,
        size: 0,
        global: false,
    };
    let object = Object {
        symbols: vec![symbol.clone(), symbol],
        ..Object::default()
    };
    assert_eq!(
        elf::write(&object),
        Err(elf::Error::Duplicate("twice".into()))
    );
}
