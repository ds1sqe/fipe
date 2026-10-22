//! The runtime archive and scratch files the tests link with.
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
