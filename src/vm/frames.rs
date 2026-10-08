//! Frames: a body with its locals, or a native in progress.
use super::{pending::Pending, Return};
use crate::{ir, mir, value::Value};
use alloc::vec::Vec;

/// A frame of the run: a body being stepped, or a native that is
/// waiting on a call it made.
#[derive(Debug)]
pub enum Frame {
    Body(BodyFrame),
    Native(NativeFrame),
}

/// A body in progress.
#[derive(Debug)]
pub struct BodyFrame {
    pub body: usize,
    pub block: usize,
    /// The next statement; the terminator when past the last.
    pub statement: usize,
    pub locals: Vec<Value>,
    /// What a closure of the body captured, read by `Operand::Capture`.
    pub captures: Vec<Value>,
    /// The witnesses of the body's hidden parameters, in its order.
    pub meanings: Vec<ir::Type>,
    /// Where the body's result goes.
    pub(super) back: Return,
}

/// A native waiting on a call it made; `node` is the call site, for a
/// Fault's span.
#[derive(Debug)]
pub struct NativeFrame {
    pub node: usize,
    pub(super) pending: Pending,
    pub(super) back: Return,
}

impl BodyFrame {
    /// A frame entering `body` with its spots filled by `args`.
    pub(super) fn new(
        program: &mir::Program,
        body: usize,
        args: Vec<Value>,
        captures: Vec<Value>,
        meanings: Vec<ir::Type>,
        back: Return,
    ) -> Self {
        let shape = &program.bodies[body];
        debug_assert_eq!(args.len(), shape.arity, "a body takes its spots");
        let mut locals = alloc::vec![Value::Unit; shape.locals.len()];
        for (spot, arg) in args.into_iter().enumerate() {
            locals[spot + 1] = arg;
        }
        Self {
            body,
            block: 0,
            statement: 0,
            locals,
            captures,
            meanings,
            back,
        }
    }
}
