//! `fipe`: Fipe scripts on this machine, through the std host
//! (`tools/host/README.md`).
//!
//! - `fipe run FILE [--budget N] [--allow-write DIR]... [-- ARG...]`
//!   runs a script: the success output to stdout, the error output to
//!   stderr, stdin as host input when the last line takes it; exit 0 a
//!   value, 1 an `Err` or a Fault, 2 when it could not run.
//! - `fipe check FILE...` checks each file; exit 0 when all check, 1
//!   when one does not, 2 when one cannot be read.
//! - `fipe repl [--strict] [--budget N] [--compile-budget N]
//!   [--allow-write DIR]...` is the REPL over the std host.
//! - `fipe help` (`--help`, `-h`) prints this usage.
//! - `fipe build FILE -o EXE [--runtime LIB] [--llvm]` builds a native
//!   executable that answers through the std host, its object from the
//!   x86-64 backend or, with `--llvm`, from LLVM IR compiled by clang;
//!   the runtime archive is `--runtime` or `FIPE_NATIVE_RT`. Building is
//!   supported only on x86-64 Linux; exit 1 on an unsupported host or a
//!   build failure, 2 for invalid arguments or a script check failure on
//!   a supported host.
#![forbid(unsafe_code)]

mod format;

use fipe_host::{load, Host, LoadError, Policy};
use fipe_native::{codec::Unit, elf, link, lower};
use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "\
usage: fipe run FILE [--budget N] [--allow-write DIR]... [-- ARG...]
       fipe host-declarations DIR
       fipe check FILE...
       fipe fmt [--check] FILE...
       fipe repl [--strict] [--budget N] [--compile-budget N] \
[--allow-write DIR]...
       fipe build FILE -o EXE [--runtime LIB] [--llvm]
       fipe help
       fipe --version
";

/// The options a command line carries.
#[derive(Default)]
struct Options {
    files: Vec<String>,
    budget: Option<u64>,
    compile_budget: Option<u64>,
    allow: Vec<PathBuf>,
    strict: bool,
    output: Option<PathBuf>,
    runtime: Option<PathBuf>,
    llvm: bool,
    /// The script's arguments, after `--`.
    args: Vec<String>,
}

/// The options among `allowed`, files as the rest.
fn parse(args: &[String], allowed: &[&str]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if !allowed.contains(&arg.as_str()) {
            if arg.starts_with('-') && arg != "-" {
                return Err(format!("unknown option {arg}\n{USAGE}"));
            }
            options.files.push(arg.clone());
            continue;
        }
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} takes a value\n{USAGE}"))
        };
        let number = |value: &str| {
            value
                .parse()
                .map_err(|_| format!("{arg} takes a number\n{USAGE}"))
        };
        match arg.as_str() {
            "--budget" => options.budget = Some(number(value()?)?),
            "--compile-budget" => {
                options.compile_budget = Some(number(value()?)?);
            }
            "--allow-write" => options.allow.push(PathBuf::from(value()?)),
            "--strict" => options.strict = true,
            "--llvm" => options.llvm = true,
            "-o" => options.output = Some(PathBuf::from(value()?)),
            "--runtime" => options.runtime = Some(PathBuf::from(value()?)),
            "--" => {
                options.args = args.cloned().collect();
                break;
            }
            _ => unreachable!("an allowed option"),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    if raw
        .first()
        .is_some_and(|command| command == "host-declarations")
    {
        return match host_declarations(&raw[1..]) {
            Ok(code) => ExitCode::from(code),
            Err((code, message)) => {
                eprintln!("{message}");
                ExitCode::from(code)
            }
        };
    }
    if raw.first().is_some_and(|command| command == "fmt") {
        return ExitCode::from(format::run(&raw[1..]));
    }
    let args: Vec<String> = raw
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let ran = match args.first().map(String::as_str) {
        Some("--version") if args.len() == 1 => {
            println!("fipe {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("run") => run(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("repl") => repl(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print!("{USAGE}");
            Ok(0)
        }
        _ => Err((2, String::from(USAGE))),
    };
    match ran {
        Ok(code) => ExitCode::from(code),
        Err((code, message)) => {
            eprint!("{message}");
            if !message.ends_with('\n') {
                eprintln!();
            }
            ExitCode::from(code)
        }
    }
}

/// A command's exit code, or the code and message of its refusal.
type Ran = Result<u8, (u8, String)>;

const fn usage(error: String) -> (u8, String) {
    (2, error)
}

/// Export fixed Standard metadata, without running a program or merging hosts.
fn host_declarations(arguments: &[std::ffi::OsString]) -> Ran {
    let [directory] = arguments else {
        return Err(usage("usage: fipe host-declarations DIR".into()));
    };
    if directory.as_encoded_bytes().starts_with(b"-") {
        return Err(usage("usage: fipe host-declarations DIR".into()));
    }
    let directory = Path::new(directory);
    fs::create_dir(directory).map_err(|error| {
        (
            2,
            format!("fipe host-declarations: {}: {error}", directory.display()),
        )
    })?;
    for (namespace, source) in fipe_host::sources() {
        let path = directory.join(format!("{namespace}.fipe"));
        let write = || -> io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(source.as_bytes())
        };
        write().map_err(|error| {
            (2, format!("fipe host-declarations: {}: {error}; incomplete destination {} left in place",
                path.display(), directory.display()))
        })?;
    }
    Ok(0)
}

/// The one file a command takes.
fn one_file(options: &Options) -> Result<&str, (u8, String)> {
    match options.files.as_slice() {
        [file] => Ok(file),
        _ => Err(usage(String::from(USAGE))),
    }
}

fn policy(allow: &[PathBuf]) -> Result<Policy, (u8, String)> {
    Policy::new(Path::new("."), allow)
        .map_err(|error| (2, format!("fipe: the write roots: {error}")))
}

fn run(args: &[String]) -> Ran {
    let options =
        parse(args, &["--budget", "--allow-write", "--"]).map_err(usage)?;
    if let [_, extra, ..] = options.files.as_slice() {
        return Err(usage(format!(
            "fipe: unexpected operand {extra}; the script's arguments \
             follow --\n"
        )));
    }
    let file = one_file(&options)?;
    let script =
        load(Path::new(file)).map_err(|error| (2, error.to_string()))?;
    let mut host = Host::new(policy(&options.allow)?)
        .with_args(options.args)
        .streaming(io::stdout(), io::stderr());
    let (_, exit) = host.run(&script, &mut io::stdin().lock(), options.budget);
    Ok(exit)
}

fn check(args: &[String]) -> Ran {
    let options = parse(args, &[]).map_err(usage)?;
    if options.files.is_empty() {
        return Err(usage(String::from(USAGE)));
    }
    let mut code = 0;
    for file in &options.files {
        match load(Path::new(file)) {
            Ok(_) => {}
            Err(error @ LoadError::Check(_)) => {
                eprintln!("{error}");
                code = code.max(1);
            }
            Err(error) => {
                eprintln!("{error}");
                code = 2;
            }
        }
    }
    Ok(code)
}

fn repl_answer(
    host: &mut Host,
    request: &fipe::embed::Request,
    tables: &fipe::ir::Tables,
) -> Result<fipe::embed::Reply, String> {
    host.flush()
        .map_err(|error| format!("cannot write output: {error}"))?;
    let reply = host.answer(request, tables);
    repl_reply(reply, host.flush())
}
fn repl_reply(
    reply: fipe::embed::Reply,
    flushed: io::Result<()>,
) -> Result<fipe::embed::Reply, String> {
    match (reply, flushed) {
        (fipe::embed::Reply::Value(Err(primary)), Err(_)) => Err(primary),
        (_, Err(error)) => Err(format!("cannot write output: {error}")),
        (reply, Ok(())) => Ok(reply),
    }
}

fn repl(args: &[String]) -> Ran {
    let options = parse(
        args,
        &["--strict", "--budget", "--compile-budget", "--allow-write"],
    )
    .map_err(usage)?;
    if !options.files.is_empty() {
        return Err(usage(String::from(USAGE)));
    }
    let mut host = Host::new(policy(&options.allow)?)
        .streaming(io::stdout(), io::stderr());
    let mut answer =
        |request: &_, tables: &_| repl_answer(&mut host, request, tables);
    let defaults = fipe_repl::Options::default();
    fipe_repl::run(
        &fipe_repl::Options {
            host: None,
            strict: options.strict,
            budget: options.budget.unwrap_or(defaults.budget),
            compile_budget: options
                .compile_budget
                .unwrap_or(defaults.compile_budget),
        },
        Some(fipe_repl::Host {
            name: "tools/host/decl",
            sources: fipe_host::sources(),
            answer: &mut answer,
        }),
    );
    Ok(0)
}

fn build(args: &[String]) -> Ran {
    let options = parse(args, &["-o", "--runtime", "--llvm"]).map_err(usage)?;
    let file = one_file(&options)?;
    let output = options
        .output
        .as_deref()
        .ok_or_else(|| usage(String::from(USAGE)))?;
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err((
            1,
            "fipe: native builds are supported only on x86-64 Linux".into(),
        ));
    }
    let script =
        load(Path::new(file)).map_err(|error| (2, error.to_string()))?;
    let (main, files) = script
        .sources
        .split_first()
        .expect("a loaded script has its main source");
    let host = fipe_host::sources()
        .iter()
        .map(|(name, text)| (String::from(*name), String::from(*text)))
        .collect();
    let (unit, program) = Unit::new(&main.1, host, files.to_vec())
        .map_err(|error| (2, format!("fipe: {error}")))?;
    let failed = |error: String| (1, format!("fipe: {error}"));
    let runtime = options
        .runtime
        .or_else(|| std::env::var_os("FIPE_NATIVE_RT").map(PathBuf::from))
        .ok_or_else(|| {
            failed(String::from(
                "no runtime: pass --runtime or set FIPE_NATIVE_RT",
            ))
        })?;
    let path = output.with_extension("o");
    if options.llvm {
        let ir = fipe_llvm::module(&program, &unit.encode()).map_err(failed)?;
        fipe_llvm::compile(&ir, &path)
            .map_err(|error| failed(error.to_string()))?;
    } else {
        let object = lower::object(&program, &unit.encode()).map_err(failed)?;
        let bytes =
            elf::write(&object).map_err(|error| failed(error.to_string()))?;
        std::fs::write(&path, &bytes)
            .map_err(|error| failed(format!("{}: {error}", path.display())))?;
    }
    let linked = link::link(&path, &runtime, output);
    let _ = std::fs::remove_file(&path);
    linked.map_err(|error| failed(error.to_string()))?;
    Ok(0)
}

#[cfg(test)]
mod repl_output_tests {
    use super::*;
    use fipe::embed::{Outcome, Reply, Request};
    use std::io::Write;
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn checked(source: &str) -> fipe::mir::Program {
        let ast = fipe::parse::parse(fipe::lex::lex(source)).unwrap();
        fipe::compile::compile(
            &fipe::check::check(
                &ast,
                &fipe_host::declarations(),
                &[],
                fipe::check::Options::default(),
            )
            .unwrap(),
        )
    }
    fn request(
        program: &fipe::mir::Program,
        directory: &std::path::Path,
    ) -> Request {
        let mut machine = fipe::vm::Machine::new(None);
        let mut host = Host::new(Policy::new(directory, &[]).unwrap());
        let mut outcome = machine.run(program);
        loop {
            let Outcome::Request(request) = outcome else {
                panic!("real put request")
            };
            if matches!(&request,Request::Call{declaration,..} if program.tables.borrow().declarations[*declaration].label=="put")
            {
                return request;
            }
            outcome = machine.reply(
                program,
                host.answer(&request, &program.tables.borrow()),
            );
        }
    }
    #[test]
    fn output_repl_adapter_preflush_refuses_effect_and_postflush_is_fatal() {
        let directory = std::env::temp_dir()
            .join(format!("fipe-repl-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let program = checked("put ./sentinel \"no\"");
        let mut host = Host::new(Policy::new(&directory, &[]).unwrap())
            .streaming(Broken, io::sink());
        host.sink.success.extend_from_slice(b"before");
        assert!(repl_answer(
            &mut host,
            &request(&program, &directory),
            &program.tables.borrow()
        )
        .unwrap_err()
        .contains("cannot write output"));
        assert!(!directory.join("sentinel").exists());
        let mut healthy = Host::new(Policy::new(&directory, &[]).unwrap())
            .streaming(io::sink(), io::sink());
        assert!(repl_answer(
            &mut healthy,
            &request(&program, &directory),
            &program.tables.borrow()
        )
        .is_ok());
        assert_eq!(std::fs::read(directory.join("sentinel")).unwrap(), b"no");
        std::fs::remove_file(directory.join("sentinel")).unwrap();
        let program = checked("put out \"payload\"");
        let mut host = Host::new(Policy::new(&directory, &[]).unwrap())
            .streaming(Broken, io::sink());
        assert!(repl_answer(
            &mut host,
            &request(&program, &directory),
            &program.tables.borrow()
        )
        .unwrap_err()
        .contains("cannot write output"));
        let mut host = Host::new(Policy::new(&directory, &[]).unwrap())
            .streaming(io::sink(), io::sink());
        assert!(repl_answer(
            &mut host,
            &request(&program, &directory),
            &program.tables.borrow()
        )
        .is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn output_repl_adapter_keeps_primary_fault_and_recoverable_reply() {
        let reply = || Reply::Value(Err(String::from("primary")));
        assert_eq!(
            repl_reply(reply(), Err(io::ErrorKind::BrokenPipe.into())),
            Err(String::from("primary"))
        );
        assert_eq!(repl_reply(reply(), Ok(())), Ok(reply()));
    }
}
