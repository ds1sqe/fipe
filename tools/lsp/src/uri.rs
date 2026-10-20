//! `file:` URIs and paths.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// The path a `file:` URI names.
#[must_use]
pub fn path(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let high = iter.next()?;
            let low = iter.next()?;
            let hex = [high, low];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

/// The `file:` URI of an absolute path.
///
/// # Panics
/// If the encoded path is not a URI, which percent-encoding rules out.
#[must_use]
pub fn of(path: &Path) -> Uri {
    let mut encoded = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    Uri::from_str(&encoded).expect("a percent-encoded file URI")
}
