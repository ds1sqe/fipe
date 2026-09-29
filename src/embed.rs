//! The embedding boundary. The crate calls nothing: every effect is a
//! [`Request`] returned to the driver, every answer a [`Reply`] passed
//! back. `Documentation/spec/embed-api.md` owns the protocol.

use alloc::string::String;

/// An effect the crate needs from its driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// The source of the file module `stem`; the driver maps the stem to
    /// a path.
    File { stem: String },
}

/// The driver's answer to the pending [`Request`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The requested file's text, or `None` when it does not exist.
    File(Option<String>),
}

/// A step that either needs an effect or has finished.
#[must_use]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    Request(Request),
    Done(T),
}
