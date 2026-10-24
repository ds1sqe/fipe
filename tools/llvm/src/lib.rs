//! Fipe's LLVM backend: a MIR program as textual LLVM IR, compiled to
//! an object by `clang` and linked against the native backend's
//! runtime (`tools/native/runtime`), which it shares with the x86-64
//! backend.
//!
//! The IR has the x86-64 backend's shape (`tools/native/src/lower.rs`):
//! one function per body, `i32 body_N(ptr args, ptr caps, ptr hidden,
//! ptr out)`, whose frame is an array of opaque 16-byte slots, every
//! operation on a slot a runtime call; `fipe_main` runs each module's
//! init body, then the main; `fipe_bodies` holds each body's address
//! and `fipe_unit` the unit's length and bytes. So both backends
//! observe as the VM does under the same runtime, and the native
//! backend's equivalence suite runs on either object.
#![forbid(unsafe_code)]

use fipe::{
    ir,
    mir::{self, Callee, Operand, Program, Rvalue, Statement, Terminator},
};
use std::{fmt, fmt::Write as _, io, path::Path, process::Command};

// Which operands the runtime reads for a site it performs.
use fipe_native_core::operands;

/// The runtime's exports the IR calls, with their LLVM signatures.
const DECLARATIONS: &str = "\
declare void @rt_units(ptr, i64)
declare void @rt_drops(ptr, i64)
declare void @rt_spots(ptr, ptr, i64)
declare void @rt_copy(ptr, ptr)
declare void @rt_move(ptr, ptr)
declare void @rt_const(ptr, i64)
declare void @rt_field(ptr, ptr, i64)
declare i64 @rt_switch(ptr)
declare i32 @rt_charge(i64, i64, i64)
declare void @rt_line(ptr, i64, i64, i64)
declare i32 @rt_assign(ptr, ptr, ptr, i64, i64, i64)
declare i32 @rt_call(ptr, ptr, ptr, i64, i64)
declare void @rt_index(ptr, ptr, i64)
declare i32 @rt_global(ptr, i64)
declare void @rt_store(ptr, i64, i64, i64)
declare i32 @rt_enter(ptr, ptr, i64, i64, i64)
declare i32 @rt_unreachable(i64, i64)
declare void @rt_push(ptr, i64, i64, i64)
declare void @rt_pop()
declare i64 @rt_test(ptr, ptr, i64, i64)
declare i32 @rt_init(i64)
";

/// The LLVM IR module of a program and the unit its executable
/// carries, which the runtime compiles again at startup.
///
/// # Errors
/// When the program declares resources, including unused declarations.
pub fn module(program: &Program, unit: &[u8]) -> Result<String, String> {
    fipe_native_core::codec::ensure_resource_free(program)?;
    let mut out = String::from("; Fipe program, LLVM backend\n");
    out.push_str(DECLARATIONS);
    for index in 0..program.bodies.len() {
        out.push('\n');
        out.push_str(&Body::new(program, index).lower());
    }
    out.push('\n');
    out.push_str(&main(program));
    let bodies: Vec<String> = (0..program.bodies.len())
        .map(|index| format!("ptr @body_{index}"))
        .collect();
    let _ = writeln!(
        out,
        "\n@fipe_bodies = constant [{} x ptr] [{}], align 8",
        bodies.len(),
        bodies.join(", ")
    );
    let mut bytes = String::new();
    for byte in unit {
        let _ = write!(bytes, "\\{byte:02X}");
    }
    let _ = writeln!(
        out,
        "@fipe_unit = constant <{{ i64, [{n} x i8] }}> <{{ i64 {n}, [{n} x i8] c\"{bytes}\" }}>, align 8",
        n = unit.len()
    );
    Ok(out)
}

/// `i32 fipe_main(ptr out)`: each module's init body in file order, a
/// module once; a Fault returns 1. Then the main body into `out`.
fn main(program: &Program) -> String {
    let mut out = String::from("define i32 @fipe_main(ptr %out) {\nentry:\n");
    let mut initialized = Vec::new();
    for (step, (module, init)) in program.inits.iter().enumerate() {
        if initialized.contains(module) {
            continue;
        }
        initialized.push(*module);
        let _ = write!(
            out,
            "  %s{step} = call i32 @rt_init(i64 {init})\n  \
             %f{step} = icmp ne i32 %s{step}, 0\n  \
             br i1 %f{step}, label %fault, label %i{step}\ni{step}:\n"
        );
    }
    let _ = write!(
        out,
        "  %r = call i32 @body_{}(ptr null, ptr null, ptr null, ptr %out)\n  \
         ret i32 %r\nfault:\n  ret i32 1\n}}\n",
        program.main
    );
    out
}

/// One body being lowered: its instructions, the counter naming its
/// values and blocks, and its frame of slots (locals, then scratch).
struct Body<'a> {
    index: usize,
    mir: &'a mir::Body,
    code: String,
    next: usize,
    slots: usize,
}

impl<'a> Body<'a> {
    fn new(program: &'a Program, index: usize) -> Self {
        let body = &program.bodies[index];
        let operands = body.blocks.iter().flat_map(|block| {
            let statements =
                block.statements.iter().map(|statement| match statement {
                    Statement::Assign(_, rvalue) => {
                        operands::rvalue(rvalue).map_or(0, <[_]>::len)
                    }
                    _ => 0,
                });
            let call = match &block.terminator {
                Terminator::Call { callee, args, .. } => {
                    operands::call(callee, args)
                }
                _ => 0,
            };
            statements.chain([call])
        });
        // One scratch slot at least, for an operand read by pointer.
        let scratch = operands.max().unwrap_or(0).max(1);
        Self {
            index,
            mir: body,
            code: String::new(),
            next: 0,
            slots: body.locals.len() + scratch,
        }
    }

    const fn fresh(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    fn line(&mut self, text: &str) {
        self.code.push_str("  ");
        self.code.push_str(text);
        self.code.push('\n');
    }

    /// A pointer to slot `slot`, as a fresh value.
    fn slot(&mut self, slot: usize) -> String {
        let name = format!("%p{}", self.fresh());
        self.line(&format!(
            "{name} = getelementptr inbounds [16 x i8], ptr %slots, i64 {slot}"
        ));
        name
    }

    fn scratch(&mut self, at: usize) -> String {
        self.slot(self.mir.locals.len() + at)
    }

    /// The site of statement `at` of `block`, or of its terminator.
    fn site(&self, block: usize, at: Option<usize>) -> String {
        format!("i64 {}, i64 {block}, i64 {}", self.index, at.unwrap_or(0))
    }

    /// A call returning a status: a Fault goes to the epilogue.
    fn checked(&mut self, call: &str) {
        let n = self.fresh();
        self.line(&format!("%s{n} = call i32 {call}"));
        self.line(&format!("%f{n} = icmp ne i32 %s{n}, 0"));
        self.line(&format!("br i1 %f{n}, label %fault, label %c{n}"));
        let _ = writeln!(self.code, "c{n}:");
    }

    /// Read an operand into slot `slot`.
    fn operand(&mut self, operand: Operand, slot: usize) {
        let out = self.slot(slot);
        match operand {
            Operand::Local(local) => {
                let from = self.slot(local);
                self.line(&format!(
                    "call void @rt_copy(ptr {out}, ptr {from})"
                ));
            }
            Operand::Const(index) => {
                self.line(&format!(
                    "call void @rt_const(ptr {out}, i64 {index})"
                ));
            }
            Operand::Capture(index) => self.line(&format!(
                "call void @rt_index(ptr {out}, ptr %caps, i64 {index})"
            )),
            Operand::Hidden(index) => self.line(&format!(
                "call void @rt_index(ptr {out}, ptr %hidden, i64 {index})"
            )),
            Operand::Global(definition) => {
                self.checked(&format!(
                    "@rt_global(ptr {out}, i64 {definition})"
                ));
            }
        }
    }

    /// Read operands into the scratch slots, in order.
    fn operands(&mut self, operands: &[Operand]) {
        let first = self.mir.locals.len();
        for (at, operand) in operands.iter().enumerate() {
            self.operand(*operand, first + at);
        }
    }

    /// A slot holding the operand's value: its local, or the first
    /// scratch slot it is read into.
    fn pointer(&mut self, operand: Operand) -> String {
        if let Operand::Local(local) = operand {
            return self.slot(local);
        }
        self.operand(operand, self.mir.locals.len());
        self.scratch(0)
    }

    fn statement(&mut self, block: usize, at: usize, statement: &Statement) {
        let site = self.site(block, Some(at));
        match statement {
            Statement::Charge(_) => {
                self.checked(&format!("@rt_charge({site})"));
            }
            Statement::Assign(dest, rvalue) => {
                self.assign(block, at, *dest, rvalue);
            }
            Statement::Line(operand) => {
                let from = self.pointer(*operand);
                self.line(&format!("call void @rt_line(ptr {from}, {site})"));
            }
            Statement::Store { value, .. } => {
                let from = self.pointer(*value);
                self.line(&format!("call void @rt_store(ptr {from}, {site})"));
            }
            Statement::Enter { value, .. } => {
                let from = self.pointer(*value);
                self.checked(&format!(
                    "@rt_enter(ptr {from}, ptr %hidden, {site})"
                ));
            }
            Statement::Push { value, .. } => {
                let from = self.pointer(*value);
                self.line(&format!("call void @rt_push(ptr {from}, {site})"));
            }
            Statement::Pop => self.line("call void @rt_pop()"),
        }
    }

    fn assign(
        &mut self,
        block: usize,
        at: usize,
        dest: usize,
        rvalue: &Rvalue,
    ) {
        match rvalue {
            Rvalue::Use(operand) => {
                self.operand(*operand, dest);
                return;
            }
            Rvalue::Field(operand, index) => {
                let from = self.pointer(*operand);
                let out = self.slot(dest);
                self.line(&format!(
                    "call void @rt_field(ptr {out}, ptr {from}, i64 {index})"
                ));
                return;
            }
            // The runtime computes every other rvalue from its operands.
            Rvalue::Admit { .. }
            | Rvalue::Construct { .. }
            | Rvalue::Native { .. }
            | Rvalue::Closure { .. }
            | Rvalue::Tuple(_)
            | Rvalue::Array(_)
            | Rvalue::Cast { .. }
            | Rvalue::Repeat { .. }
            | Rvalue::Length(_)
            | Rvalue::Wrap { .. }
            | Rvalue::Convert { .. }
            | Rvalue::Pack { .. } => {
                let read = operands::rvalue(rvalue).expect("read operands");
                self.operands(read);
            }
        }
        let out = self.slot(dest);
        let scratch = self.scratch(0);
        let site = self.site(block, Some(at));
        self.checked(&format!(
            "@rt_assign(ptr {out}, ptr {scratch}, ptr %hidden, {site})"
        ));
    }

    /// Fault `Unreachable` at the block's terminator.
    fn unreachable(&mut self, block: usize) {
        let n = self.fresh();
        self.line(&format!(
            "%u{n} = call i32 @rt_unreachable(i64 {}, i64 {block})",
            self.index
        ));
        self.line("br label %fault");
    }

    fn terminator(&mut self, block: usize, terminator: &Terminator) {
        match terminator {
            Terminator::Goto(target) => {
                self.line(&format!("br label %b{target}"));
            }
            Terminator::Return => self.line("br label %ret"),
            Terminator::Switch {
                value,
                cases,
                otherwise,
            } => {
                let from = self.pointer(*value);
                let n = self.fresh();
                self.line(&format!("%w{n} = call i64 @rt_switch(ptr {from})"));
                let default = otherwise.map_or_else(
                    || format!("u{n}"),
                    |target| format!("b{target}"),
                );
                // The first case of a value wins, as the x86 lowering's
                // compare chain does.
                let mut seen = Vec::new();
                let arms: Vec<String> = cases
                    .iter()
                    .filter(|(case, _)| {
                        let first = !seen.contains(case);
                        seen.push(*case);
                        first
                    })
                    .map(|(case, target)| {
                        format!("i64 {case}, label %b{target}")
                    })
                    .collect();
                self.line(&format!(
                    "switch i64 %w{n}, label %{default} [{}]",
                    arms.join(" ")
                ));
                if otherwise.is_none() {
                    let _ = writeln!(self.code, "u{n}:");
                    self.unreachable(block);
                }
            }
            Terminator::Call {
                callee,
                args,
                meanings,
                dest,
                target,
            } => {
                self.called(block, callee, args, meanings, *dest);
                self.line(&format!("br label %b{target}"));
            }
            Terminator::Test {
                value, pass, fail, ..
            } => {
                // 1 passes, 0 fails, 2 is a Fault.
                let from = self.pointer(*value);
                let n = self.fresh();
                self.line(&format!(
                    "%t{n} = call i64 @rt_test(ptr {from}, ptr %hidden, i64 {}, i64 {block})",
                    self.index
                ));
                self.line(&format!(
                    "switch i64 %t{n}, label %fault [i64 1, label %b{pass} i64 0, label %b{fail}]"
                ));
            }
            Terminator::Unreachable => self.unreachable(block),
        }
    }

    /// A call: a body without witnesses directly, any other callee by
    /// the runtime, which reads the arguments, then the callee's
    /// operand.
    fn called(
        &mut self,
        block: usize,
        callee: &Callee,
        args: &[Operand],
        meanings: &[ir::Type],
        dest: usize,
    ) {
        if let (Callee::Body(body), []) = (callee, meanings) {
            self.operands(args);
            let first = if args.is_empty() {
                String::from("null")
            } else {
                self.scratch(0)
            };
            let out = self.slot(dest);
            self.checked(&format!(
                "@body_{body}(ptr {first}, ptr null, ptr null, ptr {out})"
            ));
            return;
        }
        // The VM reads a global the callee names before the arguments'.
        if let Some(operand) = operands::callee(callee) {
            let first = self.mir.locals.len();
            self.operand(operand, first + args.len());
        }
        self.operands(args);
        let out = self.slot(dest);
        let scratch = self.scratch(0);
        self.checked(&format!(
            "@rt_call(ptr {out}, ptr {scratch}, ptr %hidden, i64 {}, i64 {block})",
            self.index
        ));
    }

    fn lower(mut self) -> String {
        let slots = self.slots;
        for (index, block) in self.mir.blocks.iter().enumerate() {
            let _ = writeln!(self.code, "b{index}:");
            for (at, statement) in block.statements.iter().enumerate() {
                self.statement(index, at, statement);
            }
            self.terminator(index, &block.terminator);
        }
        let mut entry = String::new();
        if self.mir.arity > 0 {
            // Spot 1 is local 1, after the result.
            let _ = write!(
                entry,
                "  %first = getelementptr inbounds [16 x i8], ptr %slots, i64 1\n  \
                 call void @rt_spots(ptr %first, ptr %args, i64 {})\n",
                self.mir.arity
            );
        }
        format!(
            "define internal i32 @body_{index}(ptr %args, ptr %caps, ptr %hidden, ptr %out) {{\n\
             entry:\n  %slots = alloca [{slots} x [16 x i8]], align 16\n  \
             call void @rt_units(ptr %slots, i64 {slots})\n{entry}  br label %b0\n\
             {code}ret:\n  %result = getelementptr inbounds [16 x i8], ptr %slots, i64 0\n  \
             call void @rt_move(ptr %out, ptr %result)\n  \
             call void @rt_drops(ptr %slots, i64 {slots})\n  ret i32 0\n\
             fault:\n  call void @rt_drops(ptr %slots, i64 {slots})\n  ret i32 1\n}}\n",
            index = self.index,
            code = self.code,
        )
    }
}

/// Why compiling the IR failed.
#[derive(Debug)]
pub enum Error {
    /// The IR file could not be written.
    Write(io::Error),
    /// `clang` could not be started.
    Spawn(io::Error),
    /// `clang` ran and failed; its standard error.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(error) => write!(f, "cannot write the IR: {error}"),
            Self::Spawn(error) => write!(f, "cannot run clang: {error}"),
            Self::Failed(stderr) => write!(f, "clang failed:\n{stderr}"),
        }
    }
}

impl std::error::Error for Error {}

/// The clang that compiles the IR: `FIPE_CLANG`, else `clang-23`.
fn clang() -> String {
    std::env::var("FIPE_CLANG").unwrap_or_else(|_| String::from("clang-23"))
}

/// Compile a module's IR to the position-independent object `object`
/// with `clang -O2`; the IR is written beside it as `.ll` and removed.
///
/// # Errors
///
/// When the IR cannot be written or `clang` cannot run or fails.
pub fn compile(ir: &str, object: &Path) -> Result<(), Error> {
    let source = object.with_extension("ll");
    std::fs::write(&source, ir).map_err(Error::Write)?;
    let result = Command::new(clang())
        .args(["-c", "-O2", "-fPIC", "-x", "ir"])
        .arg(&source)
        .arg("-o")
        .arg(object)
        .output();
    let _ = std::fs::remove_file(&source);
    let result = result.map_err(Error::Spawn)?;
    if result.status.success() {
        Ok(())
    } else {
        Err(Error::Failed(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ))
    }
}
