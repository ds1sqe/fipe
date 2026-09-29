//! Any OS `std` supports.

use std::io::{Read, Write};

/// Bytes read from stdin into `buffer`; 0 at end of input.
pub fn read(buffer: &mut [u8]) -> usize {
    loop {
        match std::io::stdin().read(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            result => return result.unwrap_or(0),
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
