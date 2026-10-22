//! Linking an object against the runtime with `cc`, which finds the C
//! start files and libc that bare `ld` would need named.
use std::{fmt, io, path::Path, process::Command};

/// The system libraries the runtime's standard library needs: rustc's
/// `--print native-static-libs` for this staticlib on x86-64 Linux,
/// checked against it by `scripts/probes.sh`.
pub const NATIVE_LIBS: &[&str] = &[
    "-lgcc_s",
    "-lutil",
    "-lrt",
    "-lpthread",
    "-lm",
    "-ldl",
    "-lc",
];

/// Why linking failed.
#[derive(Debug)]
pub enum Error {
    /// `cc` could not be started.
    Spawn(io::Error),
    /// `cc` ran and failed; its standard error.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "cannot run cc: {error}"),
            Self::Failed(stderr) => write!(f, "cc failed:\n{stderr}"),
        }
    }
}

impl std::error::Error for Error {}

/// Links `object` with the runtime archive `runtime` into the
/// executable `output`.
///
/// # Errors
///
/// When `cc` cannot be started or fails.
pub fn link(object: &Path, runtime: &Path, output: &Path) -> Result<(), Error> {
    let result = Command::new("cc")
        .arg(object)
        .arg(runtime)
        .args(NATIVE_LIBS)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(Error::Spawn)?;
    if result.status.success() {
        Ok(())
    } else {
        Err(Error::Failed(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ))
    }
}
