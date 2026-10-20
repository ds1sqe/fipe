//! `fipe-lsp [--host <dir>]`: the language server over stdio.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut options = fipe_lsp::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--host" {
            options.host = args.next().map(PathBuf::from);
        } else {
            eprintln!("usage: fipe-lsp [--host <dir>]");
            return ExitCode::FAILURE;
        }
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout().lock();
    match fipe_lsp::serve(&mut input, &mut output, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fipe-lsp: {error}");
            ExitCode::FAILURE
        }
    }
}
