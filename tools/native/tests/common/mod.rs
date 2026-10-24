//! The runtime archive, scratch files and golden listings of the
//! tests; each test binary uses a part.
#![allow(dead_code)]
use fipe::mir::Program;
use fipe_native::{codec::Unit, elf, link, lower};
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
/// directory, so the outer cargo's lock is not waited on, in the test's
/// own profile.
pub fn runtime() -> &'static Path {
    static ARCHIVE: OnceLock<PathBuf> = OnceLock::new();
    ARCHIVE.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("runtime");
        let release = !cfg!(debug_assertions);
        let status = Command::new(env!("CARGO"))
            .args(["build", "--locked", "--offline", "--manifest-path"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("runtime/Cargo.toml"),
            )
            .arg("--target-dir")
            .arg(&target)
            .args(release.then_some("--release"))
            .status()
            .unwrap();
        assert!(status.success(), "the runtime does not build");
        let profile = if release { "release" } else { "debug" };
        target.join(profile).join("libfipe_native_rt.a")
    })
}

/// The executable of a unit, linked against the runtime: its object
/// from this backend, or from the LLVM backend (`tools/llvm`) when
/// `FIPE_BACKEND=llvm`, which runs every equivalence test on it.
pub fn executable(name: &str, unit: &Unit, program: &Program) -> PathBuf {
    let dir = scratch(name);
    let path = dir.join(format!("{name}.o"));
    if std::env::var("FIPE_BACKEND").as_deref() == Ok("llvm") {
        let ir = fipe_llvm::module(program, &unit.encode()).unwrap();
        fipe_llvm::compile(&ir, &path).unwrap();
    } else {
        let object = lower::object(program, &unit.encode()).unwrap();
        std::fs::write(&path, elf::write(&object).unwrap()).unwrap();
    }
    let executable = dir.join(name);
    link::link(&path, runtime(), &executable).unwrap();
    executable
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
