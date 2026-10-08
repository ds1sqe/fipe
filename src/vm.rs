//! The machine: an interpreter over [`mir::Program`], the first backend
//! and the oracle the others must equal.
//!
//! A frame per body with its locals, on a heap vector; no operand stack
//! and no recursion, so a deep recursion grows the frames, never the
//! Rust stack, and nothing of the machine is on the Rust stack between
//! two steps. A native that calls back into the program (`map`, a
//! written `show` met on a walk) is a state machine in a native frame,
//! driven by the same loop. One machine serves a session: its globals
//! are the accepted lines' values and each line's program indexes the
//! shared tables.
mod assign;
mod calls;
mod casts;
mod defaults;
mod dispatch;
mod drive;
mod frames;
mod globals;
mod immediate;
mod natives;
mod order;
mod pending;
mod predicates;
mod step;
mod walk;

use crate::{
    embed::{Outcome, Reply},
    ir, mir,
    token::Span,
    value::{Prelude, Reason, Value},
};
use alloc::{rc::Rc, vec::Vec};
pub use frames::{BodyFrame, Frame, NativeFrame};
pub use natives::Natives;

/// A run that ended: the reason, at the node whose span this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub span: Span,
    pub reason: Reason,
}

/// The machine of one session.
#[derive(Debug)]
pub struct Machine {
    /// Each value definition's stored value, by definition index.
    globals: Vec<Option<Value>>,
    /// Which modules ran their init body, by module index.
    initialized: Vec<bool>,
    budget: u64,
    charged: u64,
    frames: Vec<Frame>,
    /// The natives, chosen once per tables; rebuilt when the tables grew.
    natives: Option<(usize, Rc<Natives>)>,
    /// The prelude's meanings, found once per tables.
    prelude: Option<(usize, Rc<Prelude>)>,
}

/// What a frame's caller does with the returned value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Return {
    /// Write the local of the body frame below and continue there.
    Into {
        dest: usize,
        block: usize,
        statement: usize,
    },
    /// Continue the body frame below at `pass` or `fail` by the Bool.
    Branch { pass: usize, fail: usize },
    /// Resume the native frame below with the value.
    Resume,
    /// Store a global the body frame below read unset, then let it
    /// retry the step that read it.
    Store(usize),
    /// The run's result.
    Finish,
}

/// What a step decided.
pub(crate) enum Flow {
    /// Keep stepping.
    Continue,
    /// The last frame returned this value.
    Done(Value),
}

impl Machine {
    /// A machine with a budget of charge units for every run it makes
    /// (the budget's exhaustion Faults once the budget is enforced).
    #[must_use]
    pub const fn new(budget: u64) -> Self {
        Self {
            globals: Vec::new(),
            initialized: Vec::new(),
            budget,
            charged: 0,
            frames: Vec::new(),
            natives: None,
            prelude: None,
        }
    }

    /// The charge units counted so far across every run.
    #[must_use]
    pub const fn charged(&self) -> u64 {
        self.charged
    }

    #[must_use]
    pub const fn budget(&self) -> u64 {
        self.budget
    }

    /// The budget of the runs to come.
    pub const fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
    }

    /// The frames of the run in progress, the innermost last; empty
    /// between runs.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Run a program: each module's init body not yet run, then the
    /// main; its value, or the Fault that ended it.
    ///
    /// # Panics
    /// On a checker or compiler invariant broken by the program, a
    /// crate defect.
    pub fn run(
        &mut self,
        program: &mir::Program,
    ) -> Outcome<Result<Value, Fault>> {
        self.prepare(program);
        for (module, body) in program.inits.clone() {
            if self.initialized[module] {
                continue;
            }
            if let Err(fault) = self.execute(program, body) {
                return Outcome::Done(Err(fault));
            }
            self.initialized[module] = true;
        }
        Outcome::Done(self.execute(program, program.main))
    }

    /// Answer a host request and continue the run; no request is made
    /// until the host callee is wired (08.3), so none can be answered.
    ///
    /// # Panics
    /// Always, until then.
    #[allow(clippy::needless_pass_by_value)] // the reply is consumed once wired
    pub fn reply(&mut self, reply: Reply) -> Outcome<Result<Value, Fault>> {
        unreachable!("no host request was made: {reply:?}")
    }

    /// Size the globals and modules to the tables; refresh the natives
    /// and the prelude when the tables grew.
    fn prepare(&mut self, program: &mir::Program) {
        let tables = program.tables.borrow();
        if self.globals.len() < tables.definitions.len() {
            self.globals.resize(tables.definitions.len(), None);
        }
        if self.initialized.len() < tables.modules.len() {
            self.initialized.resize(tables.modules.len(), false);
        }
        let declarations = tables.declarations.len();
        if self.natives.as_ref().map(|(len, _)| *len) != Some(declarations) {
            self.natives = Some((declarations, Rc::new(Natives::new(&tables))));
        }
        let meanings = tables.meanings.len();
        if self.prelude.as_ref().map(|(len, _)| *len) != Some(meanings) {
            self.prelude = Some((meanings, Rc::new(Prelude::new(&tables))));
        }
    }

    /// Run one body with no arguments to its end.
    fn execute(
        &mut self,
        program: &mir::Program,
        body: usize,
    ) -> Result<Value, Fault> {
        debug_assert!(self.frames.is_empty());
        let frame = BodyFrame::new(
            program,
            body,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Return::Finish,
        );
        self.frames.push(Frame::Body(frame));
        let result = loop {
            match self.step(program) {
                Ok(Flow::Continue) => (),
                Ok(Flow::Done(value)) => break Ok(value),
                Err(fault) => break Err(fault),
            }
        };
        self.frames.clear();
        result
    }

    pub(crate) fn natives(&self) -> Rc<Natives> {
        Rc::clone(&self.natives.as_ref().expect("prepared").1)
    }

    pub(crate) fn prelude(&self) -> Rc<Prelude> {
        Rc::clone(&self.prelude.as_ref().expect("prepared").1)
    }

    /// The span of a node, for a Fault.
    pub(crate) fn span(program: &mir::Program, node: usize) -> Span {
        program.tables.borrow().nodes[node].span
    }

    /// A type with the frame's witnesses in place of its parameters.
    pub(crate) fn substitute(
        program: &mir::Program,
        frame: &BodyFrame,
        ty: &ir::Type,
    ) -> ir::Type {
        let body = &program.bodies[frame.body];
        let fixed = ty.substituted(&body.hidden, &frame.meanings);
        crate::compile::dispatch::fixed(&program.tables.borrow(), &fixed)
    }
}
