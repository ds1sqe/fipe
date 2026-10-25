//! The base protocol: messages framed by a `Content-Length` header over
//! a byte stream, one JSON value each.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// The longest body a message may have. A longer one is read and
/// dropped, never held, so a peer cannot make the server allocate it.
pub const MAX_LENGTH: usize = 64 << 20;

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
/// A read failure, a header exceeding the byte limits, or a header without a
/// length: the framing is lost
/// and no later message can be found.
pub fn frame(input: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let mut length = None;
    let mut used = 0;
    loop {
        let Some(line) = header_line(input, &mut used)? else {
            return Ok(None);
        };
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
                frame(&mut input).unwrap(),
                Some(Frame::Message(_))
            ));
        }
        let mut exact = vec![b'x'; MAX_HEADER_LINE - 2];
        exact.extend(b"\r\nContent-Length: 2\r\n\r\n{}");
        assert!(matches!(
            frame(&mut Cursor::new(exact)).unwrap(),
            Some(Frame::Message(_))
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
            let error = frame(&mut input).unwrap_err();
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
            assert!(frame(&mut Cursor::new(bytes)).unwrap().is_none());
        }
        let mut repeated = Repeated {
            chunk: [b'x'; 31],
            consumed: 0,
        };
        let error = frame(&mut repeated).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "header line exceeds 8192 bytes");
        assert!(repeated.consumed <= MAX_HEADER_LINE);
    }
}
