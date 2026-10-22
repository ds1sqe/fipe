//! The x86-64 instructions the backend emits and their encoding.
//!
//! Every jump and call takes a 32-bit displacement, so an instruction's size
//! never depends on a label and labels resolve in one pass at the end.
use crate::elf::{Relocation, RelocationKind, Section, Symbol};
use std::fmt::{self, Write};

/// A 64-bit general-purpose register, numbered as the encoding does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Reg {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl Reg {
    const fn number(self) -> u8 {
        self as u8
    }

    /// The low three bits, which go in the opcode or `ModRM` byte.
    const fn low(self) -> u8 {
        self.number() & 7
    }

    /// Whether the register needs a REX extension bit.
    const fn high(self) -> bool {
        self.number() >= 8
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 16] = [
            "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9",
            "r10", "r11", "r12", "r13", "r14", "r15",
        ];
        f.write_str(NAMES[usize::from(self.number())])
    }
}

/// The condition of a conditional jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    /// Equal, or zero.
    E,
    /// Not equal, or not zero.
    Ne,
}

/// A jump target within one function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(pub u32);

/// One instruction, or a label's place in the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inst {
    /// `push r`
    Push(Reg),
    /// `pop r`
    Pop(Reg),
    /// `mov r, imm64`
    MovImm(Reg, u64),
    /// `mov dst, src`
    Mov(Reg, Reg),
    /// `cmp a, b`
    Cmp(Reg, Reg),
    /// `mov r, [rbp+disp32]`
    Load(Reg, i32),
    /// `mov [rbp+disp32], r`
    Store(i32, Reg),
    /// `lea r, [rbp+disp32]`
    Lea(Reg, i32),
    /// `add rsp, imm32`
    AddRsp(i32),
    /// `sub rsp, imm32`
    SubRsp(i32),
    /// `call symbol`, relocated `R_X86_64_PLT32`.
    Call(String),
    /// `jmp label`
    Jmp(Label),
    /// `jcc label`
    Jcc(Cond, Label),
    /// `test eax, eax`
    TestEax,
    /// `ret`
    Ret,
    /// The place of a label; no bytes.
    Bind(Label),
}

impl fmt::Display for Inst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Push(r) => write!(f, "    push {r}"),
            Self::Pop(r) => write!(f, "    pop {r}"),
            Self::MovImm(r, value) => write!(f, "    mov {r}, {value}"),
            Self::Mov(dst, src) => write!(f, "    mov {dst}, {src}"),
            Self::Cmp(a, b) => write!(f, "    cmp {a}, {b}"),
            Self::Load(r, disp) => write!(f, "    mov {r}, [rbp{disp:+}]"),
            Self::Store(disp, r) => write!(f, "    mov [rbp{disp:+}], {r}"),
            Self::Lea(r, disp) => write!(f, "    lea {r}, [rbp{disp:+}]"),
            Self::AddRsp(value) => write!(f, "    add rsp, {value}"),
            Self::SubRsp(value) => write!(f, "    sub rsp, {value}"),
            Self::Call(symbol) => {
                write!(f, "    call {symbol}  ; R_X86_64_PLT32 {symbol}-4")
            }
            Self::Jmp(Label(label)) => write!(f, "    jmp .L{label}"),
            Self::Jcc(Cond::E, Label(label)) => write!(f, "    je .L{label}"),
            Self::Jcc(Cond::Ne, Label(label)) => write!(f, "    jne .L{label}"),
            Self::TestEax => f.write_str("    test eax, eax"),
            Self::Ret => f.write_str("    ret"),
            Self::Bind(Label(label)) => write!(f, ".L{label}:"),
        }
    }
}

/// A function: its symbol and its instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// The symbol naming it.
    pub name: String,
    /// Whether the symbol is seen outside the object.
    pub global: bool,
    /// Its instructions, labels local to it.
    pub body: Vec<Inst>,
}

/// Why a function does not assemble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A jump to a label the function never binds.
    Unbound(String, Label),
    /// A label bound twice in one function.
    Rebound(String, Label),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbound(name, Label(label)) => {
                write!(f, "{name}: label .L{label} is never bound")
            }
            Self::Rebound(name, Label(label)) => {
                write!(f, "{name}: label .L{label} is bound twice")
            }
        }
    }
}

impl std::error::Error for Error {}

/// The `.text` section of assembled functions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    /// The machine code.
    pub bytes: Vec<u8>,
    /// One symbol per function.
    pub symbols: Vec<Symbol>,
    /// One relocation per call.
    pub relocations: Vec<Relocation>,
}

/// Assembles `functions` one after another, each 16-byte aligned.
///
/// # Errors
///
/// When a function jumps to a label it never binds, or binds one twice.
pub fn assemble(functions: &[Function]) -> Result<Text, Error> {
    let mut text = Text::default();
    for function in functions {
        while text.bytes.len() % 16 != 0 {
            text.bytes.push(0xcc); // int3: never executed
        }
        let start = text.bytes.len();
        encode(function, &mut text)?;
        text.symbols.push(Symbol {
            name: function.name.clone(),
            section: Section::Text,
            offset: start as u64,
            size: (text.bytes.len() - start) as u64,
            global: function.global,
        });
    }
    Ok(text)
}

/// The listing of `functions`: one line per instruction, labels and
/// calls symbolic, each call with its relocation.
#[must_use]
pub fn listing(functions: &[Function]) -> String {
    let mut out = String::new();
    for function in functions {
        let binding = if function.global { "global" } else { "local" };
        let _ = writeln!(out, "{}:  ; {binding}", function.name);
        for inst in &function.body {
            let _ = writeln!(out, "{inst}");
        }
    }
    out
}

fn encode(function: &Function, text: &mut Text) -> Result<(), Error> {
    let code = &mut text.bytes;
    let mut bound: Vec<(u32, usize)> = Vec::new();
    // The offset of each rel32 to patch and the label it targets.
    let mut fixups: Vec<(usize, Label)> = Vec::new();
    for inst in &function.body {
        match *inst {
            Inst::Push(r) => short(code, 0x50, r),
            Inst::Pop(r) => short(code, 0x58, r),
            Inst::MovImm(r, value) => {
                code.extend([rex(false, r.high()), 0xb8 + r.low()]);
                code.extend(value.to_le_bytes());
            }
            Inst::Mov(dst, src) => {
                code.extend([
                    rex(src.high(), dst.high()),
                    0x89,
                    0xc0 | src.low() << 3 | dst.low(),
                ]);
            }
            Inst::Cmp(a, b) => {
                code.extend([
                    rex(b.high(), a.high()),
                    0x39,
                    0xc0 | b.low() << 3 | a.low(),
                ]);
            }
            Inst::Load(r, disp) => frame(code, 0x8b, r, disp),
            Inst::Store(disp, r) => frame(code, 0x89, r, disp),
            Inst::Lea(r, disp) => frame(code, 0x8d, r, disp),
            Inst::AddRsp(value) => stack(code, 0xc4, value),
            Inst::SubRsp(value) => stack(code, 0xec, value),
            Inst::Call(ref symbol) => {
                code.push(0xe8);
                text.relocations.push(Relocation {
                    offset: code.len() as u64,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Plt32,
                    // The displacement counts from the end of the field.
                    addend: -4,
                });
                code.extend([0; 4]);
            }
            Inst::Jmp(label) => {
                code.push(0xe9);
                fixups.push((code.len(), label));
                code.extend([0; 4]);
            }
            Inst::Jcc(cond, label) => {
                code.extend([0x0f, if cond == Cond::E { 0x84 } else { 0x85 }]);
                fixups.push((code.len(), label));
                code.extend([0; 4]);
            }
            Inst::TestEax => code.extend([0x85, 0xc0]),
            Inst::Ret => code.push(0xc3),
            Inst::Bind(Label(label)) => {
                if bound.iter().any(|&(other, _)| other == label) {
                    return Err(Error::Rebound(
                        function.name.clone(),
                        Label(label),
                    ));
                }
                bound.push((label, code.len()));
            }
        }
    }
    for (at, label) in fixups {
        let Some(&(_, target)) = bound.iter().find(|&&(l, _)| l == label.0)
        else {
            return Err(Error::Unbound(function.name.clone(), label));
        };
        let from = i64::try_from(at + 4).expect("text fits i64");
        let to = i64::try_from(target).expect("text fits i64");
        let rel = i32::try_from(to - from).expect("function fits rel32");
        code[at..at + 4].copy_from_slice(&rel.to_le_bytes());
    }
    Ok(())
}

/// A REX prefix with W set; `r` extends `ModRM.reg`, `b` `ModRM.rm` or
/// the opcode's register.
fn rex(r: bool, b: bool) -> u8 {
    0x48 | u8::from(r) << 2 | u8::from(b)
}

/// `push` or `pop`: the register in the opcode, REX only for r8–r15.
fn short(code: &mut Vec<u8>, opcode: u8, r: Reg) {
    if r.high() {
        code.push(0x41);
    }
    code.push(opcode + r.low());
}

/// `opcode r, [rbp+disp32]`: `ModRM` mod 10, rm 101 is rbp with a
/// 32-bit displacement and no SIB byte.
fn frame(code: &mut Vec<u8>, opcode: u8, r: Reg, disp: i32) {
    code.extend([rex(r.high(), false), opcode, 0x85 | r.low() << 3]);
    code.extend(disp.to_le_bytes());
}

/// `add` or `sub rsp, imm32`, told apart by `modrm`.
fn stack(code: &mut Vec<u8>, modrm: u8, value: i32) {
    code.extend([0x48, 0x81, modrm]);
    code.extend(value.to_le_bytes());
}
