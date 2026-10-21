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

pub fn write(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    // A closed stdout leaves nothing to report to.
    let _ = stdout.write_all(bytes).and_then(|()| stdout.flush());
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
