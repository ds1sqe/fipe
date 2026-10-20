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
mod budget;
mod calls;
mod casts;
mod defaults;
mod dispatch;
mod drive;
mod entries;
mod frames;
mod globals;
mod immediate;
mod merge;
mod natives;
mod order;
mod pending;
mod predicates;
mod step;
mod stepping;
mod walk;

use crate::{
    embed::{Outcome, Reply, Request},
    ir, mir,
    session::Withdrawn,
    token::Span,
    value::{Prelude, Reason, Value},
};
use alloc::{collections::VecDeque, rc::Rc, vec::Vec};
pub use frames::{BodyFrame, Frame, NativeFrame};
pub use natives::Natives;

/// A run that ended: the reason, at the node whose span this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// The node's span, in the source `namespace` names.
    pub span: Span,
    /// The source the span is in, by `tables.namespaces`: `main` or
    /// `line N` for a session's lines, a file module's stem, a host
    /// module's name, `commands`, or `prelude`.
    pub namespace: usize,
    pub reason: Reason,
}

/// What a run observably did.
///
/// Every backend must reproduce it from the same unit and host replies
/// on a run no budget stops (`09-host.md` §Backend equivalence); where
/// a budget stops a run, and what it charged, are not part of it. A
/// host driver assembles it: the success and error output are the
/// bytes its sink policy wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub result: Result<Value, Fault>,
    pub trace: Vec<(Request, Reply)>,
    pub success: Vec<u8>,
    pub error: Vec<u8>,
}

/// Where a stepped run stopped ([`Machine::begin`]).
///
/// Stops are not observations: a stepped run makes the same host
/// requests, in the same order, and charges the same units as `run`.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    /// The top body frame is about to run the statement or terminator
    /// of IR node `node`; a block's charge is not a stop.
    Before(usize),
    /// The statement or call of IR node `node` wrote this value into
    /// its frame: an assignment, or a call's result landing in its
    /// destination. A marked pipe's or `||`'s result is one, whichever
    /// side it took.
    After(usize, Value),
    /// The run waits on a host request; [`Machine::answer`] replies.
    Request(Request),
    /// The run ended with its value or Fault.
    Done(Result<Value, Fault>),
}

/// Which body of a run is in progress: a module's init, by its
/// position in `Program.inits`, or the main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Init(usize),
    Main,
}

/// The machine of one session.
#[derive(Debug)]
pub struct Machine {
    /// Each value definition's stored value, by definition index.
    globals: Vec<Option<Value>>,
    /// Which modules ran their init body, by module index.
    initialized: Vec<bool>,
    /// The units each run may spend; `None` is unbounded.
    budget: Option<u64>,
    charged: u64,
    frames: Vec<Frame>,
    /// The natives, chosen once per tables; rebuilt when the tables grew.
    natives: Option<(usize, Rc<Natives>)>,
    /// The prelude's meanings, found once per tables.
    prelude: Option<(usize, Rc<Prelude>)>,
    /// The body of the run in progress; `None` between runs.
    stage: Option<Stage>,
    /// Where an owed host reply goes, and the call's node.
    waiting: Option<(Return, usize)>,
    /// The `with` keys and values open, outermost first.
    environment: Vec<(usize, Value)>,
    /// Every program run so far, joined: the bodies frames and function
    /// values index.
    program: Option<Rc<mir::Program>>,
    /// Whether the run in progress was begun stepped.
    stepping: bool,
    /// The stops a stepped run reached and has not handed out yet.
    stops: VecDeque<Stop>,
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
    /// Continue the body frame below where it stands; the value is
    /// dropped (a passed entry).
    Continue,
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
    /// A host call: the run waits for its reply.
    Request(Request),
}

impl Machine {
    /// A machine with a budget of charge units for each run it makes,
    /// or none for unbounded runs; a run that cannot pay Faults
    /// `Budget` at the first unpaid node.
    #[must_use]
    pub const fn new(budget: Option<u64>) -> Self {
        Self {
            globals: Vec::new(),
            initialized: Vec::new(),
            budget,
            charged: 0,
            frames: Vec::new(),
            natives: None,
            prelude: None,
            stage: None,
            waiting: None,
            environment: Vec::new(),
            program: None,
            stepping: false,
            stops: VecDeque::new(),
        }
    }

    /// The charge units of the run in progress, or of the last run.
    #[must_use]
    pub const fn charged(&self) -> u64 {
        self.charged
    }

    /// The budget of each run; `None` is unbounded.
    #[must_use]
    pub const fn budget(&self) -> Option<u64> {
        self.budget
    }

    /// The budget of the runs to come; `None` is unbounded.
    pub const fn set_budget(&mut self, budget: Option<u64>) {
        self.budget = budget;
    }

    /// Forget what a session line withdrawn after its run Faulted
    /// added: its values and the module inits it ran, so that giving
    /// the line again runs them again.
    pub fn withdraw(&mut self, withdrawn: &Withdrawn) {
        self.globals.truncate(withdrawn.definitions);
        self.initialized.truncate(withdrawn.modules);
        if let Some(program) = self.program.as_mut() {
            merge::forget(
                Rc::make_mut(program),
                withdrawn.definitions,
                withdrawn.tests,
            );
        }
        // Both are found per tables length, which the next line may
        // reach again with other contents.
        self.natives = None;
        self.prelude = None;
    }

    /// The frames of the run in progress, the innermost last; empty
    /// between runs.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// The values the runs so far stored for value definitions, by
    /// index into `Tables.definitions`, in that order; a definition not
    /// yet run, or withdrawn, is absent. Reading changes no run.
    pub fn globals(&self) -> impl Iterator<Item = (usize, &Value)> {
        self.globals
            .iter()
            .enumerate()
            .filter_map(|(definition, value)| {
                Some((definition, value.as_ref()?))
            })
    }

    /// Run a program: each module's init body not yet run, then the
    /// main; its value, the Fault that ended it, or the host request it
    /// waits on.
    ///
    /// # Panics
    /// On a checker or compiler invariant broken by the program, a
    /// crate defect.
    pub fn run(
        &mut self,
        program: &mir::Program,
    ) -> Outcome<Result<Value, Fault>> {
        let program = self.open(program);
        self.advance(&program)
    }

    /// Join a run's program, size the machine to it and reset the run.
    fn open(&mut self, program: &mir::Program) -> Rc<mir::Program> {
        merge::merge(&mut self.program, program);
        let program = self.joined();
        self.prepare(&program);
        self.charged = 0;
        self.frames.clear();
        self.environment.clear();
        self.waiting = None;
        self.stage = Some(Stage::Init(0));
        self.stepping = false;
        self.stops.clear();
        program
    }

    /// Every program run so far, joined: the bodies that frames and
    /// function values index, renumbered from each run's program.
    #[must_use]
    pub fn program(&self) -> Option<&mir::Program> {
        self.program.as_deref()
    }

    fn joined(&self) -> Rc<mir::Program> {
        Rc::clone(self.program.as_ref().expect("a program ran"))
    }

    /// Answer the owed host request and continue the run, with the
    /// program `run` was given: the value goes where the call wanted
    /// it; a failure Faults `Host` at the call.
    ///
    /// # Panics
    /// When no host request is owed, or on a reply of another kind.
    pub fn reply(
        &mut self,
        program: &mir::Program,
        reply: Reply,
    ) -> Outcome<Result<Value, Fault>> {
        let (back, node) = self.waiting.take().expect("a host request is owed");
        let joined = self.joined();
        debug_assert!(Rc::ptr_eq(&joined.tables, &program.tables));
        let program = &*joined;
        let flow = match reply {
            Reply::Value(Ok(value)) => self.deliver(program, value, back),
            Reply::Value(Err(message)) => {
                Err(Self::fault(program, node, Reason::Host(message)))
            }
            Reply::File(_) => panic!("a host call takes a value reply"),
        };
        self.settle(program, flow)
            .unwrap_or_else(|| self.advance(program))
    }

    /// Step the run until it ends or asks the host; each body of the
    /// stage in turn, from an empty frame stack.
    fn advance(
        &mut self,
        program: &mir::Program,
    ) -> Outcome<Result<Value, Fault>> {
        loop {
            if self.frames.is_empty() {
                self.enter_next(program);
            }
            let flow = self.step(program);
            if let Some(outcome) = self.settle(program, flow) {
                return outcome;
            }
        }
    }

    /// The body the stage runs next: the first init not yet run from
    /// the stage's position, else the main.
    fn next_body(&mut self, program: &mir::Program) -> usize {
        if let Some(Stage::Init(position)) = self.stage {
            let next = program.inits[position..]
                .iter()
                .position(|(module, _)| !self.initialized[*module]);
            if let Some(offset) = next {
                self.stage = Some(Stage::Init(position + offset));
                return program.inits[position + offset].1;
            }
            self.stage = Some(Stage::Main);
        }
        program.main
    }

    /// What a step means for the run: keep going (`None`), or the
    /// outcome to return.
    fn settle(
        &mut self,
        program: &mir::Program,
        flow: Result<Flow, Fault>,
    ) -> Option<Outcome<Result<Value, Fault>>> {
        match flow {
            Ok(Flow::Continue) => None,
            Ok(Flow::Request(request)) => Some(Outcome::Request(request)),
            Ok(Flow::Done(value)) => {
                self.frames.clear();
                if let Some(Stage::Init(position)) = self.stage {
                    let (module, _) = program.inits[position];
                    self.initialized[module] = true;
                    self.stage = Some(Stage::Init(position + 1));
                    return None;
                }
                self.end();
                Some(Outcome::Done(Ok(value)))
            }
            Err(fault) => {
                self.end();
                Some(Outcome::Done(Err(fault)))
            }
        }
    }

    fn end(&mut self) {
        self.frames.clear();
        self.environment.clear();
        self.waiting = None;
        self.stage = None;
    }

    /// Hand a host call to the driver; the run waits for `reply`.
    pub(crate) fn host(
        &mut self,
        declaration: usize,
        arguments: Vec<Value>,
        back: Return,
        node: usize,
    ) -> Flow {
        self.waiting = Some((back, node));
        Flow::Request(Request::Call {
            declaration,
            arguments,
            environment: self.environment.clone(),
        })
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

    pub(crate) fn natives(&self) -> Rc<Natives> {
        Rc::clone(&self.natives.as_ref().expect("prepared").1)
    }

    pub(crate) fn prelude(&self) -> Rc<Prelude> {
        Rc::clone(&self.prelude.as_ref().expect("prepared").1)
    }

    /// A Fault at a node, located in the node's source.
    pub(crate) fn fault(
        program: &mir::Program,
        node: usize,
        reason: Reason,
    ) -> Fault {
        let tables = program.tables.borrow();
        let node = &tables.nodes[node];
        Fault {
            span: node.span,
            namespace: node.namespace,
            reason,
        }
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
