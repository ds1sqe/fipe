//! A MIR program lowered to x86-64: one function per body, `fipe_main`
//! entering the main body, and the object holding them with the unit
//! the runtime compiles again at startup.
//!
//! A body is `u32 body_N(const Value* args, const Value* caps, const
//! Value* hidden, Value* out)`. Its frame keeps the four arguments,
//! then its locals and the scratch slots a call's operands are read
//! into, one opaque 16-byte slot each; every operation on a slot is a
//! runtime call. A status 1 jumps to the epilogue, which drops the
//! slots and returns it.
use crate::{
    elf::{Object, Relocation, RelocationKind, Section, Symbol},
    x64::{self, Cond, Function, Inst, Label, Reg},
};
use fipe::mir::{
    self, Callee, Operand, Program, Rvalue, Statement, Terminator,
};
use std::fmt::{self, Write};

// The lowering reads only the table of which natives the runtime has.
#[allow(dead_code)]
#[path = "../runtime/src/ops/natives.rs"]
mod natives;

/// Why a program does not lower.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A form the backend does not lower yet, in a body.
    Unsupported { body: usize, form: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { body, form } => {
                write!(
                    f,
                    "body {body}: the native backend cannot lower {form} yet"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// The bytes of the frame before the slots: the four arguments.
const HEADER: i32 = 32;
const SLOT: i32 = 16;

/// The functions of a program: each body, then `fipe_main`.
///
/// # Errors
///
/// [`Error::Unsupported`] for the first form not lowered yet.
pub fn lower(program: &Program) -> Result<Vec<Function>, Error> {
    let natives = natives::Natives::new(&program.tables.borrow());
    let mut functions = Vec::with_capacity(program.bodies.len() + 1);
    for index in 0..program.bodies.len() {
        functions.push(Lowering::new(program, index, &natives).body()?);
    }
    functions.push(Function {
        name: "fipe_main".into(),
        global: true,
        body: vec![
            Inst::Push(Reg::Rbp),
            Inst::Mov(Reg::Rbp, Reg::Rsp),
            Inst::Mov(Reg::Rcx, Reg::Rdi),
            Inst::MovImm(Reg::Rdi, 0),
            Inst::MovImm(Reg::Rsi, 0),
            Inst::MovImm(Reg::Rdx, 0),
            Inst::Call(symbol(program.main)),
            Inst::Pop(Reg::Rbp),
            Inst::Ret,
        ],
    });
    Ok(functions)
}

/// The object of a program: its functions, `fipe_bodies` (each body's
/// address, by index) and `fipe_unit` (the unit's length, then its
/// bytes).
///
/// # Errors
///
/// As [`lower`].
///
/// # Panics
///
/// When a generated function does not assemble, a defect of the
/// lowering.
pub fn object(program: &Program, unit: &[u8]) -> Result<Object, Error> {
    let text = x64::assemble(&lower(program)?).expect("labels bound once");
    let mut symbols = text.symbols;
    let mut rodata = (unit.len() as u64).to_le_bytes().to_vec();
    rodata.extend(unit);
    symbols.push(Symbol {
        name: "fipe_unit".into(),
        section: Section::Rodata,
        offset: 0,
        size: rodata.len() as u64,
        global: true,
    });
    let bodies = program.bodies.len();
    symbols.push(Symbol {
        name: "fipe_bodies".into(),
        section: Section::DataRelRo,
        offset: 0,
        size: 8 * bodies as u64,
        global: true,
    });
    Ok(Object {
        text: text.bytes,
        text_relocations: text.relocations,
        rodata,
        data: vec![0; 8 * bodies],
        data_relocations: (0..bodies)
            .map(|index| Relocation {
                offset: 8 * index as u64,
                symbol: symbol(index),
                kind: RelocationKind::Absolute,
                addend: 0,
            })
            .collect(),
        symbols,
    })
}

/// The listing of a program's object: its functions, then its data.
///
/// # Errors
///
/// As [`lower`].
pub fn listing(program: &Program) -> Result<String, Error> {
    let mut out = x64::listing(&lower(program)?);
    out.push_str("fipe_bodies:  ; global, .data.rel.ro\n");
    for index in 0..program.bodies.len() {
        let name = symbol(index);
        let _ = writeln!(out, "    .quad {name}  ; R_X86_64_64 {name}+0");
    }
    out.push_str(
        "fipe_unit:  ; global, .rodata: the unit's length and bytes\n",
    );
    Ok(out)
}

fn symbol(body: usize) -> String {
    format!("body_{body}")
}

const fn imm(value: usize) -> u64 {
    value as u64
}

/// One body being lowered.
struct Lowering<'a> {
    index: usize,
    body: &'a mir::Body,
    natives: &'a natives::Natives,
    code: Vec<Inst>,
    /// Locals, then scratch slots.
    slots: usize,
}

impl<'a> Lowering<'a> {
    fn new(
        program: &'a Program,
        index: usize,
        natives: &'a natives::Natives,
    ) -> Self {
        let body = &program.bodies[index];
        let operands = body.blocks.iter().flat_map(|block| {
            let statements =
                block.statements.iter().map(|statement| match statement {
                    Statement::Assign(_, Rvalue::Construct { fields, .. }) => {
                        fields.len()
                    }
                    Statement::Assign(_, Rvalue::Native { args, .. }) => {
                        args.len()
                    }
                    _ => 0,
                });
            let call = match &block.terminator {
                Terminator::Call { args, .. } => args.len(),
                _ => 0,
            };
            statements.chain([call])
        });
        // One scratch slot at least, for an operand read by pointer.
        let scratch = operands.max().unwrap_or(0).max(1);
        Self {
            index,
            body,
            natives,
            code: Vec::new(),
            slots: body.locals.len() + scratch,
        }
    }

    const fn unsupported<T>(&self, form: &'static str) -> Result<T, Error> {
        Err(Error::Unsupported {
            body: self.index,
            form,
        })
    }

    /// The frame offset of slot `slot`.
    fn slot(&self, slot: usize) -> i32 {
        let slot = i32::try_from(slot).expect("a frame fits i32");
        self.base() + SLOT * slot
    }

    fn base(&self) -> i32 {
        let slots = i32::try_from(self.slots).expect("a frame fits i32");
        -(HEADER + SLOT * slots)
    }

    fn scratch(&self, at: usize) -> i32 {
        self.slot(self.body.locals.len() + at)
    }

    fn block_label(block: usize) -> Label {
        Label(u32::try_from(block).expect("blocks fit u32"))
    }

    fn ret(&self) -> Label {
        Self::block_label(self.body.blocks.len())
    }

    fn fault(&self) -> Label {
        Self::block_label(self.body.blocks.len() + 1)
    }

    fn done(&self) -> Label {
        Self::block_label(self.body.blocks.len() + 2)
    }

    fn emit(&mut self, insts: impl IntoIterator<Item = Inst>) {
        self.code.extend(insts);
    }

    fn call(&mut self, symbol: &str) {
        self.code.push(Inst::Call(symbol.into()));
    }

    /// A call returning a status: a Fault goes to the epilogue.
    fn checked(&mut self, symbol: &str) {
        self.call(symbol);
        let fault = self.fault();
        self.emit([Inst::TestEax, Inst::Jcc(Cond::Ne, fault)]);
    }

    /// The site of statement `at` of `block` in `rdi` and on, or of
    /// its terminator when `at` is `None`.
    fn site(&mut self, regs: &[Reg], block: usize, at: Option<usize>) {
        let values = [self.index, block, at.unwrap_or(0)];
        for (reg, value) in regs.iter().zip(values) {
            self.code.push(Inst::MovImm(*reg, imm(value)));
        }
    }

    /// Read an operand into slot `slot`.
    fn operand(&mut self, operand: Operand, slot: usize) -> Result<(), Error> {
        let out = self.slot(slot);
        match operand {
            Operand::Local(local) => {
                let from = self.slot(local);
                self.emit([
                    Inst::Lea(Reg::Rdi, out),
                    Inst::Lea(Reg::Rsi, from),
                ]);
                self.call("rt_copy");
            }
            Operand::Const(index) => {
                self.emit([
                    Inst::Lea(Reg::Rdi, out),
                    Inst::MovImm(Reg::Rsi, imm(index)),
                ]);
                self.call("rt_const");
            }
            Operand::Capture(_) => return self.unsupported("a captured value"),
            Operand::Global(_) => return self.unsupported("a global"),
            Operand::Hidden(_) => return self.unsupported("a hidden witness"),
        }
        Ok(())
    }

    /// Read operands into the scratch slots, in order.
    fn operands(&mut self, operands: &[Operand]) -> Result<(), Error> {
        let first = self.body.locals.len();
        for (at, operand) in operands.iter().enumerate() {
            self.operand(*operand, first + at)?;
        }
        Ok(())
    }

    /// The frame offset of a slot holding the operand's value: its
    /// local, or the first scratch slot it is read into.
    fn pointer(&mut self, operand: Operand) -> Result<i32, Error> {
        if let Operand::Local(local) = operand {
            return Ok(self.slot(local));
        }
        self.operand(operand, self.body.locals.len())?;
        Ok(self.scratch(0))
    }

    /// Refuse a prelude native the runtime has not copied yet.
    fn native(&self, op: usize) -> Result<(), Error> {
        match self.natives.get(op) {
            natives::Kind::Native(None) => {
                self.unsupported("a native other than an Integer operation")
            }
            natives::Kind::Native(Some(_)) | natives::Kind::Host => Ok(()),
        }
    }

    fn statement(
        &mut self,
        block: usize,
        at: usize,
        statement: &Statement,
    ) -> Result<(), Error> {
        let site = [Reg::Rdi, Reg::Rsi, Reg::Rdx];
        match statement {
            Statement::Charge(_) => {
                self.site(&site, block, Some(at));
                self.checked("rt_charge");
            }
            Statement::Assign(dest, rvalue) => {
                self.assign(block, at, *dest, rvalue)?;
            }
            Statement::Line(operand) => {
                let from = self.pointer(*operand)?;
                self.code.push(Inst::Lea(Reg::Rdi, from));
                self.site(&[Reg::Rsi, Reg::Rdx, Reg::Rcx], block, Some(at));
                self.call("rt_line");
            }
            Statement::Store { .. } => return self.unsupported("a Store"),
            Statement::Enter { .. } => return self.unsupported("an Enter"),
            Statement::Push { .. } => return self.unsupported("a Push"),
            Statement::Pop => return self.unsupported("a Pop"),
        }
        Ok(())
    }

    fn assign(
        &mut self,
        block: usize,
        at: usize,
        dest: usize,
        rvalue: &Rvalue,
    ) -> Result<(), Error> {
        let out = self.slot(dest);
        match rvalue {
            Rvalue::Use(operand) => return self.operand(*operand, dest),
            Rvalue::Field(operand, index) => {
                let from = self.pointer(*operand)?;
                self.emit([
                    Inst::Lea(Reg::Rdi, out),
                    Inst::Lea(Reg::Rsi, from),
                    Inst::MovImm(Reg::Rdx, imm(*index)),
                ]);
                self.call("rt_field");
                return Ok(());
            }
            Rvalue::Construct { fields, .. } => self.operands(fields)?,
            Rvalue::Native { op, args } => {
                self.native(*op)?;
                self.operands(args)?;
            }
            Rvalue::Tuple(_) => return self.unsupported("a Tuple"),
            Rvalue::Array(_) => return self.unsupported("an Array"),
            Rvalue::Repeat { .. } => return self.unsupported("a Repeat"),
            Rvalue::Length(_) => return self.unsupported("a Length"),
            Rvalue::Closure { .. } => return self.unsupported("a Closure"),
            Rvalue::Wrap { .. } => return self.unsupported("a Wrap"),
            Rvalue::Cast { .. } => return self.unsupported("a Cast"),
            Rvalue::Convert { .. } => return self.unsupported("a Convert"),
            Rvalue::Pack { .. } => return self.unsupported("a Pack"),
        }
        let scratch = self.scratch(0);
        self.emit([Inst::Lea(Reg::Rdi, out), Inst::Lea(Reg::Rsi, scratch)]);
        self.site(&[Reg::Rdx, Reg::Rcx, Reg::R8], block, Some(at));
        self.checked("rt_assign");
        Ok(())
    }

    fn terminator(
        &mut self,
        block: usize,
        terminator: &Terminator,
    ) -> Result<(), Error> {
        match terminator {
            Terminator::Goto(target) => {
                self.code.push(Inst::Jmp(Self::block_label(*target)));
            }
            Terminator::Return => {
                let ret = self.ret();
                self.code.push(Inst::Jmp(ret));
            }
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                let from = self.pointer(*value)?;
                self.code.push(Inst::Lea(Reg::Rdi, from));
                self.call("rt_switch");
                for (case, target) in cases {
                    self.emit([
                        Inst::MovImm(Reg::Rcx, imm(*case)),
                        Inst::Cmp(Reg::Rax, Reg::Rcx),
                        Inst::Jcc(Cond::E, Self::block_label(*target)),
                    ]);
                }
                if let Some(otherwise) = otherwise {
                    self.code.push(Inst::Jmp(Self::block_label(*otherwise)));
                } else {
                    self.site(&[Reg::Rdi, Reg::Rsi], block, None);
                    self.call("rt_unreachable");
                    let fault = self.fault();
                    self.code.push(Inst::Jmp(fault));
                }
            }
            Terminator::Call {
                callee,
                args,
                meanings,
                dest,
                target,
            } => {
                if !meanings.is_empty() {
                    return self.unsupported("a call with witnesses");
                }
                self.called(block, callee, args, *dest)?;
                self.code.push(Inst::Jmp(Self::block_label(*target)));
            }
            Terminator::Test { .. } => return self.unsupported("a Test"),
            Terminator::Unreachable => {
                return self.unsupported("an Unreachable");
            }
        }
        Ok(())
    }

    /// A call's callee with its arguments, the result into `dest`.
    fn called(
        &mut self,
        block: usize,
        callee: &Callee,
        args: &[Operand],
        dest: usize,
    ) -> Result<(), Error> {
        match callee {
            Callee::Body(_) | Callee::Host(_) => {}
            Callee::Native(op) => self.native(*op)?,
            Callee::Derived { .. } => {
                return self.unsupported("a Derived callee");
            }
            Callee::Dispatch { .. } => {
                return self.unsupported("a Dispatch callee");
            }
            Callee::Value(_) => return self.unsupported("a Value callee"),
        }
        self.operands(args)?;
        let (out, scratch) = (self.slot(dest), self.scratch(0));
        if let Callee::Body(body) = callee {
            let args = if args.is_empty() {
                Inst::MovImm(Reg::Rdi, 0)
            } else {
                Inst::Lea(Reg::Rdi, scratch)
            };
            self.emit([
                args,
                Inst::MovImm(Reg::Rsi, 0),
                Inst::MovImm(Reg::Rdx, 0),
                Inst::Lea(Reg::Rcx, out),
            ]);
            self.checked(&symbol(*body));
        } else {
            self.emit([Inst::Lea(Reg::Rdi, out), Inst::Lea(Reg::Rsi, scratch)]);
            self.site(&[Reg::Rdx, Reg::Rcx], block, None);
            self.checked("rt_call");
        }
        Ok(())
    }

    fn body(mut self) -> Result<Function, Error> {
        let (slots, base) = (imm(self.slots), self.base());
        self.emit([
            Inst::Push(Reg::Rbp),
            Inst::Mov(Reg::Rbp, Reg::Rsp),
            // A multiple of 16, so each call is 16-byte aligned.
            Inst::SubRsp(-base),
            Inst::Store(-8, Reg::Rdi),
            Inst::Store(-16, Reg::Rsi),
            Inst::Store(-24, Reg::Rdx),
            Inst::Store(-32, Reg::Rcx),
            Inst::Lea(Reg::Rdi, base),
            Inst::MovImm(Reg::Rsi, slots),
        ]);
        self.call("rt_units");
        if self.body.arity > 0 {
            // Spot 1 is local 1, after the result.
            let first = self.slot(1);
            self.emit([
                Inst::Lea(Reg::Rdi, first),
                Inst::Load(Reg::Rsi, -8),
                Inst::MovImm(Reg::Rdx, imm(self.body.arity)),
            ]);
            self.call("rt_spots");
        }
        for (index, block) in self.body.blocks.iter().enumerate() {
            self.code.push(Inst::Bind(Self::block_label(index)));
            for (at, statement) in block.statements.iter().enumerate() {
                self.statement(index, at, statement)?;
            }
            self.terminator(index, &block.terminator)?;
        }
        let (ret, fault, done) = (self.ret(), self.fault(), self.done());
        let result = self.slot(0);
        self.emit([
            Inst::Bind(ret),
            Inst::Load(Reg::Rdi, -32),
            Inst::Lea(Reg::Rsi, result),
        ]);
        self.call("rt_move");
        self.emit([Inst::Lea(Reg::Rdi, base), Inst::MovImm(Reg::Rsi, slots)]);
        self.call("rt_drops");
        self.emit([
            Inst::MovImm(Reg::Rax, 0),
            Inst::Jmp(done),
            Inst::Bind(fault),
            Inst::Lea(Reg::Rdi, base),
            Inst::MovImm(Reg::Rsi, slots),
        ]);
        self.call("rt_drops");
        self.emit([
            Inst::MovImm(Reg::Rax, 1),
            Inst::Bind(done),
            Inst::Mov(Reg::Rsp, Reg::Rbp),
            Inst::Pop(Reg::Rbp),
            Inst::Ret,
        ]);
        Ok(Function {
            name: symbol(self.index),
            global: false,
            body: self.code,
        })
    }
}
