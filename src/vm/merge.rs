//! The bodies of every program a machine ran, in one program.
//!
//! Each session line is its own program, and a function value names its
//! body by index, so a value one line stored must find its body when a
//! later line calls it. The machine keeps one program of its own: each
//! run's bodies join it, renumbered, and a body an earlier run already
//! brought, a definition's, a test's or a global's, is not brought
//! again. A body's index there never changes while the tables are the
//! same.
use crate::mir::{
    Body, Callee, Operand, Program, Rvalue, Statement, Terminator,
};
use alloc::{rc::Rc, vec, vec::Vec};

/// Join `program` to the machine's program, starting a new one when the
/// tables differ; the result runs with `program`'s main and inits.
pub(super) fn merge(merged: &mut Option<Rc<Program>>, program: &Program) {
    let fresh = merged
        .as_ref()
        .is_none_or(|kept| !Rc::ptr_eq(&kept.tables, &program.tables));
    if fresh {
        *merged = Some(Rc::new(Program {
            tables: Rc::clone(&program.tables),
            bodies: Vec::new(),
            constants: Vec::new(),
            definitions: Vec::new(),
            tests: Vec::new(),
            inits: Vec::new(),
            globals: Vec::new(),
            main: 0,
        }));
    }
    let kept = Rc::make_mut(merged.as_mut().expect("a program"));
    Joining::new(kept, program).join();
}

/// Forget the definitions and tests from `definitions` and `tests` on:
/// a withdrawn line's, whose indices the next line may take again.
pub(super) fn forget(program: &mut Program, definitions: usize, tests: usize) {
    program.definitions.truncate(definitions);
    program.globals.truncate(definitions);
    program.tests.truncate(tests);
}

struct Joining<'a> {
    kept: &'a mut Program,
    program: &'a Program,
    /// Each body of `program` by its index in `kept`, once known.
    bodies: Vec<Option<usize>>,
    /// Each constant of `program` by its index in `kept`, once used.
    constants: Vec<Option<usize>>,
}

impl<'a> Joining<'a> {
    fn new(kept: &'a mut Program, program: &'a Program) -> Self {
        let size = program.definitions.len();
        if kept.definitions.len() < size {
            kept.definitions.resize(size, None);
            kept.globals.resize(size, None);
        }
        if kept.tests.len() < program.tests.len() {
            kept.tests.resize(program.tests.len(), None);
        }
        Self {
            kept,
            program,
            bodies: vec![None; program.bodies.len()],
            constants: vec![None; program.constants.len()],
        }
    }

    fn join(mut self) {
        let program = self.program;
        // A body the kept program holds already is the same body: the
        // tables are shared and a definition never changes.
        let known = |kept: &[Option<usize>], own: &[Option<usize>]| {
            own.iter()
                .zip(kept)
                .filter_map(|(own, kept)| Some(((*own)?, (*kept)?)))
                .collect::<Vec<_>>()
        };
        for (own, kept) in known(&self.kept.definitions, &program.definitions)
            .into_iter()
            .chain(known(&self.kept.tests, &program.tests))
            .chain(known(&self.kept.globals, &program.globals))
        {
            self.bodies[own] = Some(kept);
        }
        let mut order = Vec::new();
        let roots = program
            .definitions
            .iter()
            .chain(&program.tests)
            .chain(&program.globals)
            .filter_map(|body| *body)
            .chain(program.inits.iter().map(|(_, body)| *body))
            .chain([program.main]);
        let mut pending: Vec<usize> = roots.collect();
        while let Some(body) = pending.pop() {
            if self.bodies[body].is_some() {
                continue;
            }
            self.bodies[body] = Some(self.kept.bodies.len() + order.len());
            order.push(body);
            each_body(&program.bodies[body], |next| pending.push(next));
        }
        for body in order {
            let renumbered = self.renumbered(&program.bodies[body]);
            self.kept.bodies.push(renumbered);
        }
        let at = |body: usize, bodies: &[Option<usize>]| {
            bodies[body].expect("every root is joined")
        };
        for (map, own) in [
            (&mut self.kept.definitions, &program.definitions),
            (&mut self.kept.tests, &program.tests),
            (&mut self.kept.globals, &program.globals),
        ] {
            for (kept, own) in map.iter_mut().zip(own) {
                if kept.is_none() {
                    *kept = own.map(|body| at(body, &self.bodies));
                }
            }
        }
        self.kept.inits = program
            .inits
            .iter()
            .map(|(module, body)| (*module, at(*body, &self.bodies)))
            .collect();
        self.kept.main = at(program.main, &self.bodies);
    }

    /// A body of `program` with its body and constant indices in `kept`.
    fn renumbered(&mut self, body: &Body) -> Body {
        let mut body = body.clone();
        for block in &mut body.blocks {
            for statement in &mut block.statements {
                match statement {
                    Statement::Assign(_, rvalue) => self.rvalue(rvalue),
                    Statement::Store { value, .. }
                    | Statement::Enter { value, .. }
                    | Statement::Push { value, .. }
                    | Statement::Line(value) => self.operand(value),
                    Statement::Charge(_) | Statement::Pop => {}
                }
            }
            match &mut block.terminator {
                Terminator::Switch { value, .. }
                | Terminator::Test { value, .. } => self.operand(value),
                Terminator::Call { callee, args, .. } => {
                    match callee {
                        Callee::Body(index) => *index = self.body(*index),
                        Callee::Value(value)
                        | Callee::Dispatch {
                            witness: crate::mir::Witness::Value(value),
                            ..
                        } => self.operand(value),
                        Callee::Native(_)
                        | Callee::Host(_)
                        | Callee::Derived { .. }
                        | Callee::Dispatch { .. } => {}
                    }
                    for arg in args.iter_mut() {
                        self.operand(arg);
                    }
                }
                Terminator::Goto(_)
                | Terminator::Return
                | Terminator::Unreachable => {}
            }
        }
        body
    }

    fn rvalue(&mut self, rvalue: &mut Rvalue) {
        match rvalue {
            Rvalue::Use(value)
            | Rvalue::Repeat { value, .. }
            | Rvalue::Field(value, _)
            | Rvalue::Length(value)
            | Rvalue::Wrap { value, .. }
            | Rvalue::Cast { value, .. }
            | Rvalue::Convert { value, .. }
            | Rvalue::Pack { value, .. } => self.operand(value),
            Rvalue::Construct { fields: values, .. }
            | Rvalue::Tuple(values)
            | Rvalue::Array(values)
            | Rvalue::Native { args: values, .. } => {
                for value in values.iter_mut() {
                    self.operand(value);
                }
            }
            Rvalue::Closure { body, captures, .. } => {
                *body = self.body(*body);
                for value in captures.iter_mut() {
                    self.operand(value);
                }
            }
        }
    }

    fn operand(&mut self, operand: &mut Operand) {
        if let Operand::Const(index) = operand {
            let own = *index;
            *index = if let Some(kept) = self.constants[own] {
                kept
            } else {
                self.kept
                    .constants
                    .push(self.program.constants[own].clone());
                let kept = self.kept.constants.len() - 1;
                self.constants[own] = Some(kept);
                kept
            };
        }
    }

    fn body(&self, index: usize) -> usize {
        self.bodies[index].expect("a reachable body is joined")
    }
}

/// Every body a body names: a closure's and a direct call's.
fn each_body(body: &Body, mut found: impl FnMut(usize)) {
    for block in &body.blocks {
        for statement in &block.statements {
            if let Statement::Assign(_, Rvalue::Closure { body, .. }) =
                statement
            {
                found(*body);
            }
        }
        if let Terminator::Call {
            callee: Callee::Body(body),
            ..
        } = &block.terminator
        {
            found(*body);
        }
    }
}
