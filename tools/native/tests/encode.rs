//! Each instruction form, encoded and compared byte for byte with what
//! GNU `as` assembles from the same instructions in Intel syntax.
use fipe_native::x64::{self, Cond, Error, Function, Inst, Label, Reg};
use std::{path::Path, process::Command};

fn function(body: Vec<Inst>) -> Function {
    Function {
        name: "f".into(),
        global: false,
        body,
    }
}

/// The `.text` bytes `as` makes of `source`.
fn assembled(dir: &Path, source: &str) -> Vec<u8> {
    let (input, object, text) =
        (dir.join("in.s"), dir.join("in.o"), dir.join("text.bin"));
    std::fs::write(&input, format!(".intel_syntax noprefix\n{source}\n"))
        .unwrap();
    let status = Command::new("as")
        .arg("-o")
        .arg(&object)
        .arg(&input)
        .status()
        .unwrap();
    assert!(status.success(), "as rejects:\n{source}");
    let status = Command::new("objcopy")
        .args(["-O", "binary", "-j", ".text"])
        .arg(&object)
        .arg(&text)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(text).unwrap()
}

/// Each body encodes to the bytes `as` makes of its source. `{disp32}`
/// keeps `as` from choosing the short forms the encoder never emits.
fn same(name: &str, cases: Vec<(Vec<Inst>, &str)>) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (body, source) in cases {
        let ours = x64::assemble(&[function(body)]).unwrap().bytes;
        assert_eq!(ours, assembled(&dir, source), "{source}");
    }
}

#[test]
fn push_and_pop_every_register_group() {
    same(
        "push_pop",
        vec![
            (vec![Inst::Push(Reg::Rax)], "push rax"),
            (vec![Inst::Push(Reg::Rbp)], "push rbp"),
            (vec![Inst::Push(Reg::R12)], "push r12"),
            (vec![Inst::Pop(Reg::Rbx)], "pop rbx"),
            (vec![Inst::Pop(Reg::R15)], "pop r15"),
        ],
    );
}

#[test]
fn mov_of_an_immediate_and_a_register() {
    same(
        "mov",
        vec![
            (vec![Inst::MovImm(Reg::Rax, 0)], "movabs rax, 0"),
            (vec![Inst::MovImm(Reg::R9, u64::MAX)], "movabs r9, -1"),
            (
                vec![Inst::MovImm(Reg::Rdi, 0x0123_4567_89ab_cdef)],
                "movabs rdi, 0x0123456789abcdef",
            ),
            (vec![Inst::Mov(Reg::Rbp, Reg::Rsp)], "mov rbp, rsp"),
            (vec![Inst::Mov(Reg::Rsp, Reg::Rbp)], "mov rsp, rbp"),
            (vec![Inst::Mov(Reg::R10, Reg::Rdi)], "mov r10, rdi"),
            (vec![Inst::Mov(Reg::Rsi, Reg::R13)], "mov rsi, r13"),
        ],
    );
}

#[test]
fn mov_and_lea_through_the_frame() {
    same(
        "frame",
        vec![
            (
                vec![Inst::Load(Reg::Rax, -24)],
                "{disp32} mov rax, [rbp-24]",
            ),
            (vec![Inst::Load(Reg::R11, 4096)], "mov r11, [rbp+4096]"),
            (
                vec![Inst::Store(-16, Reg::Rdi)],
                "{disp32} mov [rbp-16], rdi",
            ),
            (
                vec![Inst::Store(i32::MIN, Reg::R14)],
                "mov [rbp-0x80000000], r14",
            ),
            (vec![Inst::Lea(Reg::Rdi, -16)], "{disp32} lea rdi, [rbp-16]"),
            (vec![Inst::Lea(Reg::R8, 0)], "{disp32} lea r8, [rbp+0]"),
        ],
    );
}

#[test]
fn add_and_sub_of_the_stack_pointer() {
    // `as` takes the 32-bit immediate only for values past a byte.
    same(
        "stack",
        vec![
            (vec![Inst::SubRsp(4096)], "sub rsp, 4096"),
            (vec![Inst::AddRsp(4096)], "add rsp, 4096"),
            (vec![Inst::AddRsp(-129)], "add rsp, -129"),
        ],
    );
}

#[test]
fn call_jumps_test_and_ret() {
    let (l0, l1) = (Label(0), Label(1));
    same(
        "control",
        vec![
            (vec![Inst::Call("rt_unit".into())], "call rt_unit"),
            (vec![Inst::TestEax], "test eax, eax"),
            (vec![Inst::Ret], "ret"),
            (
                vec![
                    Inst::Bind(l0),
                    Inst::Jmp(l1),
                    Inst::Jcc(Cond::E, l0),
                    Inst::Jcc(Cond::Ne, l1),
                    Inst::Bind(l1),
                    Inst::Ret,
                ],
                ".L0:\n{disp32} jmp .L1\n{disp32} je .L0\n{disp32} jne .L1\n\
                 .L1:\nret",
            ),
        ],
    );
}

#[test]
fn a_call_is_relocated_against_its_symbol() {
    let text =
        x64::assemble(&[function(vec![Inst::Call("rt_unit".into())])]).unwrap();
    assert_eq!(text.relocations.len(), 1);
    let relocation = &text.relocations[0];
    assert_eq!(
        (
            relocation.offset,
            relocation.symbol.as_str(),
            relocation.addend
        ),
        (1, "rt_unit", -4)
    );
}

#[test]
fn functions_are_aligned_and_named() {
    let text = x64::assemble(&[
        function(vec![Inst::Ret]),
        Function {
            name: "g".into(),
            global: true,
            body: vec![Inst::TestEax, Inst::Ret],
        },
    ])
    .unwrap();
    let places: Vec<_> = text
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.offset, s.size, s.global))
        .collect();
    assert_eq!(places, [("f", 0, 1, false), ("g", 16, 3, true)]);
    assert_eq!(text.bytes[1..16], [0xcc; 15]);
}

#[test]
fn a_label_jumped_to_must_be_bound_once() {
    let unbound = x64::assemble(&[function(vec![Inst::Jmp(Label(3))])]);
    assert_eq!(unbound, Err(Error::Unbound("f".into(), Label(3))));
    let twice = x64::assemble(&[function(vec![
        Inst::Bind(Label(1)),
        Inst::Bind(Label(1)),
    ])]);
    assert_eq!(twice, Err(Error::Rebound("f".into(), Label(1))));
    assert_eq!(
        unbound.unwrap_err().to_string(),
        "f: label .L3 is never bound"
    );
}
