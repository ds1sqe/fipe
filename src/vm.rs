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
mod fits;
mod frames;
mod globals;
mod immediate;
mod merge;
mod natives;
mod order;
mod pending;
mod predicates;
mod resource;
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

/// Why a run cannot take host input: its last top-level line is not a
/// function of one open spot.
///
/// `spots` is how many open spots the line
/// has: `0` for a value that is not a function, a binding, or no line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputError {
    pub spots: usize,
}

/// The type of the spot host input enters: the program's last
/// top-level line must be a function of one open spot
/// (`09-host.md` §Input, output and observation).
///
/// # Errors
/// [`InputError`] with the line's open spots when it has another
/// number of them.
pub fn input_spot(program: &mir::Program) -> Result<ir::Type, InputError> {
    let tables = program.tables.borrow();
    borrowed_input_spot(program, &tables).cloned()
}

fn borrowed_input_spot<'a>(
    program: &'a mir::Program,
    tables: &'a ir::Tables,
) -> Result<&'a ir::Type, InputError> {
    let mut ty = &program.bodies[program.main].locals[0].ty;
    let mut spots = 0;
    let mut spot = None;
    loop {
        match ty {
            ir::Type::Session(index) => {
                match tables.session_meanings.get(*index) {
                    Some(Some(fixed)) => ty = fixed,
                    _ => break,
                }
            }
            ir::Type::Function { parameter, result } => {
                spots += 1;
                spot.get_or_insert(parameter);
                ty = result;
            }
            _ => break,
        }
    }
    match spot {
        Some(spot) if spots == 1 => Ok(spot.as_ref()),
        _ => Err(InputError { spots }),
    }
}

/// Host input over the run in progress: owed to the function the last
/// line of node `line` makes, or applied to it.
#[derive(Debug)]
enum Input {
    Owed { value: Value, line: usize },
    Applied { line: usize },
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
    /// The host call an owed reply answers.
    waiting: Option<Owed>,
    /// The `with` keys and values open, outermost first.
    environment: Vec<(usize, Value)>,
    /// Every program run so far, joined: the bodies frames and function
    /// values index.
    program: Option<Rc<mir::Program>>,
    /// Whether the run in progress was begun stepped.
    stepping: bool,
    /// The stops a stepped run reached and has not handed out yet.
    stops: VecDeque<Stop>,
    /// Each main expression line's value in the run in progress or the
    /// last one, with the line's IR node.
    lines: Vec<(usize, Value)>,
    /// The host input of the run in progress, if it takes one.
    input: Option<Input>,
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
    /// Publish guarded spots to the existing callee without a source stop.
    Arguments {
        body: usize,
        arity: usize,
        node: usize,
    },
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

/// A host call waiting on its reply: where the value goes, the call's
/// node, and the type its declaration gives the value at this call.
#[derive(Debug)]
pub(crate) struct Owed {
    back: Return,
    expected: resource::Frozen,
    refused: bool,
    site: Fault,
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
            lines: Vec::new(),
            input: None,
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

    /// Stop unfinished execution without invoking the host or undoing effects.
    ///
    /// Returns true only when an active run was abandoned. False leaves an
    /// already queued stepped terminal result intact: consume that `Stop::Done`
    /// as success or Fault, not abandonment. Repeated abandonment is a no-op.
    /// Prior globals, module initialization, program, lines and charges remain;
    /// a session driver separately withdraws an abandoned line when needed.
    /// Invalidate the driver's pending-call association first and discard late
    /// replies before starting another run; this method does not identify replies.
    #[must_use]
    pub fn abandon(&mut self) -> bool {
        if self.stage.is_none() {
            return false;
        }
        self.end();
        self.stops.clear();
        self.stepping = false;
        true
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

    /// Run a program as [`Machine::run`] does, then apply the function
    /// its last line makes to host input: the run's value, and that
    /// line's in `lines`, is the function's result. Input without the
    /// shape of the spot's type ([`input_spot`]) Faults `Host` "the
    /// input is not a T" at that line before the run starts.
    ///
    /// # Errors
    /// [`InputError`], before any effect, when the last line is not a
    /// function of one open spot.
    ///
    /// # Panics
    /// As [`Machine::run`].
    pub fn run_with_input(
        &mut self,
        program: &mir::Program,
        input: Value,
    ) -> Result<Outcome<Result<Value, Fault>>, InputError> {
        {
            let tables = program.tables.borrow();
            borrowed_input_spot(program, &tables)?;
        }
        let line = program.bodies[program.main].node;
        let program = self.open(program);
        if let Err(reason) = self.admit_input(&program, &input) {
            self.end();
            return Ok(Outcome::Done(Err(Self::fault(&program, line, reason))));
        }
        self.input = Some(Input::Owed { value: input, line });
        Ok(self.advance(&program))
    }

    fn admit_input(
        &mut self,
        program: &mir::Program,
        input: &Value,
    ) -> Result<(), Reason> {
        let work = self.resource_work();
        let result = {
            let tables = program.tables.borrow();
            let spot = borrowed_input_spot(program, &tables)
                .expect("validated input arity");
            resource::Frozen::capture(spot, &tables, self.prelude(), &work)
                .and_then(|expected| {
                    if expected.fits(input, &tables, &work)? {
                        Ok(())
                    } else {
                        Err(expected.input_mismatch())
                    }
                })
        };
        self.pay(work.spent())?;
        result
    }

    /// Join a run's program, size the machine to it and reset the run.
    fn open(&mut self, program: &mir::Program) -> Rc<mir::Program> {
        if merge::merge(&mut self.program, program) {
            self.globals.clear();
            self.initialized.clear();
            self.natives = None;
            self.prelude = None;
        }
        let program = self.joined();
        self.prepare(&program);
        self.charged = 0;
        self.frames.clear();
        self.environment.clear();
        self.waiting = None;
        self.stage = Some(Stage::Init(0));
        self.stepping = false;
        self.stops.clear();
        self.lines.clear();
        self.input = None;
        program
    }

    /// Each main expression line's value, with the line's IR node, in
    /// line order: the run in progress or the last one. A line whose
    /// run ended before it is absent. Reading changes nothing.
    #[must_use]
    pub fn lines(&self) -> &[(usize, Value)] {
        &self.lines
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
    /// it; a failure, or a value that does not have the shape of the
    /// type the call declared, Faults `Host` at the call.
    ///
    /// # Panics
    /// When no host request is owed, or on a reply of another kind.
    pub fn reply(
        &mut self,
        program: &mir::Program,
        reply: Reply,
    ) -> Outcome<Result<Value, Fault>> {
        let joined = self.joined();
        debug_assert!(Rc::ptr_eq(&joined.tables, &program.tables));
        let program = &*joined;
        let flow = self.replied(program, reply);
        self.settle(program, flow)
            .unwrap_or_else(|| self.advance(program))
    }

    /// The owed call's reply delivered, or the Fault it makes.
    fn replied(
        &mut self,
        program: &mir::Program,
        reply: Reply,
    ) -> Result<Flow, Fault> {
        let Owed {
            back,
            expected,
            refused,
            mut site,
        } = self.waiting.take().expect("a host request is owed");
        match reply {
            Reply::Value(_) if refused => {
                site.reason = Reason::Budget;
                Err(site)
            }
            Reply::Value(Ok(value)) => {
                let work = self.resource_work();
                let fits =
                    expected.fits(&value, &program.tables.borrow(), &work);
                let paid = self.pay(work.spent());
                let reason = match paid.and(fits) {
                    Ok(true) => None,
                    Ok(false) => Some(expected.mismatch()),
                    Err(reason) => Some(reason),
                };
                if let Some(reason) = reason {
                    site.reason = reason;
                    return Err(site);
                }
                self.deliver(program, value, back)
            }
            Reply::Value(Err(message)) => {
                site.reason = Reason::Host(message);
                Err(site)
            }
            Reply::File(_) => panic!("a host call takes a value reply"),
        }
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
                match self.input.take() {
                    Some(Input::Owed { value: input, line }) => {
                        return self.apply_input(program, &value, input, line);
                    }
                    Some(Input::Applied { line }) => {
                        self.lines.push((line, value.clone()));
                    }
                    None => {}
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
        self.input = None;
    }

    /// Call the main's function value with the host input; its result
    /// takes the place of the function as the last line's value.
    fn apply_input(
        &mut self,
        program: &mir::Program,
        function: &Value,
        input: Value,
        line: usize,
    ) -> Option<Outcome<Result<Value, Fault>>> {
        self.input = Some(Input::Applied { line });
        let flow = self.call_value(
            program,
            function,
            alloc::vec![input],
            Return::Finish,
            line,
        );
        self.settle(program, flow)
    }

    /// Hand a host call to the driver; the run waits for `reply`, whose
    /// value must have the shape of `ty`, the call's result type.
    pub(crate) fn host(
        &mut self,
        program: &mir::Program,
        declaration: usize,
        arguments: Vec<Value>,
        ty: ir::Type,
        back: Return,
        node: usize,
    ) -> Result<Flow, Fault> {
        let ty = crate::value::resource::Forest(alloc::vec![ty]);
        let work = self.resource_work();
        let expected = resource::Frozen::capture(
            &ty.0[0],
            &program.tables.borrow(),
            self.prelude(),
            &work,
        );
        self.pay(work.spent())
            .map_err(|reason| Self::fault(program, node, reason))?;
        let expected =
            expected.map_err(|reason| Self::fault(program, node, reason))?;
        self.waiting = Some(Owed {
            back,
            expected,
            refused: false,
            site: Self::fault(program, node, Reason::Budget),
        });
        Ok(Flow::Request(Request::Call {
            declaration,
            arguments,
            environment: self.environment.clone(),
        }))
    }

    fn resource_work(&self) -> resource::Work {
        resource::Work::new(
            self.budget.map(|limit| limit.saturating_sub(self.charged)),
        )
    }

    /// The exact frozen direct resource result of the currently pending Host call.
    ///
    /// # Errors
    /// Returns `NoPendingCall` outside a Call or `NotResourceResult` for a
    /// composite/ordinary result. It never guesses a nested resource position.
    pub fn pending_resource_type(
        &self,
    ) -> Result<
        crate::value::resource::ResourceType,
        crate::value::resource::ResourceError,
    > {
        let owed = self
            .waiting
            .as_ref()
            .ok_or(crate::value::resource::ResourceError::NoPendingCall)?;
        if owed.refused {
            return Err(crate::value::resource::ResourceError::Budget);
        }
        owed.expected.direct()
    }

    /// Select a resource type from the same frozen expectation used to admit a reply.
    ///
    /// # Errors
    /// A bad ordinal/kind reports its zero-based path step. An expectation which
    /// cannot expose resources returns `NotResourceResult` before path validation.
    /// Budget refusal remains pending until reply/answer or abandonment; stop Host
    /// work and submit the normal reply to observe the captured Budget terminal.
    ///
    /// # Panics
    /// On an internal defect which removes the owed call during projection.
    pub fn pending_resource_type_at(
        &mut self,
        path: &[crate::value::resource::ResourceStep],
    ) -> Result<
        crate::value::resource::ResourceType,
        crate::value::resource::ResourceError,
    > {
        use crate::value::resource::ResourceError;
        let owed = self.waiting.as_ref().ok_or(ResourceError::NoPendingCall)?;
        if owed.refused {
            return Err(ResourceError::Budget);
        }
        if path.is_empty() {
            return owed.expected.direct();
        }
        let work = self.resource_work();
        let result = owed.expected.project(path, &work);
        let paid = self.pay(work.spent());
        if paid.is_err() || matches!(result, Err(ResourceError::Budget)) {
            self.waiting
                .as_mut()
                .expect("projection keeps the call pending")
                .refused = true;
            return Err(ResourceError::Budget);
        }
        result
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

    /// A type with the frame's witnesses, charged before resource-capable copies.
    pub(crate) fn substitute(
        &mut self,
        program: &mir::Program,
        ty: &ir::Type,
    ) -> Result<ir::Type, Reason> {
        self.substitute_charged(program, ty, false)
    }

    pub(super) fn substitute_charged(
        &mut self,
        program: &mir::Program,
        ty: &ir::Type,
        callable: bool,
    ) -> Result<ir::Type, Reason> {
        let budgeted = callable || program.tables.borrow().has_resources;
        let work = if budgeted {
            self.resource_work()
        } else {
            resource::Work::new(None)
        };
        let copied = {
            let Some(Frame::Body(frame)) = self.frames.last() else {
                unreachable!("body frame")
            };
            let body = &program.bodies[frame.body];
            let tables = program.tables.borrow();
            resource::materialize(
                ty,
                &body.hidden,
                &frame.meanings,
                &tables,
                &work,
                &mut Ok,
            )
        };
        if budgeted {
            self.pay(work.spent())?;
        }
        Ok(copied?.0.pop().expect("copied type"))
    }
}
