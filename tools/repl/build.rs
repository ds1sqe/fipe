fn main() {
    // The `linux` platform supplies `_start` itself.
    if std::env::var_os("CARGO_FEATURE_LINUX").is_some() {
        println!("cargo:rustc-link-arg-bins=-nostartfiles");
    }
}
