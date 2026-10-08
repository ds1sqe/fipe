//! The machine over the graph: evaluation rows through `check`,
//! `compile` and `vm::Machine`.
use fipe::{
    check,
    decl::Declarations,
    embed::Outcome,
    ir,
    lex::lex,
    parse::parse,
    value::{structural, Value},
    vm::{Fault, Machine},
};

#[path = "vm/run.rs"]
mod run;

#[path = "vm/arith.rs"]
mod arith;
#[path = "vm/calls.rs"]
mod calls;
#[path = "vm/closures.rs"]
mod closures;
#[path = "vm/dispatch.rs"]
mod dispatch;
#[path = "vm/natives.rs"]
mod natives;
