use std::{env, path::PathBuf, process::Command};

fn main() {
    let host = env::var("HOST").expect("HOST");
    let target = env::var("TARGET").expect("TARGET");
    assert_eq!(host, target, "C provider proof refuses cross compilation");
    assert_eq!(
        target, "x86_64-unknown-linux-gnu",
        "unqualified C proof target"
    );
    let root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("fixture root"));
    let header = root.join("../../include");
    let source = root.join("provider.c");
    let object = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("provider.o");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", header.join("fipe.h").display());
    let output = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(header)
        .arg("-c")
        .arg(source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("C provider proof requires cc");
    assert!(
        output.status.success(),
        "cc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("cargo:rustc-link-arg-tests={}", object.display());
}
