//! The graph below the checked unit, the one contract every backend
//! consumes.
//!
//! A program is bodies; a body is blocks over typed locals; a block is
//! statements and one terminator. Every reference is an index into the
//! program or into the unit's `ir::Tables`; no name is executable, and
//! nothing here is a `String`.
use crate::{ir, value::Value};
use alloc::{rc::Rc, vec::Vec};
use core::cell::RefCell;

/// A lowered unit: every body reachable from its main, lowered once.
#[derive(Clone, Debug)]
pub struct Program {
    /// The unit's tables, shared by a session's units.
    pub tables: Rc<RefCell<ir::Tables>>,
    pub bodies: Vec<Body>,
    /// Creation locators for live rows; withdrawal discards removed rows
    /// while retained callable bodies can still bind members of the live prefix.
    pub receiver_adapters: Vec<ReceiverAdapter>,
    /// Immutable invocation gates, indexed by body, retained for aliases.
    pub receiver_contexts: Vec<Option<ReceiverContext>>,
    /// Values prepared at compile time, read by `Operand::Const`.
    pub constants: Vec<Value>,
    /// The body of each definition with spots, sized to
    /// `tables.definitions`; `None` for a value definition (a global)
    /// or one this program does not reach.
    pub definitions: Vec<Option<usize>>,
    /// The body of each limit test a gate or an entry reaches, sized to
    /// `tables.tests`; arity one, the self slot.
    pub tests: Vec<Option<usize>>,
    /// A module index with its init body, in file order; a backend runs
    /// each not yet initialized before `main`.
    pub inits: Vec<(usize, usize)>,
    /// The body computing a value definition that no line stores, such
    /// as an `impl` block's value member, by definition index; a backend
    /// runs it on the first read of the unset global and stores it.
    pub globals: Vec<Option<usize>>,
    /// The body of the unit's main lines.
    pub main: usize,
}

/// The identity of the raw operation behind a receiver-first adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverTarget {
    Definition(usize),
    Declared(usize),
    Derived(ir::Derived),
}
/// A locator is scoped to the current append-only table rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverAdapter {
    pub member: usize,
    pub row: usize,
    pub target: ReceiverTarget,
    pub receiver_spot: usize,
    pub body: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverComparison {
    Other,
    Eq,
    Cmp,
}
/// The extra witness is separate from the raw operation's witnesses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverContext {
    /// The selected row's input entry, admitted as each spot is supplied.
    pub input_contract: Option<usize>,
    /// Exact executed Test-meaning IDs, not the operation's other variables.
    pub input_parameters: Vec<usize>,
    pub receiver_meaning: usize,
    pub comparison: ReceiverComparison,
    /// A Declared adapter's full caller callable, projected only on full call.
    pub protocol_result: Option<ReceiverResult>,
}
/// Where the caller callable is held and how many protocol formals to peel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverResult {
    pub meaning: usize,
    pub arity: usize,
}

/// A graph of blocks over typed locals: a function, a thunk, a test, a
/// default, a module's init or the main.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// The IR node this body lowers.
    pub node: usize,
    /// Declared spots, in order; zero for a value.
    pub arity: usize,
    /// How many captured values a closure of this body supplies.
    pub captures: usize,
    /// Generated open-call wrapper mapping; absent on ordinary bodies.
    pub binding: Option<OpenBinding>,
    /// The IR meaning parameters whose witness the caller supplies, in
    /// order: a `Type::Parameter(p)` met at run time is the witness at
    /// `p`'s position here.
    pub hidden: Vec<usize>,
    /// `_0` the result, then the spots, then binds, call slots and
    /// temporaries.
    pub locals: Vec<Local>,
    pub blocks: Vec<Block>,
}

/// The captured callee and relative raw spots of a generated open-call wrapper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenBinding {
    pub callee_capture: usize,
    pub open: Vec<usize>,
}

/// A typed slot of a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub ty: ir::Type,
    /// The IR slot this local holds, whose name is
    /// `Tables.slots[slot].name`; `None` for a temporary.
    pub slot: Option<usize>,
}

/// Statements and one terminator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
    /// The IR node of each statement, then of the terminator.
    pub nodes: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Assign(usize, Rvalue),
    /// The IR nodes this block pays, in the order their lowering
    /// completed; opens the block when any, and exhaustion Faults at
    /// the span of the first node that cannot be paid.
    Charge(Vec<usize>),
    /// Write a value definition's global.
    Store {
        definition: usize,
        value: Operand,
    },
    /// An `Eval` entry; failure Faults naming it.
    Enter {
        entry: usize,
        value: Operand,
    },
    /// Open the child environment of a `With`; `key` indexes
    /// `tables.keys`. A backend keeps the stack and hands it to the
    /// host on a host call.
    Push {
        key: usize,
        value: Operand,
    },
    /// Close the environment the matching `Push` opened.
    Pop,
    /// A main line's value, kept for the host in line order
    /// ([`crate::vm::Machine::lines`]); its node is the line.
    Line(Operand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Local(usize),
    Const(usize),
    Capture(usize),
    /// A value definition's stored global.
    Global(usize),
    /// The frame's hidden witness, as `Value::Meaning`.
    Hidden(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rvalue {
    Use(Operand),
    /// Admit one source supply without invoking the callable.
    /// Returns the private callable and admitted argument as a two-part tuple.
    Admit {
        values: [Operand; 2],
        spot: usize,
    },
    Construct {
        meaning: usize,
        constructor: usize,
        fields: Vec<Operand>,
    },
    Tuple(Vec<Operand>),
    Array(Vec<Operand>),
    Repeat {
        value: Operand,
        length: Length,
    },
    /// A record's field, a tuple's part, a sequence's element, or a
    /// packed value's payload at `0`.
    Field(Operand, usize),
    /// The element count of a sequence, as an integer.
    Length(Operand),
    /// `meanings` are substituted by the creating frame.
    Closure {
        body: usize,
        captures: Vec<Operand>,
        meanings: Vec<ir::Type>,
    },
    /// A rebuild selected by the checker.
    Wrap {
        meaning: usize,
        constructor: usize,
        value: Operand,
    },
    /// A cast of `value` by its row; a selected reader or shower was
    /// called before, so the Read row unwraps and the Show row passes.
    Cast {
        row: ir::CastRow,
        from: ir::Type,
        to: ir::Type,
        value: Operand,
    },
    /// A conversion's adaptation of `value`; a selected member was
    /// called before. `Check` and `TryCheck` test `predicate`.
    Convert {
        mode: ir::ConversionMode,
        predicate: ir::Predicate,
        value: Operand,
    },
    Pack {
        tag: ir::Type,
        value: Operand,
    },
    /// A prelude operation by its declaration; may Fault, never
    /// suspends.
    Native {
        op: usize,
        args: Vec<Operand>,
    },
}

/// The length of a `Repeat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    Fixed(u64),
    /// Read from the frame's hidden witness.
    Hidden(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    Goto(usize),
    /// Constructor index to block; on a packed value the cases are
    /// meaning indices, on an integer the numbers.
    Switch {
        value: Operand,
        cases: Vec<(usize, usize)>,
        otherwise: Option<usize>,
    },
    /// A gate pattern.
    Test {
        predicate: ir::Predicate,
        value: Operand,
        pass: usize,
        fail: usize,
    },
    /// `meanings` are the callee's witnesses, substituted by the calling
    /// frame; the result lands in `dest`, then `target` runs.
    Call {
        callee: Callee,
        args: Vec<Operand>,
        meanings: Vec<ir::Type>,
        dest: usize,
        target: usize,
    },
    Return,
    /// A block coverage proves no run reaches; reaching it is a crate
    /// defect.
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callee {
    Body(usize),
    /// A prelude declaration, by `tables.declarations`.
    Native(usize),
    /// A declaration of another namespace, by `tables.declarations`;
    /// the backend hands the call to the host.
    Host(usize),
    /// A derived operation the backend performs structurally on the
    /// type, substituted by the frame when it holds a parameter.
    Derived {
        kind: ir::Derived,
        ty: ir::Type,
    },
    /// Resolved at run time by the witness.
    Dispatch {
        member: usize,
        witness: Witness,
    },
    /// Eval/apply: fewer arguments than the closure's arity fill, equal
    /// enter, more enter and apply the rest to the result.
    Value(Operand),
}

/// What a run-time dispatch reads, mirroring `ir::Receiver`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Witness {
    /// The value's tag.
    Value(Operand),
    /// A meaning, substituted by the frame.
    Meaning(ir::Type),
}
