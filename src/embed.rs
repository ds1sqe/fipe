//! The embedding boundary. The crate calls nothing: every effect is a
//! [`Request`] returned to the driver, every answer a [`Reply`] passed
//! back. `Documentation/api/09-embed.md` owns the protocol.

use crate::value::Value;
use alloc::{string::String, vec::Vec};

/// An effect the crate needs from its driver.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// The source of the file module `stem`; the driver maps the stem to
    /// a path.
    File { stem: String },
    /// A call of the host declaration `declaration` (an index into the
    /// unit's `tables.declarations`) with its arguments in spot order,
    /// under the environment the enclosing `with` blocks opened: keys
    /// index `tables.keys`, outermost first.
    Call {
        declaration: usize,
        arguments: Vec<Value>,
        environment: Vec<(usize, Value)>,
    },
}

/// The driver's answer to the pending [`Request`].
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The requested file's text, or `None` when it does not exist.
    File(Option<String>),
    /// A host call's value, or the message of its failure, which Faults
    /// the run.
    Value(Result<Value, String>),
}

/// A step that either needs an effect or has finished.
#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    Request(Request),
    Done(T),
}
