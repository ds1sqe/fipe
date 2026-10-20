fn main() {
    let src = std::path::Path::new("src");
    let mut build = cc::Build::new();
    build.std("c11").include(src);
    #[cfg(target_env = "msvc")]
    build.flag("-utf-8");
    for file in ["parser.c", "scanner.c"] {
        let path = src.join(file);
        build.file(&path);
        println!("cargo:rerun-if-changed={}", path.display());
    }
    build.compile("tree-sitter-fipe");
}
