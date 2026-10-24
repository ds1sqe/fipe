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
    Pipe { requests: File, replies: File },
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
            return Ok(Self::Pipe { requests, replies });
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
            Self::Pipe { .. } => {
                let frame = Frame::Call {
                    request: request.clone(),
                    lines: lines.to_vec(),
                };
                self.send(&frame)?;
                let Self::Pipe { replies, .. } = self else {
                    unreachable!("the pipe the frame went down")
                };
                let mut length = [0; 8];
                replies.read_exact(&mut length).map_err(Error::Io)?;
                let length = usize::try_from(u64::from_le_bytes(length))
                    .map_err(|_| Error::Codec(codec::Error::Malformed))?;
                let mut payload = vec![0; length];
                replies.read_exact(&mut payload).map_err(Error::Io)?;
                codec::decode_reply(&payload).map_err(Error::Codec)
            }
        }
    }

    /// Write one frame on the pipe.
    pub fn send(&mut self, frame: &Frame) -> Result<(), Error> {
        let Self::Pipe { requests, .. } = self else {
            return Err(Error::Absent);
        };
        let payload = frame.encode().map_err(Error::Codec)?;
        let mut out = (payload.len() as u64).to_le_bytes().to_vec();
        out.extend(payload);
        requests.write_all(&out).map_err(Error::Io)
    }
}
