//! `fipe-native build <file.fipe> -o <exe>`: a Fipe program compiled to
//! an x86-64 Linux executable.
//!
//! `--host DIR` checks it against the host declarations in `DIR`;
//! `--object` stops at the ELF object; otherwise the object is linked
//! with `cc` against the runtime archive `--runtime LIB`, or the one
//! `FIPE_NATIVE_RT` names.

use fipe_native::{codec::Unit, elf, host_declarations, link, lower};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "usage: fipe-native build <file.fipe> -o <output> \
                     [--host DIR] [--runtime LIB] [--object]";

/// What `build` was asked.
struct Build {
    source: PathBuf,
    output: PathBuf,
    host: Option<PathBuf>,
    runtime: Option<PathBuf>,
    object: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Build, String> {
    if args.next().as_deref() != Some("build") {
        return Err(USAGE.into());
    }
    let (mut source, mut output, mut host, mut runtime) =
        (None, None, None, None);
    let mut object = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().map(PathBuf::from).ok_or(USAGE);
        match arg.as_str() {
            "-o" => output = Some(value()?),
            "--host" => host = Some(value()?),
            "--runtime" => runtime = Some(value()?),
            "--object" => object = true,
            _ if source.is_none() && !arg.starts_with('-') => {
                source = Some(PathBuf::from(arg));
            }
            _ => return Err(USAGE.into()),
        }
    }
    let runtime = runtime
        .or_else(|| std::env::var_os("FIPE_NATIVE_RT").map(PathBuf::from));
    match (source, output) {
        (Some(source), Some(output)) => Ok(Build {
            source,
            output,
            host,
            runtime,
            object,
        }),
        _ => Err(USAGE.into()),
    }
}

fn build(build: &Build) -> Result<(), String> {
    let read = |path: &Path| {
        std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    let main = read(&build.source)?;
    let host = match &build.host {
        Some(dir) => host_declarations(dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?,
        None => Vec::new(),
    };
    let (unit, program) = Unit::new(&main, host)?;
    let object =
        lower::object(&program, &unit.encode()).map_err(|e| e.to_string())?;
    let bytes = elf::write(&object).map_err(|error| error.to_string())?;
    let write = |path: &Path| {
        std::fs::write(path, &bytes)
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    if build.object {
        return write(&build.output);
    }
    let runtime = build.runtime.as_deref().ok_or_else(|| {
        String::from("no runtime: pass --runtime or set FIPE_NATIVE_RT")
    })?;
    let path = build.output.with_extension("o");
    write(&path)?;
    let linked = link::link(&path, runtime, &build.output);
    let _ = std::fs::remove_file(&path);
    linked.map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)).and_then(|asked| build(&asked)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fipe-native: {error}");
            ExitCode::FAILURE
        }
    }
}
