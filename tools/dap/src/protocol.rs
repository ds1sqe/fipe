//! The wire: Debug Adapter Protocol messages, each a JSON body after a
//! `Content-Length` header, and the few message shapes the adapter
//! sends.
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};

/// The longest body the adapter reads; a longer one is discarded
/// unread and reported, never allocated.
pub const MAX_LENGTH: usize = 16 << 20;

/// What one framed message on the input is.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A body that parses as JSON.
    Message(Value),
    /// A frame the adapter cannot read: why, in words. The input goes on
    /// at the next frame.
    Malformed(String),
}

/// Read one message: `None` at the end of the input.
///
/// A header without a usable `Content-Length`, a body longer than
/// [`MAX_LENGTH`] or one that is not JSON is [`Incoming::Malformed`],
/// and the next read starts at the following frame.
///
/// # Errors
/// A failed read.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Incoming>> {
    let mut length = None;
    let mut header = false;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if header {
                break;
            }
            // Blank lines between frames are not a header.
            continue;
        }
        header = true;
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<usize>());
            }
        }
    }
    let length = match length {
        Some(Ok(length)) => length,
        Some(Err(_)) => {
            return Ok(Some(Incoming::Malformed(String::from(
                "the Content-Length header is not a number",
            ))))
        }
        None => {
            return Ok(Some(Incoming::Malformed(String::from(
                "a message without a Content-Length header",
            ))))
        }
    };
    if length > MAX_LENGTH {
        let wanted = u64::try_from(length).unwrap_or(u64::MAX);
        io::copy(&mut input.by_ref().take(wanted), &mut io::sink())?;
        return Ok(Some(Incoming::Malformed(format!(
            "a message of {length} bytes is longer than the {MAX_LENGTH} \
             the adapter reads"
        ))));
    }
    let mut body = vec![0; length];
    match input.read_exact(&mut body) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    }
    Ok(Some(serde_json::from_slice(&body).map_or_else(
        |error| Incoming::Malformed(format!("the body is not JSON: {error}")),
        Incoming::Message,
    )))
}

/// Write one message with its header.
///
/// # Errors
/// A failed write.
pub fn write(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// The messages the adapter sends, numbered in order.
#[derive(Debug, Default)]
pub struct Outgoing {
    seq: u64,
    /// Every message made so far and not yet taken.
    pub messages: Vec<Value>,
}

impl Outgoing {
    const fn next(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// A response to `request`, with a body on success or a message on
    /// failure.
    pub fn respond(&mut self, request: &Value, result: Result<Value, String>) {
        let seq = self.next();
        let mut response = json!({
            "seq": seq,
            "type": "response",
            "request_seq": request["seq"].as_u64().unwrap_or(0),
            "command": request["command"].as_str().unwrap_or_default(),
            "success": result.is_ok(),
        });
        match result {
            Ok(body) if !body.is_null() => response["body"] = body,
            Ok(_) => {}
            Err(message) => response["message"] = Value::String(message),
        }
        self.messages.push(response);
    }

    /// An event with its body.
    pub fn event(&mut self, event: &str, body: Value) {
        let seq = self.next();
        let mut message =
            json!({ "seq": seq, "type": "event", "event": event });
        if !body.is_null() {
            message["body"] = body;
        }
        self.messages.push(message);
    }

    /// Take the messages made so far.
    pub fn take(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.messages)
    }
}
