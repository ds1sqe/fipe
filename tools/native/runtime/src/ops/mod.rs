//! The VM operations the generated code calls, copied one family per
//! file from `src/vm` (`COPIED.md` keeps each source and its hash), over
//! the run's state: what `vm::Machine` keeps besides its frames, which
//! are the machine stack's here.
mod budget;
mod calls;
mod fits;
mod natives;
mod slots;

pub use calls::call;
pub use slots::{assign, field, switch_key};

use crate::host::Host;
use fipe::{
    mir::{self, Program, Statement},
    value::{Prelude, Reason, Value},
    vm::Fault,
};

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
    pub program: Program,
    pub prelude: Prelude,
    natives: natives::Natives,
    /// The units each run may spend; `None` is unbounded.
    pub budget: Option<u64>,
    pub charged: u64,
    /// The Fault a status 1 stands for.
    pub fault: Option<Fault>,
    /// Each main expression line's value, with the line's IR node.
    pub lines: Vec<(usize, Value)>,
    /// The `with` keys and values open, outermost first.
    pub environment: Vec<(usize, Value)>,
    pub host: Host,
}

impl State {
    pub fn new(program: Program, budget: Option<u64>, host: Host) -> Self {
        let tables = program.tables.borrow();
        let prelude = Prelude::new(&tables);
        let natives = natives::Natives::new(&tables);
        drop(tables);
        Self {
            program,
            prelude,
            natives,
            budget,
            charged: 0,
            fault: None,
            lines: Vec::new(),
            environment: Vec::new(),
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

    /// How many operands the generated code reads for the site: an
    /// assignment's fields or arguments, a call's arguments.
    ///
    /// # Panics
    ///
    /// At a site with none, a defect of the lowering.
    #[must_use]
    pub fn operands(&self, site: Site) -> usize {
        let block = self.block(site);
        if site.index == block.statements.len() {
            let mir::Terminator::Call { args, .. } = &block.terminator else {
                unreachable!("a call site")
            };
            return args.len();
        }
        match self.statement(site) {
            Statement::Assign(_, mir::Rvalue::Construct { fields, .. }) => {
                fields.len()
            }
            Statement::Assign(_, mir::Rvalue::Native { args, .. }) => {
                args.len()
            }
            other => unreachable!("no operands read for {other:?}"),
        }
    }

    fn statement(&self, site: Site) -> &Statement {
        &self.block(site).statements[site.index]
    }

    /// Keep the Fault of `reason` at the site's node; the status 1
    /// the generated code returns stands for it.
    fn faulted(&mut self, site: Site, reason: Reason) -> Faulted {
        self.fault = Some(fault(&self.program, self.node(site), reason));
        Faulted
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

    /// A block coverage excluded was reached: a switch without the case.
    pub fn unreachable(&mut self, site: Site) -> Faulted {
        self.faulted(site, Reason::Unreachable)
    }
}

/// A Fault at a node, located in the node's source (`vm.rs`).
fn fault(program: &Program, node: usize, reason: Reason) -> Fault {
    let tables = program.tables.borrow();
    let node = &tables.nodes[node];
    Fault {
        span: node.span,
        namespace: node.namespace,
        reason,
    }
}
