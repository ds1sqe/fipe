use crate::{bytecode_vm::bytecode::instruction::Instruction, object::CompiledFunction};

pub struct Frame {
    fun: CompiledFunction,
    bp: usize,
    /// instruction pointer
    ic: usize,
}

impl Frame {
    pub fn new(fun: CompiledFunction, base_ptr: usize) -> Self {
        Self {
            fun,
            bp: base_ptr,
            ic: 0,
        }
    }

    pub fn rext_instruction(&self) -> Instruction {
        self.fun.instructions.read_instruction(self.ic)
    }

    pub fn set_ic(&mut self, tgt: usize) {
        self.ic = tgt
    }
    pub fn add_ic(&mut self, tgt: usize) {
        self.ic += tgt
    }

    pub fn ic(&self) -> usize {
        self.ic
    }
    pub fn bp(&self) -> usize {
        self.bp
    }

    pub fn is_runnable(&self) -> bool {
        self.ic < self.fun.instructions.length()
    }

    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        buf += &format!("<FRAME> IC : {}, BP : {}\n", self.ic, self.bp);
        buf += "\nINSTRUCTIONS\n";
        buf += &self.fun.instructions.to_string_with_highlight(self.ic);

        buf
    }
}
