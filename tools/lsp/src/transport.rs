//! The base protocol: messages framed by a `Content-Length` header over
//! a byte stream, one JSON value each.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// The longest body a message may have. A longer one is read and
/// dropped, never held, so a peer cannot make the server allocate it.
pub const MAX_LENGTH: usize = 64 << 20;

/// What one frame held: a JSON value, or why it is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Message(Value),
    /// A body longer than [`MAX_LENGTH`], dropped unread.
    TooLong(usize),
    /// A body that is not JSON, with the parser's reason.
    Unparsable(String),
}

/// Read one frame; `None` at the end of the stream, or when the stream
/// ends inside a body.
///
/// # Errors
/// A read failure, or a header without a length: the framing is lost
/// and no later message can be found.
pub fn frame(input: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<u64>().ok();
            }
        }
    }
    let length = length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "no Content-Length")
    })?;
    let fits = usize::try_from(length).ok().filter(|n| *n <= MAX_LENGTH);
    let Some(fits) = fits else {
        let dropped = io::copy(&mut input.take(length), &mut io::sink())?;
        if dropped < length {
            return Ok(None);
        }
        return Ok(Some(Frame::TooLong(
            usize::try_from(length).unwrap_or(usize::MAX),
        )));
    };
    let mut body = vec![0; fits];
    match input.read_exact(&mut body) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    Ok(Some(match serde_json::from_slice(&body) {
        Ok(value) => Frame::Message(value),
        Err(error) => Frame::Unparsable(error.to_string()),
    }))
}

/// Read one message, for a client that trusts its peer; `None` at the
/// end of the stream.
///
/// # Errors
/// A read failure, a header without a length, or a body that is too
/// long or not JSON.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    match frame(input)? {
        None => Ok(None),
        Some(Frame::Message(value)) => Ok(Some(value)),
        Some(Frame::TooLong(length)) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a body of {length} bytes is too long"),
        )),
        Some(Frame::Unparsable(reason)) => {
            Err(io::Error::new(io::ErrorKind::InvalidData, reason))
        }
    }
}

/// Write one message and flush it.
///
/// # Errors
/// A write failure.
pub fn write(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}
