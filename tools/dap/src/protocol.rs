//! The wire: Debug Adapter Protocol messages, each a JSON body after a
//! `Content-Length` header, and the few message shapes the adapter
//! sends.
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// Read one message: `None` at the end of the input.
///
/// # Errors
/// A malformed header or body, or a failed read.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length.ok_or_else(|| invalid("no Content-Length"))?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::from)
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

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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
            "request_seq": request["seq"],
            "command": request["command"],
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
