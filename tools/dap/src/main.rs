//! The adapter over stdio: a reader thread hands each request to the
//! session on the main thread, which writes the responses and events.
use fipe_dap::{
    protocol,
    session::{Control, Session},
};
use std::{
    io::{self, BufReader},
    sync::mpsc,
    thread,
};

fn main() {
    let (requests, received) = mpsc::channel();
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        while let Ok(Some(message)) = protocol::read(&mut input) {
            if requests.send(message).is_err() {
                break;
            }
        }
    });
    let mut session = Session::default();
    let mut output = io::stdout().lock();
    for request in received {
        let control = session.handle(&request);
        for message in session.out.take() {
            if protocol::write(&mut output, &message).is_err() {
                return;
            }
        }
        if control == Control::Exit {
            return;
        }
    }
}
