use crate::object::ClosureFunction;

use super::super::bytecode::errors::FrameError;
use super::super::bytecode::instruction::Instruction;

/// Stack Frame of function.
///
/// * cf - Compiled function closure
/// * bp - base pointer aka bp (stack index of return)
/// * ic - instruction counter (same as program counter) aka ic
pub struct Frame {
    /// * cf - Compiled function closure
    cf: ClosureFunction,
    /// * bp - base pointer aka bp (stack index of return)
    bp: usize,
    /// * ic - instruction counter (same as program counter) aka ic
    ic: usize,
}

impl Frame {
    /// Creates a new [`Frame`].
    /// * `fun`- Compiled function closure
    /// * `base_ptr` - base pointer aka bp (stack index of return)
    pub fn new(fun: ClosureFunction, base_ptr: usize) -> Self {
        Self {
            cf: fun,
            bp: base_ptr,
            ic: 0,
        }
    }

    /// Returns the rext instruction of this [`Frame`]
    /// by reading instruction from [`Self::cf`],
    ///
    /// # Errors [`FrameError::InnerError`]
    ///
    /// This function will return an error if failed to read instruction
    pub fn rext_instruction(&self) -> Result<Instruction, FrameError> {
        Ok(self.cf.fun.instructions.read_instruction(self.ic)?)
    }

    /// Sets the ic of this [`Frame`].
    pub fn set_ic(&mut self, tgt: usize) {
        self.ic = tgt
    }
    /// Add ic with given `offset`
    pub fn add_ic(&mut self, offset: usize) {
        self.ic += offset
    }

    /// Returns the ic of this [`Frame`].
    pub fn ic(&self) -> usize {
        self.ic
    }

    /// Returns the bp of this [`Frame`].
    pub fn bp(&self) -> usize {
        self.bp
    }

    /// Returns the is runnable of this [`Frame`].
    pub fn is_runnable(&self) -> bool {
        self.ic < self.cf.fun.instructions.length()
    }

    /// Returns a reference to the get closure ref of this [`Frame`].
    pub fn get_closure_ref(&self) -> &ClosureFunction {
        &self.cf
    }

    /// Create String from `&self`
    pub fn to_string(&self) -> String {
        let mut buf = String::new();
        buf += &format!("<FRAME> IC : {}, BP : {}\n", self.ic, self.bp);
        buf += "\nINSTRUCTIONS\n";
        buf += &self.cf.fun.instructions.to_string_with_highlight(self.ic);

        buf
    }
}
