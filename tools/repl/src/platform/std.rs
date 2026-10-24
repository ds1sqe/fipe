//! Any OS `std` supports.

use std::io::{Read, Write};

/// Bytes read from stdin into `buffer`; 0 at end of input.
pub fn read(buffer: &mut [u8]) -> Result<usize, ()> {
    loop {
        match std::io::stdin().read(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            result => return result.map_err(|_| ()),
        }
    }
}

pub fn write(bytes: &[u8]) -> Result<(), ()> {
    write_to(&mut std::io::stdout().lock(), bytes).map_err(|_| ())
}

fn write_to(writer: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()
}

#[cfg(test)]
pub(super) fn saved() -> bool {
    SAVED.lock().expect("saved terminal").is_some()
}

pub fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The arguments after the program's name.
pub fn args() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Whether `NO_COLOR` is set to a non-empty value.
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// Whether the working directory is now `path`.
pub fn chdir(path: &str) -> bool {
    std::env::set_current_dir(path).is_ok()
}

pub fn exit(code: i32) -> ! {
    cooked();
    std::process::exit(code)
}

/// The terminal settings before [`raw`], restored by [`cooked`].
static SAVED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Whether stdin and stdout are both a terminal.
pub fn terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Whether stdin is now in raw mode: no echo, no line buffering, Ctrl-C
/// as a byte; output still maps LF to CR LF. `std` has no termios, so
/// this runs `stty` on the inherited stdin; without it the REPL keeps
/// cooked line input.
pub fn raw() -> bool {
    let stty = |args: &[&str]| {
        std::process::Command::new("stty")
            .args(args)
            .stdin(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
    };
    let Some(saved) = stty(&["-g"]) else {
        return false;
    };
    let saved = String::from_utf8_lossy(&saved.stdout).trim().to_owned();
    let raw = ["-icanon", "-echo", "-isig", "-iexten", "-ixon", "-icrnl"];
    if stty(&raw).is_none() || stty(&["min", "1", "time", "0"]).is_none() {
        let _ = stty(&[&saved]);
        return false;
    }
    *SAVED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(saved);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cooked();
        hook(info);
    }));
    true
}

/// The terminal as [`raw`] found it; nothing when it was not switched.
pub fn cooked() {
    let saved = SAVED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(saved) = saved {
        let _ = std::process::Command::new("stty").arg(saved).status();
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;
    struct Writer {
        bytes: Vec<u8>,
        left: usize,
        flush_error: bool,
        calls: usize,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            if self.left == 0 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            let n = bytes.len().min(self.left);
            self.bytes.extend_from_slice(&bytes[..n]);
            self.left -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.flush_error {
                Err(std::io::ErrorKind::Other.into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn output_writer_errors_preserve_exact_prefix() {
        for left in [0, 2, 4] {
            let mut writer = Writer {
                bytes: Vec::new(),
                left,
                flush_error: true,
                calls: 0,
            };
            assert!(write_to(&mut writer, b"abcd").is_err());
            assert_eq!(writer.bytes, &b"abcd"[..left]);
            assert_eq!(writer.calls, if left == 2 { 2 } else { 1 });
        }
        let mut bytes = Vec::new();
        write_to(&mut bytes, b"abcd").unwrap();
        assert_eq!(bytes, b"abcd");
        assert_eq!(
            write_to(&mut &mut [0u8; 0][..], b"x").unwrap_err().kind(),
            std::io::ErrorKind::WriteZero
        );
    }
}
