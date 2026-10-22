//! The runtime archive, scratch files and golden listings of the
//! tests; each test binary uses a part.
#![allow(dead_code)]
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// A fresh scratch directory `name` under the tests' target directory.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `libfipe_native_rt.a`, built once per test binary in its own target
/// directory, so the outer cargo's lock is not waited on.
pub fn runtime() -> &'static Path {
    static ARCHIVE: OnceLock<PathBuf> = OnceLock::new();
    ARCHIVE.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("runtime");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--locked", "--offline", "--manifest-path"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("runtime/Cargo.toml"),
            )
            .arg("--target-dir")
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success(), "the runtime does not build");
        target.join("debug/libfipe_native_rt.a")
    })
}

/// Runs `program` and returns its exit code.
pub fn run(program: &Path) -> i32 {
    Command::new(program).status().unwrap().code().unwrap()
}

/// `listing` equals `tests/golden/<name>.lst`; `FIPE_BLESS=1` rewrites
/// the file instead.
pub fn golden(name: &str, listing: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.lst"));
    if std::env::var_os("FIPE_BLESS").is_some() {
        std::fs::write(&path, listing).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), listing);
}
