use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let host = env::var("HOST").expect("HOST");
    let target = env::var("TARGET").expect("TARGET");
    assert_eq!(host, target, "linked example refuses cross compilation");
    assert_eq!(
        target, "x86_64-unknown-linux-gnu",
        "unqualified example target"
    );
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("example root"));
    let header =
        PathBuf::from(env::var_os("FIPE_FFI_INCLUDE").expect(
            "set FIPE_FFI_INCLUDE to the absolute directory containing the matching fipe.h",
        ));
    assert!(header.is_absolute(), "FIPE_FFI_INCLUDE must be absolute");
    let header_file = header.join("fipe.h");
    assert!(header_file.is_file(), "FIPE_FFI_INCLUDE has no fipe.h");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let api = root.join("api.fipi");
    let bytes = fs::read(&api).expect("read api.fipi");
    std::str::from_utf8(&bytes).expect("api.fipi must be UTF-8");
    assert!(!bytes.is_empty(), "api.fipi must not be empty");
    let elements: Vec<_> = bytes.iter().map(u8::to_string).collect();
    fs::write(
        output.join("interface.h"),
        format!(
            "static const uint8_t declarations[] = {{{}}};\n",
            elements.join(",")
        ),
    )
    .expect("write C interface bytes");
    let source = root.join("provider.c");
    println!("cargo:rerun-if-env-changed=FIPE_FFI_INCLUDE");
    for path in [&source, &api, &header_file] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let object = output.join("provider.o");
    let compiled = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(&header)
        .arg("-I")
        .arg(&output)
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("linked example requires an installed C11 cc compiler");
    assert!(
        compiled.status.success(),
        "cc failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    // Both the runnable example and its unit-test binary need the C descriptor.
    println!("cargo:rustc-link-arg={}", object.display());
}
