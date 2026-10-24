//! `fipe-lsp [--host <dir>]`: the language server over stdio.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE:&str="usage: fipe-lsp [--host <dir>]\n       fipe-lsp --version\n       fipe-lsp --help\n";

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() == 1 {
        match arguments[0].as_str() {
            "--version" => {
                println!("fipe-lsp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }
    let mut options = fipe_lsp::Options::default();
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--host" {
            let Some(path) = args.next() else {
                eprint!("{USAGE}");
                return ExitCode::FAILURE;
            };
            options.host = Some(PathBuf::from(path));
        } else {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout().lock();
    match fipe_lsp::serve(&mut input, &mut output, &options) {
        Ok(true) => ExitCode::SUCCESS,
        // `exit`, or the end of the input, without `shutdown` first.
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("fipe-lsp: {error}");
            ExitCode::FAILURE
        }
    }
}
