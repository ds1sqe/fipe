//! Lowering a checked unit to its MIR program.
//!
//! The checker recorded every choice, so lowering cannot fail: it panics
//! only on a checker invariant, a crate defect. Every body reachable
//! from the unit's main is lowered once; a definition's body is shared
//! by every reference and every row that targets it.
mod adapters;
mod bodies;
mod builder;
mod calls;
mod cases;
mod columns;
mod constants;
pub(crate) mod dispatch;
mod meanings;
mod outcomes;
mod parts;
mod patterns;
mod thunks;

use crate::{ir, mir, mir::Terminator, value::Value};
use alloc::{rc::Rc, vec::Vec};
use builder::Builder;
use core::cell::Ref;
use patterns::build_function;

/// Lower a checked unit to the program every backend consumes.
///
/// # Panics
/// Panics only if a checker invariant is broken, a crate defect.
#[must_use]
pub fn compile(unit: &ir::Unit) -> mir::Program {
    let tables = unit.tables();
    let mut lowering = Lowering {
        prelude: tables
            .namespaces
            .iter()
            .position(|label| label == "prelude"),
        tables: &tables,
        bodies: Vec::new(),
        constants: Vec::new(),
        definitions: alloc::vec![None; tables.definitions.len()],
        tests: alloc::vec![None; tables.tests.len()],
        globals: alloc::vec![None; tables.definitions.len()],
        stored: stored(&tables),
        pending: Vec::new(),
    };
    let main = lowering.lines(&unit.main);
    let inits = (0..tables.modules.len())
        .map(|module| (module, lowering.lines(&tables.modules[module].lines)))
        .collect();
    while let Some(pending) = lowering.pending.pop() {
        match pending {
            Pending::Definition(definition, body) => {
                lowering.definition(definition, body);
            }
            Pending::Test(test, body) => lowering.test(test, body),
            Pending::Global(definition, body) => {
                lowering.global(definition, body);
            }
        }
    }
    mir::Program {
        tables: Rc::clone(&unit.tables),
        bodies: lowering.bodies,
        constants: lowering.constants,
        definitions: lowering.definitions,
        tests: lowering.tests,
        globals: lowering.globals,
        inits,
        main,
    }
}

/// Which definitions a line stores: those bound by a slot pattern.
fn stored(tables: &ir::Tables) -> Vec<bool> {
    let mut stored = alloc::vec![false; tables.definitions.len()];
    for node in &tables.nodes {
        if let ir::Kind::Bind { pattern, .. } | ir::Kind::Lift { pattern, .. } =
            &node.kind
        {
            mark_stored(tables, pattern, &mut stored);
        }
    }
    stored
}

/// The definitions a binding pattern's slots hold: a line's names.
fn mark_stored(
    tables: &ir::Tables,
    pattern: &ir::Pattern,
    stored: &mut [bool],
) {
    use ir::PatternKind as P;
    match &pattern.kind {
        P::Slot(slot) => {
            if let Some(definition) = tables.slots[*slot].definition {
                stored[definition] = true;
            }
        }
        P::Any | P::Literal { .. } => (),
        P::Constructor { parts, .. } | P::List(parts) | P::Tuple(parts) => {
            for part in parts {
                mark_stored(tables, part, stored);
            }
        }
        P::Record { fields, .. } => {
            for (_, part) in fields {
                mark_stored(tables, part, stored);
            }
        }
        P::Side { payload, .. } => {
            if let Some(payload) = payload {
                mark_stored(tables, payload, stored);
            }
        }
        P::Gate { payload, .. } | P::Tag { payload, .. } => {
            mark_stored(tables, payload, stored);
        }
    }
}

/// A body reserved but not yet lowered.
enum Pending {
    Definition(usize, usize),
    Test(usize, usize),
    Global(usize, usize),
}

/// The program under construction.
struct Lowering<'a> {
    tables: &'a Ref<'a, ir::Tables>,
    /// The prelude's namespace, whose declarations are native.
    prelude: Option<usize>,
    bodies: Vec<mir::Body>,
    constants: Vec<Value>,
    definitions: Vec<Option<usize>>,
    tests: Vec<Option<usize>>,
    /// The body of each value definition no line stores.
    globals: Vec<Option<usize>>,
    /// Which value definitions a line stores.
    stored: Vec<bool>,
    /// Bodies whose index is reserved but not yet lowered.
    pending: Vec<Pending>,
}

impl Lowering<'_> {
    /// The body lowered for a definition with spots, reserved on first
    /// use; `None` for a value definition, a global.
    fn function_body(&mut self, definition: usize) -> Option<usize> {
        if let Some(body) = self.definitions[definition] {
            return Some(body);
        }
        let node = self.unwrapped(self.tables.definitions[definition].body);
        if !matches!(self.tables.nodes[node].kind, ir::Kind::Function { .. }) {
            return None;
        }
        let body = self.reserve(node);
        self.definitions[definition] = Some(body);
        self.pending.push(Pending::Definition(definition, body));
        Some(body)
    }

    /// The body of a limit test, reserved on first use.
    fn test_body(&mut self, test: usize) -> usize {
        if let Some(body) = self.tests[test] {
            return body;
        }
        let body = self.reserve(self.tables.tests[test].body);
        self.tests[test] = Some(body);
        self.pending.push(Pending::Test(test, body));
        body
    }

    /// Lower every test a predicate composes, so a backend finds them.
    pub(super) fn reach_predicate(&mut self, predicate: &ir::Predicate) {
        match predicate {
            ir::Predicate::Nonzero { .. } => (),
            ir::Predicate::Function { parameter, result } => {
                self.reach_predicate(parameter);
                self.reach_predicate(result);
            }
            ir::Predicate::All(parts) | ir::Predicate::Any(parts) => {
                for part in parts {
                    self.reach_predicate(part);
                }
            }
            ir::Predicate::Not(inner) => self.reach_predicate(inner),
            ir::Predicate::Test { index, .. } => {
                self.test_body(*index);
            }
        }
    }

    /// A type with every fixed session meaning substituted.
    pub(super) fn fixed(&self, ty: &ir::Type) -> ir::Type {
        dispatch::fixed(self.tables, ty)
    }

    /// Lower every written row of the members a native or derived
    /// operation may call while it walks a value (`show` of a record
    /// whose field has a written `show`), so the machine finds the
    /// bodies. The labels name the members by the words of the prelude.
    pub(super) fn reach_members(&mut self, labels: &[&str]) {
        let rows: Vec<usize> = self
            .tables
            .dispatch
            .iter()
            .filter(|member| labels.contains(&member.label.as_str()))
            .flat_map(|member| member.rows.iter())
            .filter_map(|row| match row.target {
                ir::Implementation::Definition { index, .. } => Some(index),
                _ => None,
            })
            .collect();
        for definition in rows {
            self.function_body(definition);
        }
    }

    /// The members a prelude word walks by tag at run time, if any.
    pub(super) fn walked_members(label: &str) -> &'static [&'static str] {
        match label {
            "show" => &["show"],
            "eq" | "count" => &["eq"],
            "cmp" | "sort" => &["cmp"],
            "default" => &["default"],
            "sum" => &["add", "zero"],
            _ => &[],
        }
    }

    /// A body index with a placeholder, filled when it is lowered, so a
    /// recursive reference finds it.
    fn reserve(&mut self, node: usize) -> usize {
        self.bodies.push(mir::Body {
            node,
            arity: 0,
            captures: 0,
            hidden: Vec::new(),
            locals: Vec::new(),
            blocks: Vec::new(),
        });
        self.bodies.len() - 1
    }

    /// A definition's body is the function below its single-part blocks.
    fn unwrapped(&self, mut node: usize) -> usize {
        while let ir::Kind::Block(parts) = &self.tables.nodes[node].kind {
            let [part] = parts.as_slice() else { break };
            node = *part;
        }
        node
    }

    /// Whether a declaration is the prelude's, hence native.
    fn native(&self, declaration: usize) -> bool {
        Some(self.tables.declarations[declaration].namespace) == self.prelude
    }

    /// A constant's index, shared when an equal one exists; floats are
    /// equal by their bits, so `-0.0` keeps its sign.
    fn constant(&mut self, value: Value) -> usize {
        if let Some(index) =
            self.constants.iter().position(|known| same(known, &value))
        {
            return index;
        }
        self.constants.push(value);
        self.constants.len() - 1
    }

    /// The number of spots a declaration's type takes.
    fn arity(ty: &ir::Type) -> usize {
        let mut count = 0;
        let mut ty = ty;
        while let ir::Type::Function { result, .. } = ty {
            count += 1;
            ty = result;
        }
        count
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::F32(a), Value::F32(b)) => a.to_bits() == b.to_bits(),
        (Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
        (Value::F32(_) | Value::F64(_), _)
        | (_, Value::F32(_) | Value::F64(_)) => false,
        _ => a == b,
    }
}

impl Lowering<'_> {
    /// The main body or a module's init body: its lines in order, `_0`
    /// the last value.
    pub(super) fn lines(&mut self, lines: &[usize]) -> usize {
        let node = lines.last().copied().unwrap_or(0);
        let result = lines
            .last()
            .map_or(ir::Type::Unit, |last| self.tables.nodes[*last].ty.clone());
        let mut builder =
            Builder::new(self, node, result, Vec::new(), Vec::new());
        builder.sequence(node, lines, Some(0));
        builder.terminate(Terminator::Return, node);
        builder.finish(None)
    }

    /// A definition's reserved body, from its function node.
    pub(super) fn definition(&mut self, definition: usize, body: usize) {
        let node = self.unwrapped(self.tables.definitions[definition].body);
        let ir::Kind::Function {
            hidden,
            captures,
            arms,
            ..
        } = &self.tables.nodes[node].kind
        else {
            unreachable!("a reserved body is a function")
        };
        build_function(self, node, hidden, captures, arms, Some(body));
    }

    /// A value definition read as a global: when no line stores it, a
    /// body computing it, reserved on first read.
    pub(super) fn reach_global(&mut self, definition: usize) {
        if self.stored[definition] || self.globals[definition].is_some() {
            return;
        }
        let body = self.reserve(self.tables.definitions[definition].body);
        self.globals[definition] = Some(body);
        self.pending.push(Pending::Global(definition, body));
    }

    /// A global's reserved body: the definition's value into `_0`.
    pub(super) fn global(&mut self, definition: usize, body: usize) {
        let node = self.tables.definitions[definition].body;
        let result = self.tables.nodes[node].ty.clone();
        let mut builder =
            Builder::new(self, node, result, Vec::new(), Vec::new());
        builder.lower_to(node, Some(0));
        builder.terminate(Terminator::Return, node);
        builder.finish(Some(body));
    }

    /// A limit test's reserved body: one spot, the self slot.
    pub(super) fn test(&mut self, test: usize, body: usize) {
        let ir::Test {
            parameters,
            body: node,
            self_slot,
            ..
        } = &self.tables.tests[test];
        let (node, self_slot) = (*node, *self_slot);
        let hidden = parameters.clone();
        let result = self.tables.nodes[node].ty.clone();
        let self_ty = self.tables.slots[self_slot].ty.clone();
        let mut builder = Builder::new(self, node, result, hidden, Vec::new());
        builder.spot(Some(self_slot), self_ty, node);
        builder.lower_to(node, Some(0));
        builder.terminate(Terminator::Return, node);
        builder.finish(Some(body));
    }
}
