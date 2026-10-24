//! The VM operations the generated code calls, copied one family per
//! file from `src/vm` (`COPIED.md` keeps each source and its hash), over
//! the run's state: what `vm::Machine` keeps besides its frames, which
//! are the machine stack's here.
//!
//! An operation that calls back into the program (a body, a function
//! value, a written member) enters machine code through
//! `abi::invoke`, which runs rt exports of its own; so no operation
//! holds the state across a call back: each reads what it needs from a
//! [`Ctx`] of shared parts, and borrows the state only for a moment.
mod budget;
mod calls;
mod casts;
mod dispatch;
mod entries;
mod fits;
mod higher;
mod immediate;
mod natives;
use fipe_native_core::operands;
mod predicates;
mod slots;
mod walk;

pub use calls::{call, init};
pub use entries::enter;
pub use predicates::branch;
pub use slots::{assign, field, global, switch_key};

use crate::{host::Host, start};
use fipe::{
    ir,
    mir::{self, Program, Statement},
    value::{Prelude, Reason, Value},
    vm::Fault,
};
use std::{cell::Ref, rc::Rc};

/// A run that Faulted; the Fault is in [`State::fault`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Faulted;

/// A statement or terminator of the program, by body, block and
/// position; the terminator's position is the statement count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    pub body: usize,
    pub block: usize,
    pub index: usize,
}

/// One run of the program the executable carries.
pub struct State {
    pub program: Rc<Program>,
    prelude: Rc<Prelude>,
    natives: Rc<natives::Natives>,
    /// The units each run may spend; `None` is unbounded.
    pub budget: Option<u64>,
    pub charged: u64,
    /// The Fault a status 1 stands for.
    pub fault: Option<Fault>,
    /// Each main expression line's value, with the line's IR node.
    pub lines: Vec<(usize, Value)>,
    /// The `with` keys and values open, outermost first.
    pub environment: Vec<(usize, Value)>,
    /// Each value definition's stored value, by definition index.
    globals: Vec<Option<Value>>,
    pub host: Host,
}

impl State {
    pub fn new(program: Program, budget: Option<u64>, host: Host) -> Self {
        let tables = program.tables.borrow();
        let prelude = Rc::new(Prelude::new(&tables));
        let natives = Rc::new(natives::Natives::new(&tables));
        let globals = vec![None; tables.definitions.len()];
        drop(tables);
        Self {
            program: Rc::new(program),
            prelude,
            natives,
            budget,
            charged: 0,
            fault: None,
            lines: Vec::new(),
            environment: Vec::new(),
            globals,
            host,
        }
    }

    fn block(&self, site: Site) -> &mir::Block {
        &self.program.bodies[site.body].blocks[site.block]
    }

    fn node(&self, site: Site) -> usize {
        self.block(site).nodes[site.index]
    }

    /// The site of the block's terminator.
    #[must_use]
    pub fn terminator(&self, site: Site) -> Site {
        let index = self.block(site).statements.len();
        Site { index, ..site }
    }

    /// How many operands the generated code reads for the site, by
    /// `operands`.
    ///
    /// # Panics
    ///
    /// At a site with none, a defect of the lowering.
    #[must_use]
    pub fn operands(&self, site: Site) -> usize {
        let block = self.block(site);
        if site.index == block.statements.len() {
            let mir::Terminator::Call { callee, args, .. } = &block.terminator
            else {
                unreachable!("a call site")
            };
            return operands::call(callee, args);
        }
        match self.statement(site) {
            Statement::Assign(_, rvalue) => operands::rvalue(rvalue)
                .unwrap_or_else(|| unreachable!("no operands for {rvalue:?}"))
                .len(),
            other => unreachable!("no operands read for {other:?}"),
        }
    }

    /// How many witnesses a frame of `body` has.
    #[must_use]
    pub fn hidden(&self, body: usize) -> usize {
        self.program.bodies[body].hidden.len()
    }

    fn statement(&self, site: Site) -> &Statement {
        &self.block(site).statements[site.index]
    }

    /// `Charge`: pay the statement's nodes.
    ///
    /// # Errors
    ///
    /// When the budget cannot pay; the Fault is kept.
    pub fn charge(&mut self, site: Site) -> Result<(), Faulted> {
        let Statement::Charge(nodes) = self.statement(site) else {
            unreachable!("a charge site")
        };
        let nodes = nodes.clone();
        match self.pay_nodes(&nodes) {
            Ok(()) => Ok(()),
            Err(fault) => {
                self.fault = Some(fault);
                Err(Faulted)
            }
        }
    }

    /// `Line`: keep a main line's value; its node is the line.
    pub fn line(&mut self, site: Site, value: Value) {
        let node = self.node(site);
        self.lines.push((node, value));
    }

    /// `Store`: write a value definition's global.
    pub fn store(&mut self, site: Site, value: Value) {
        let Statement::Store { definition, .. } = *self.statement(site) else {
            unreachable!("a store site")
        };
        self.globals[definition] = Some(value);
    }

    /// `Push`: open the child environment of a `with`.
    pub fn push(&mut self, site: Site, value: Value) {
        let Statement::Push { key, .. } = *self.statement(site) else {
            unreachable!("a push site")
        };
        self.environment.push((key, value));
    }

    /// `Pop`: close the environment the matching `Push` opened.
    pub fn pop(&mut self) {
        self.environment.pop();
    }

    /// A block coverage excluded was reached: a switch without the
    /// case, or an `Unreachable` terminator.
    pub fn unreachable(&mut self, site: Site) -> Faulted {
        let node = self.node(site);
        self.fault = Some(fault(&self.program, node, Reason::Unreachable));
        Faulted
    }
}

/// The shared parts of the run an operation reads: the program, the
/// prelude's meanings and the natives.
#[derive(Clone)]
pub struct Ctx {
    program: Rc<Program>,
    prelude: Rc<Prelude>,
    natives: Rc<natives::Natives>,
}

impl Ctx {
    /// The run's.
    fn get() -> Self {
        start::with(|state| Self {
            program: Rc::clone(&state.program),
            prelude: Rc::clone(&state.prelude),
            natives: Rc::clone(&state.natives),
        })
    }

    fn tables(&self) -> Ref<'_, ir::Tables> {
        self.program.tables.borrow()
    }

    fn node(&self, site: Site) -> usize {
        self.program.bodies[site.body].blocks[site.block].nodes[site.index]
    }
}

/// A frame of machine code as the runtime sees it: its body and its
/// witnesses, read from the `hidden` slots it was entered with.
struct Frame {
    body: usize,
    meanings: Vec<Rc<fipe::value::Meaning>>,
}

impl Frame {
    fn new(body: usize, hidden: &[Value]) -> Self {
        let meanings = hidden
            .iter()
            .map(|witness| match witness {
                Value::Meaning(ty) => Rc::clone(ty),
                other => unreachable!("a witness, not {other:?}"),
            })
            .collect();
        Self { body, meanings }
    }

    /// A type with the frame's witnesses in place of its parameters
    /// (`Machine::substitute`).
    fn substitute(&self, ctx: &Ctx, ty: &ir::Type) -> ir::Type {
        let body = &ctx.program.bodies[self.body];
        dispatch::frame_type(&ctx.tables(), ty, &body.hidden, &self.meanings)
    }

    /// The type of a local, witnesses in place (`Machine::local_type`).
    fn local_type(&self, ctx: &Ctx, local: usize) -> ir::Type {
        self.substitute(ctx, &ctx.program.bodies[self.body].locals[local].ty)
    }
}

/// Apply the run's value, the function its last line made, to the
/// host input, at that line: the result; a Fault is kept, as for
/// status 1.
pub fn apply(
    function: &Value,
    input: Value,
    line: usize,
) -> Result<Value, Faulted> {
    calls::call_value(&Ctx::get(), function, vec![input], line)
}

/// The name of `ty` when `value` has not its shape.
pub fn misfit(value: &Value, ty: &ir::Type) -> Result<Option<String>, Reason> {
    let ctx = Ctx::get();
    let tables = ctx.tables();
    let matched = fits::fits(value, ty, &tables, &ctx.prelude, &mut |units| {
        start::with(|state| state.pay(units))
    })?;
    Ok((!matched).then(|| fits::named(ty, &tables)))
}

/// Keep the Fault of `reason` at `node`; the status 1 the generated
/// code returns stands for it.
fn fail(node: usize, reason: Reason) -> Faulted {
    start::with(|state| {
        state.fault = Some(fault(&state.program, node, reason));
    });
    Faulted
}

/// Pay `units` for the operation at `node`, or Fault `Budget` there.
fn pay(units: u64, node: usize) -> Result<(), Faulted> {
    start::with(|state| state.pay(units)).map_err(|reason| fail(node, reason))
}

/// A Fault at a node, located in the node's source (`vm.rs`).
pub fn fault(program: &Program, node: usize, reason: Reason) -> Fault {
    let tables = program.tables.borrow();
    let node = &tables.nodes[node];
    Fault {
        span: node.span,
        namespace: node.namespace,
        reason,
    }
}
