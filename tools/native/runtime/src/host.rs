//! The host of a run: the pipe, or the std host in the process.
//!
//! With a pipe the program writes frames on fd 3 and reads replies on
//! fd 4, each a length of eight bytes little-endian, then the payload
//! `codec` defines. A host call writes a `Call` frame and blocks on
//! its reply; the run's end writes a `Done` frame. Without fd 3 the
//! std host (`fipe-host`) answers in the process, under its contract:
//! the working directory and `FIPE_ALLOW_WRITE`'s directories are the
//! write roots, the arguments after the program's name are `cli.args`,
//! and the output goes to the standard streams.
use crate::codec::{self, Frame};
use fipe::{
    embed::{Reply, Request},
    ir::Tables,
    value::Value,
};
use fipe_host::Policy;
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Why a host call got no reply.
#[derive(Debug)]
pub enum Error {
    /// No host pipe is connected.
    Absent,
    Io(io::Error),
    Codec(codec::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("no host is connected"),
            Self::Io(error) => write!(f, "the host pipe failed: {error}"),
            Self::Codec(error) => write!(f, "{error}"),
        }
    }
}

/// Who answers the run's calls.
pub enum Host {
    /// The two ends of the pipe.
    Pipe {
        requests: File,
        replies: File,
        failed: bool,
    },
    /// The std host, its output streamed to stdout and stderr.
    Std(fipe_host::Host),
}

impl Host {
    /// The pipe on fds 3 and 4, opened through `/dev/fd`, or, when fd 3
    /// is not open, the std host over the working directory.
    ///
    /// # Errors
    /// When a write root cannot be resolved.
    pub fn open() -> Result<Self, String> {
        let requests = OpenOptions::new().write(true).open("/dev/fd/3");
        let replies = File::open("/dev/fd/4");
        if let Some((requests, replies)) = requests.ok().zip(replies.ok()) {
            return Ok(Self::Pipe {
                requests,
                replies,
                failed: false,
            });
        }
        let allow: Vec<PathBuf> = std::env::var_os("FIPE_ALLOW_WRITE")
            .map(|dirs| std::env::split_paths(&dirs).collect())
            .unwrap_or_default();
        let policy = Policy::new(Path::new("."), &allow)
            .map_err(|error| format!("the write roots: {error}"))?;
        let args = std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let host = fipe_host::Host::new(policy)
            .with_args(args)
            .streaming(io::stdout(), io::stderr());
        Ok(Self::Std(host))
    }

    /// Whether a host pipe is connected.
    pub const fn connected(&self) -> bool {
        matches!(self, Self::Pipe { .. })
    }

    /// Ask the host a call, with the main lines kept so far; its reply.
    pub fn call(
        &mut self,
        request: &Request,
        lines: &[(usize, Value)],
        tables: &Tables,
    ) -> Result<Result<Value, String>, Error> {
        match self {
            Self::Std(host) => {
                host.sink.lines(lines, tables);
                if let Err(error) = host.flush() {
                    return Ok(Err(format!("cannot write output: {error}")));
                }
                let reply = host.answer(request, tables);
                let flushed = host.flush();
                match (reply, flushed) {
                    (Reply::Value(Err(reason)), _) => Ok(Err(reason)),
                    (_, Err(error)) => {
                        Ok(Err(format!("cannot write output: {error}")))
                    }
                    (Reply::Value(value), Ok(())) => Ok(value),
                    (Reply::File(_), Ok(())) => {
                        unreachable!("a run asks only for host calls")
                    }
                }
            }
            Self::Pipe { failed: true, .. } => Err(Error::Absent),
            Self::Pipe { .. } => {
                let frame = Frame::Call {
                    request: request.clone(),
                    lines: lines.to_vec(),
                };
                let result = self.send(&frame).and_then(|()| {
                    let Self::Pipe { replies, .. } = self else {
                        unreachable!("the pipe the frame went down")
                    };
                    read_reply(replies)
                });
                if result.is_err() {
                    let Self::Pipe { failed, .. } = self else {
                        unreachable!()
                    };
                    *failed = true;
                }
                result
            }
        }
    }

    /// Write one frame on the pipe.
    pub fn send(&mut self, frame: &Frame) -> Result<(), Error> {
        let Self::Pipe {
            requests,
            failed: false,
            ..
        } = self
        else {
            return Err(Error::Absent);
        };
        let payload = frame.encode().map_err(Error::Codec)?;
        let mut out = (payload.len() as u64).to_le_bytes().to_vec();
        out.extend(payload);
        requests.write_all(&out).map_err(Error::Io)
    }
}

fn read_reply(replies: &mut impl Read) -> Result<Result<Value, String>, Error> {
    let mut length = [0; 8];
    replies.read_exact(&mut length).map_err(Error::Io)?;
    let announced = u64::from_le_bytes(length);
    if announced > codec::MAX_REPLY_BYTES {
        return Err(Error::Codec(codec::Error::TooLong(announced)));
    }
    let length = usize::try_from(announced)
        .map_err(|_| Error::Codec(codec::Error::Malformed))?;
    let mut payload = vec![0; length];
    replies.read_exact(&mut payload).map_err(Error::Io)?;
    codec::decode_reply(&payload).map_err(Error::Codec)
}

#[cfg(test)]
mod reply_tests {
    use super::*;
    use std::io::Cursor;

    struct HeaderOnly {
        header: Cursor<[u8; 8]>,
        body_reads: usize,
    }
    impl Read for HeaderOnly {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.header.position() == 8 {
                self.body_reads += 1;
                panic!("excessive length reached body reader");
            }
            self.header.read(bytes)
        }
    }
    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut bytes =
            u64::try_from(payload.len()).unwrap().to_le_bytes().to_vec();
        bytes.extend(payload);
        bytes
    }
    #[test]
    fn native_reply_reader_bounds_lengths_before_body_and_preserves_valid_errors(
    ) {
        for reply in [Ok(Value::Int(7)), Err("denied".into())] {
            let bytes = codec::encode_reply(&reply).unwrap();
            assert_eq!(
                read_reply(&mut Cursor::new(framed(&bytes))).unwrap(),
                reply
            );
        }
        for length in 0..8 {
            let error =
                read_reply(&mut Cursor::new(vec![0; length])).unwrap_err();
            assert!(
                matches!(error, Error::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof)
            );
        }
        let mut short = 24_u64.to_le_bytes().to_vec();
        short.extend([0; 8]);
        assert!(matches!(read_reply(&mut Cursor::new(short)),
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof));
        for length in [codec::MAX_REPLY_BYTES + 1, u64::MAX] {
            let mut input = HeaderOnly {
                header: Cursor::new(length.to_le_bytes()),
                body_reads: 0,
            };
            assert!(matches!(read_reply(&mut input),
                Err(Error::Codec(codec::Error::TooLong(actual))) if actual == length));
            assert_eq!(input.body_reads, 0);
        }
        assert!(matches!(
            read_reply(&mut Cursor::new(framed(&99_u64.to_le_bytes()))),
            Err(Error::Codec(codec::Error::Tag(99)))
        ));
        let invalid_utf8: Vec<_> = [1_u64, 1]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .chain([255])
            .collect();
        assert!(matches!(
            read_reply(&mut Cursor::new(framed(&invalid_utf8))),
            Err(Error::Codec(codec::Error::Malformed))
        ));
        // Real production limit, not a small test-only substitute. Chain avoids
        // allocating another framing copy of the 64 MiB payload.
        let length = usize::try_from(codec::MAX_REPLY_BYTES).unwrap() - 24;
        let value = Value::Text(std::rc::Rc::new("x".repeat(length)));
        let payload = codec::encode_reply(&Ok(value.clone())).unwrap();
        assert_eq!(
            u64::try_from(payload.len()).unwrap(),
            codec::MAX_REPLY_BYTES
        );
        let mut input = Cursor::new(codec::MAX_REPLY_BYTES.to_le_bytes())
            .chain(Cursor::new(payload.as_slice()));
        assert_eq!(read_reply(&mut input).unwrap(), Ok(value));
    }
}
