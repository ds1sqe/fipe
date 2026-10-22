//! The host pipe: the program writes frames on fd 3 and reads replies
//! on fd 4, each a length of eight bytes little-endian, then the
//! payload `codec` defines. A host call writes a `Call` frame and
//! blocks on its reply; the run's end writes a `Done` frame. Without
//! fd 3 no host is connected: a host call Faults and the result goes to
//! the standard streams.
use crate::codec::{self, Frame};
use fipe::{embed::Request, value::Value};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
};

/// Why a host call got no reply.
#[derive(Debug)]
pub enum Error {
    /// No host is connected.
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

/// The two ends of the pipe, when a host is connected.
pub struct Host(Option<(File, File)>);

impl Host {
    /// The pipe on fds 3 and 4, opened through `/dev/fd`; no host when
    /// fd 3 is not open.
    pub fn open() -> Self {
        let requests = OpenOptions::new().write(true).open("/dev/fd/3");
        let replies = File::open("/dev/fd/4");
        Self(requests.ok().zip(replies.ok()))
    }

    /// Whether a host is connected.
    pub const fn connected(&self) -> bool {
        self.0.is_some()
    }

    /// Ask the host a call, with the main lines kept so far; its reply.
    pub fn call(
        &mut self,
        request: &Request,
        lines: &[(usize, Value)],
    ) -> Result<Result<Value, String>, Error> {
        let frame = Frame::Call {
            request: request.clone(),
            lines: lines.to_vec(),
        };
        self.send(&frame)?;
        let (_, replies) = self.0.as_mut().ok_or(Error::Absent)?;
        let mut length = [0; 8];
        replies.read_exact(&mut length).map_err(Error::Io)?;
        let length = usize::try_from(u64::from_le_bytes(length))
            .map_err(|_| Error::Codec(codec::Error::Malformed))?;
        let mut payload = vec![0; length];
        replies.read_exact(&mut payload).map_err(Error::Io)?;
        codec::decode_reply(&payload).map_err(Error::Codec)
    }

    /// Write one frame.
    pub fn send(&mut self, frame: &Frame) -> Result<(), Error> {
        let (requests, _) = self.0.as_mut().ok_or(Error::Absent)?;
        let payload = frame.encode().map_err(Error::Codec)?;
        let mut out = (payload.len() as u64).to_le_bytes().to_vec();
        out.extend(payload);
        requests.write_all(&out).map_err(Error::Io)
    }
}
