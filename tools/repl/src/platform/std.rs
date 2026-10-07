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
    std::process::exit(code)
}
