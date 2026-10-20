//! The adapter over stdio: a reader thread hands each request to the
//! session on the main thread, which writes the responses and events.
//! A running program advances in slices, so a `pause` that arrives
//! meanwhile is seen between them.
use fipe_dap::{
    protocol,
    session::{Control, Session},
};
use std::{
    io::{self, BufReader},
    sync::mpsc::{self, TryRecvError},
    thread,
};

/// Stops a running program takes before the loop looks for requests.
const SLICE: usize = 10_000;

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
    loop {
        let request = if session.running() {
            match received.try_recv() {
                Ok(request) => Some(request),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match received.recv() {
                Ok(request) => Some(request),
                Err(_) => return,
            }
        };
        let control = request
            .as_ref()
            .map_or(Control::Continue, |request| session.handle(request));
        if session.running() {
            session.advance(SLICE);
        }
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
