//! The wire: Debug Adapter Protocol messages, each a JSON body after a
//! `Content-Length` header, and the few message shapes the adapter
//! sends.
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};

/// The longest body the adapter reads; a longer one is discarded
/// unread and reported, never allocated.
pub const MAX_LENGTH: usize = 16 << 20;

/// Header-line bytes, including LF or CRLF.
pub const MAX_HEADER_LINE: usize = 8192;
/// All header bytes, including separators and inter-frame blank lines.
pub const MAX_HEADER_BYTES: usize = 32768;

fn header_line(
    input: &mut impl BufRead,
    used: &mut usize,
) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |at| at + 1);
        if count > MAX_HEADER_LINE - line.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "header line exceeds 8192 bytes",
            ));
        }
        if count > MAX_HEADER_BYTES - *used {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "headers exceed 32768 bytes",
            ));
        }
        let complete = available[count - 1] == b'\n';
        line.extend_from_slice(&available[..count]);
        *used += count;
        input.consume(count);
        if complete {
            break;
        }
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

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
/// A failed read or a header exceeding [`MAX_HEADER_LINE`] or
/// [`MAX_HEADER_BYTES`]. Over-limit headers are fatal, not recoverable messages.
pub fn read(input: &mut impl BufRead) -> io::Result<Option<Incoming>> {
    let mut length = None;
    let mut header = false;
    let mut used = 0;
    loop {
        let Some(line) = header_line(input, &mut used)? else {
            return Ok(None);
        };
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

#[cfg(test)]
mod header_tests {
    use super::*;
    use std::io::Cursor;

    // No backing allocation proportional to the advertised input length.
    struct Repeated {
        chunk: [u8; 31],
        consumed: usize,
    }
    impl Read for Repeated {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("an excessive header must not reach the body reader")
        }
    }
    impl BufRead for Repeated {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(&self.chunk)
        }
        fn consume(&mut self, count: usize) {
            self.consumed += count;
        }
    }
    struct Tracked {
        input: Cursor<Vec<u8>>,
        body_reads: usize,
    }
    impl Read for Tracked {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.body_reads += 1;
            self.input.read(bytes)
        }
    }
    impl BufRead for Tracked {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.input.fill_buf()
        }
        fn consume(&mut self, count: usize) {
            self.input.consume(count);
        }
    }
    fn padded_header(total: usize) -> Vec<u8> {
        let mut bytes = b"Content-Length: 2\r\n".to_vec();
        let mut remaining = total - bytes.len() - 2;
        while remaining != 0 {
            let length = remaining.min(MAX_HEADER_LINE);
            assert!(length >= 2);
            bytes.resize(bytes.len() + length - 2, b'x');
            bytes.extend(b"\r\n");
            remaining -= length;
        }
        bytes.extend(b"\r\n{}");
        bytes
    }
    #[test]
    fn header_byte_limits_preserve_boundaries_and_eof_policy() {
        for bytes in [
            b"Content-Length: 2\r\nX: ok\r\n\r\n{}".to_vec(),
            b"content-length: 2\nX: ok\n\n{}".to_vec(),
            padded_header(MAX_HEADER_BYTES),
        ] {
            let mut input = Cursor::new(bytes);
            assert!(matches!(
                read(&mut input).unwrap(),
                Some(Incoming::Message(_))
            ));
        }
        let mut exact = vec![b'x'; MAX_HEADER_LINE - 2];
        exact.extend(b"\r\nContent-Length: 2\r\n\r\n{}");
        assert!(matches!(
            read(&mut Cursor::new(exact)).unwrap(),
            Some(Incoming::Message(_))
        ));
        let mut over = vec![b'x'; MAX_HEADER_LINE - 1];
        over.extend(b"\r\nContent-Length: 2\r\n\r\n{}");
        for bytes in [
            over,
            padded_header(MAX_HEADER_BYTES + 1),
            b"X: \xff\r\n\r\n".to_vec(),
        ] {
            let mut input = Tracked {
                input: Cursor::new(bytes),
                body_reads: 0,
            };
            let error = read(&mut input).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(input.body_reads, 0);
            assert!(
                input.input.position()
                    <= u64::try_from(MAX_HEADER_BYTES).unwrap()
            );
        }
        for bytes in [
            b"X: partial".as_slice(),
            b"Content-Length: 2\r\n\r\n{".as_slice(),
        ] {
            assert!(read(&mut Cursor::new(bytes)).unwrap().is_none());
        }
        let mut repeated = Repeated {
            chunk: [b'x'; 31],
            consumed: 0,
        };
        let error = read(&mut repeated).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "header line exceeds 8192 bytes");
        assert!(repeated.consumed <= MAX_HEADER_LINE);
        let mut blanks = Repeated {
            chunk: [b'\n'; 31],
            consumed: 0,
        };
        let error = read(&mut blanks).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "headers exceed 32768 bytes");
        assert_eq!(blanks.consumed, MAX_HEADER_BYTES);
    }
}
