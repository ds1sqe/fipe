//! The adapter over stdio: a reader thread hands each request to the
//! session on the main thread, which writes the responses and events.
//! A running program advances in slices, so a `pause` that arrives
//! meanwhile is seen between them.
use fipe_dap::{
    protocol::{self, Incoming},
    session::{Control, Session},
};
use std::{
    io::{self, BufReader},
    process::ExitCode,
    sync::mpsc::{self, TryRecvError},
    thread,
};

/// Stops a running program takes before the loop looks for requests.
const SLICE: usize = 10_000;

const USAGE:&str="usage: fipe-dap\n       fipe-dap --version\n       fipe-dap --help\nDebug Adapter Protocol over stdio; configure launches through protocol requests.\n";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        println!("fipe-dap {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if !args.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    }
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fipe-dap: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve() -> io::Result<()> {
    let (requests, received) = mpsc::channel();
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        // A malformed frame is handed on too, so the session answers it
        // and the input goes on; only the end of the input or a failed
        // read ends the reader.
        loop {
            let incoming = match protocol::read(&mut input) {
                Ok(Some(incoming)) => Ok(incoming),
                Ok(None) => break,
                Err(error) => {
                    let _ = requests.send(Err(error));
                    break;
                }
            };
            if requests.send(incoming).is_err() {
                break;
            }
        }
    });
    let mut session = Session::default();
    let mut output = io::stdout().lock();
    loop {
        let request = if session.running() {
            match received.try_recv() {
                Ok(request) => Some(request?),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        } else {
            match received.recv() {
                Ok(request) => Some(request?),
                Err(_) => return Ok(()),
            }
        };
        let control = match &request {
            Some(Incoming::Message(request)) => session.handle(request),
            Some(Incoming::Malformed(why)) => {
                session.malformed(why);
                Control::Continue
            }
            None => Control::Continue,
        };
        if session.running() {
            session.advance(SLICE);
        }
        for message in session.out.take() {
            protocol::write(&mut output, &message)?;
        }
        if control == Control::Exit {
            return Ok(());
        }
    }
}
