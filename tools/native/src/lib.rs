//! Fipe's native backend: x86-64 machine code for Linux and the System
//! V ABI, written into an ELF64 relocatable object and linked with `cc`
//! against the runtime `libfipe_native_rt.a`.
#![forbid(unsafe_code)]

pub use fipe_native_core::codec;
pub mod elf;
pub mod link;
pub mod lower;
pub mod x64;

use std::{io, path::Path};

/// A host's declaration files in `dir`, as a unit carries them:
/// `commands.fipe` first, then every other `*.fipe` by name, each named
/// by its stem.
///
/// # Errors
///
/// When the directory or a file cannot be read.
pub fn host_declarations(dir: &Path) -> io::Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "fipe")
        {
            let stem = path.file_stem().unwrap_or_default();
            let stem = stem.to_string_lossy().into_owned();
            files.push((stem, std::fs::read_to_string(&path)?));
        }
    }
    files.sort_by(|(a, _), (b, _)| {
        (a != "commands", a.as_str()).cmp(&(b != "commands", b.as_str()))
    });
    Ok(files)
}
