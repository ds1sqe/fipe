//! A debug adapter for Fipe: the Debug Adapter Protocol over stdio, a
//! session over one program and a replay host reading the crate's
//! trace fixtures. Every value shown is the VM's and every stop is one
//! the VM took; the adapter owns only the protocol, the positions and
//! the session.
pub mod debuggee;
pub mod inspect;
pub mod lines;
pub mod protocol;
pub mod replay;
pub mod session;
