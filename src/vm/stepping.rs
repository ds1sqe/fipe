//! A stepped run: the same steps as `run`, handed out as stops before
//! each statement and terminator and after each write.
use super::{Fault, Frame, Input, InputError, Machine, Stop};
use crate::{
    embed::{Outcome, Reply},
    mir,
    mir::Statement,
    value::Value,
};

impl Machine {
    /// Begin a stepped run of a program, as `run` would run it, without
    /// stepping yet: [`Machine::next_stop`] advances it.
    pub fn begin(&mut self, program: &mir::Program) {
        self.open(program);
        self.stepping = true;
    }

    /// Begin a stepped run as [`Machine::run_with_input`] would run it:
    /// the function the last line makes is applied to `input` at the
    /// end, and input without the shape of its spot Faults `Host` at
    /// that line at the first stop.
    ///
    /// # Errors
    /// [`InputError`], before any effect, when the last line is not a
    /// function of one open spot.
    pub fn begin_with_input(
        &mut self,
        program: &mir::Program,
        input: Value,
    ) -> Result<(), InputError> {
        {
            let tables = program.tables.borrow();
            super::borrowed_input_spot(program, &tables)?;
        }
        let line = program.bodies[program.main].node;
        self.begin(program);
        let program = self.joined();
        if let Err(reason) = self.admit_input(&program, &input) {
            self.end();
            self.queue(Some(Outcome::Done(Err(Self::fault(
                &program, line, reason,
            )))));
            return Ok(());
        }
        self.input = Some(Input::Owed { value: input, line });
        Ok(())
    }

    /// Advance a stepped run to its next stop.
    ///
    /// # Panics
    /// When no stepped run is in progress or a host reply is owed, a
    /// driver defect; on a checker or compiler invariant broken by the
    /// program, a crate defect.
    pub fn next_stop(&mut self) -> Stop {
        let program = self.joined();
        loop {
            if let Some(stop) = self.stops.pop_front() {
                return stop;
            }
            assert!(self.stepping, "no stepped run is in progress");
            assert!(self.waiting.is_none(), "a host reply is owed");
            if self.frames.is_empty() {
                self.enter_next(&program);
            }
            if let Some(node) = self.announce(&program) {
                return Stop::Before(node);
            }
            let assigned = self.assigned(&program);
            let queued = self.stops.len();
            let flow = self.step(&program);
            let settled = self.settle(&program, flow);
            if settled.is_none() {
                if let Some((depth, dest, node)) = assigned {
                    if self.frames.len() == depth && self.stops.len() == queued
                    {
                        let value = self.top().locals[dest].clone();
                        self.stops.push_back(Stop::After(node, value));
                    }
                }
            }
            self.queue(settled);
        }
    }

    /// Answer the host request a stepped run stopped on; the next
    /// [`Machine::next_stop`] continues from it. A value goes where the
    /// call wanted it; a failure, or a value that does not have the
    /// shape of the type the call declared, Faults `Host` at the call.
    ///
    /// # Panics
    /// When no host request is owed, or on a reply of another kind.
    pub fn answer(&mut self, reply: Reply) {
        let program = self.joined();
        let flow = self.replied(&program, reply);
        let settled = self.settle(&program, flow);
        self.queue(settled);
    }

    /// Hand out what a step ended with, after the stops it queued.
    fn queue(&mut self, settled: Option<Outcome<Result<Value, Fault>>>) {
        match settled {
            None => {}
            Some(Outcome::Request(request)) => {
                self.stops.push_back(Stop::Request(request));
            }
            Some(Outcome::Done(result)) => {
                self.stepping = false;
                self.stops.push_back(Stop::Done(result));
            }
        }
    }

    /// The node the top body frame stops before, unless it stopped
    /// there already or the next statement is a charge.
    fn announce(&mut self, program: &mir::Program) -> Option<usize> {
        let Some(Frame::Body(frame)) = self.frames.last_mut() else {
            return None;
        };
        let position = (frame.block, frame.statement);
        let block = &program.bodies[frame.body].blocks[frame.block];
        let charge = matches!(
            block.statements.get(frame.statement),
            Some(Statement::Charge(_))
        );
        if charge || frame.announced == Some(position) {
            return None;
        }
        frame.announced = Some(position);
        Some(block.nodes[frame.statement])
    }

    /// The frame depth, destination and node of the assignment the top
    /// body frame runs next, if that is what it runs.
    fn assigned(
        &self,
        program: &mir::Program,
    ) -> Option<(usize, usize, usize)> {
        let Some(Frame::Body(frame)) = self.frames.last() else {
            return None;
        };
        let block = &program.bodies[frame.body].blocks[frame.block];
        match block.statements.get(frame.statement) {
            Some(Statement::Assign(dest, _)) => {
                Some((self.frames.len(), *dest, block.nodes[frame.statement]))
            }
            _ => None,
        }
    }

    /// A value landing in the top body frame from a call or a started
    /// native: the stop after the node that made it. The frame still
    /// stands where it made the call; `(block, statement)` is where it
    /// continues, its own position when an assignment made it.
    pub(super) fn landed(
        &mut self,
        program: &mir::Program,
        block: usize,
        statement: usize,
        value: &Value,
    ) {
        let Some(Frame::Body(frame)) = self.frames.last() else {
            return;
        };
        let current = &program.bodies[frame.body].blocks[frame.block];
        let node = if (block, statement) == (frame.block, frame.statement)
            && statement > 0
        {
            current.nodes[statement - 1]
        } else {
            *current.nodes.last().expect("a terminator's node")
        };
        self.stops.push_back(Stop::After(node, value.clone()));
    }
}

impl Machine {
    /// Push the stage's next body as a fresh frame.
    pub(super) fn enter_next(&mut self, program: &mir::Program) {
        let body = self.next_body(program);
        let frame = super::BodyFrame::new(
            program,
            body,
            alloc::vec::Vec::new(),
            alloc::vec::Vec::new(),
            alloc::vec::Vec::new(),
            super::Return::Finish,
        );
        self.frames.push(Frame::Body(frame));
    }
}
